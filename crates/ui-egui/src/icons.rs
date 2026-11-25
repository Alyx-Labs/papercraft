//! Semantic icon registry backed by the vendored Lucide outline SVG assets.

use egui::{
    Color32, CornerRadius, FontId, Pos2, Rect, Response, Sense, Stroke, StrokeKind, Vec2, pos2,
};

use crate::theme::Tokens;

/// Semantic icon names used throughout the interface.
///
/// Each variant is mapped to a local Lucide outline SVG in [`icon_svg`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    FountainPen,
    BallPen,
    BrushPen,
    Highlighter,
    HighlighterFreehand,
    HighlighterStraight,
    Eraser,
    StrokeEraser,
    PrecisionEraser,
    Undo,
    Redo,
    ClearPage,
    FitPage,
    ZoomIn,
    ZoomOut,
    Settings,
    Preset,
    Stabilizer,
    Pressure,
    Paper,
    Reset,
    Info,
    Notebook,
    Library,
    Search,
    Plus,
    Export,
    AddPage,
    Duplicate,
    PrevPage,
    NextPage,
    Close,
    Palette,
    Edit,
    Trash,
    Check,
    Sun,
    Moon,
    Account,
    Sync,
    CheckCircle,
    AlertCircle,
    Server,
    Eye,
    EyeOff,
    ShapeTool,
    LineShape,
    RectangleShape,
    EllipseShape,
    TriangleShape,
    ArrowShape,
    Ruler,
    Select,
    Image,
    Link,
    Folder,
    FolderOpen,
    FolderPlus,
    FolderPen,
    MoveToFolder,
    Inbox,
    ChevronUp,
    ChevronDown,
    More,
    Star,
    Tag,
    ListView,
    GridView,
    Sort,
    MoveUp,
    MoveDown,
    Bookmark,
    RotateCw,
    CheckSquare,
    Transfer,
    Pencil,
    Lasso,
    PolygonShape,
    FillBucket,
    Paste,
    Bold,
    Italic,
    Underline,
    AlignLeft,
    AlignCenter,
    AlignRight,
    CheckList,
    Sticker,
    LaserPointer,
    GripHorizontal,
    Text,
    GitHub,
    Home,
    Help,
    Keyboard,
    Lock,
    LockOpen,
}

/// Load the local outline SVG assigned to a semantic icon.
fn icon_svg(icon: Icon) -> &'static str {
    match icon {
        Icon::FountainPen => include_str!("../assets/icons/lucide/pen-tool.svg"),
        Icon::BallPen => include_str!("../assets/icons/lucide/pen-line.svg"),
        Icon::BrushPen => include_str!("../assets/icons/lucide/brush.svg"),
        Icon::Highlighter => include_str!("../assets/icons/lucide/highlighter.svg"),
        Icon::HighlighterFreehand => include_str!("../assets/icons/lucide/signature.svg"),
        Icon::HighlighterStraight => include_str!("../assets/icons/lucide/ruler.svg"),
        Icon::Eraser => include_str!("../assets/icons/lucide/eraser.svg"),
        Icon::StrokeEraser => include_str!("../assets/icons/lucide/scissors.svg"),
        Icon::PrecisionEraser => include_str!("../assets/icons/lucide/crosshair.svg"),
        Icon::Undo => include_str!("../assets/icons/lucide/undo-2.svg"),
        Icon::Redo => include_str!("../assets/icons/lucide/redo-2.svg"),
        Icon::ClearPage => include_str!("../assets/icons/lucide/file-x.svg"),
        Icon::FitPage => include_str!("../assets/icons/lucide/maximize.svg"),
        Icon::ZoomIn => include_str!("../assets/icons/lucide/zoom-in.svg"),
        Icon::ZoomOut => include_str!("../assets/icons/lucide/zoom-out.svg"),
        Icon::Settings => include_str!("../assets/icons/lucide/settings-2.svg"),
        Icon::Preset => include_str!("../assets/icons/lucide/sparkles.svg"),
        Icon::Stabilizer => include_str!("../assets/icons/lucide/spline.svg"),
        Icon::Pressure => include_str!("../assets/icons/lucide/activity.svg"),
        Icon::Paper => include_str!("../assets/icons/lucide/file.svg"),
        Icon::Reset => include_str!("../assets/icons/lucide/rotate-ccw.svg"),
        Icon::Info => include_str!("../assets/icons/lucide/info.svg"),
        Icon::Notebook => include_str!("../assets/icons/lucide/book-open.svg"),
        Icon::Library => include_str!("../assets/icons/lucide/library.svg"),
        Icon::Search => include_str!("../assets/icons/lucide/search.svg"),
        Icon::Plus => include_str!("../assets/icons/lucide/plus.svg"),
        Icon::Export => include_str!("../assets/icons/lucide/upload.svg"),
        Icon::AddPage => include_str!("../assets/icons/lucide/file-plus.svg"),
        Icon::Duplicate => include_str!("../assets/icons/lucide/copy.svg"),
        Icon::PrevPage => include_str!("../assets/icons/lucide/chevron-left.svg"),
        Icon::NextPage => include_str!("../assets/icons/lucide/chevron-right.svg"),
        Icon::Close => include_str!("../assets/icons/lucide/x.svg"),
        Icon::Palette => include_str!("../assets/icons/lucide/palette.svg"),
        Icon::Edit => include_str!("../assets/icons/lucide/pencil.svg"),
        Icon::Trash => include_str!("../assets/icons/lucide/trash.svg"),
        Icon::Check => include_str!("../assets/icons/lucide/check.svg"),
        Icon::Sun => include_str!("../assets/icons/lucide/sun.svg"),
        Icon::Moon => include_str!("../assets/icons/lucide/moon.svg"),
        Icon::Account => include_str!("../assets/icons/lucide/user-round.svg"),
        Icon::Sync => include_str!("../assets/icons/lucide/cloud-sync.svg"),
        Icon::CheckCircle => include_str!("../assets/icons/lucide/circle-check.svg"),
        Icon::AlertCircle => include_str!("../assets/icons/lucide/circle-exclamation-point.svg"),
        Icon::Server => include_str!("../assets/icons/lucide/server.svg"),
        Icon::Eye => include_str!("../assets/icons/lucide/eye.svg"),
        Icon::EyeOff => include_str!("../assets/icons/lucide/eye-off.svg"),
        Icon::ShapeTool => include_str!("../assets/icons/lucide/shapes.svg"),
        Icon::LineShape => include_str!("../assets/icons/lucide/minus.svg"),
        Icon::RectangleShape => include_str!("../assets/icons/lucide/square.svg"),
        Icon::EllipseShape => include_str!("../assets/icons/lucide/circle.svg"),
        Icon::TriangleShape => include_str!("../assets/icons/lucide/triangle.svg"),
        Icon::ArrowShape => include_str!("../assets/icons/lucide/arrow-up-right.svg"),
        Icon::Ruler => include_str!("../assets/icons/lucide/ruler.svg"),
        Icon::Select => include_str!("../assets/icons/lucide/mouse-pointer-2.svg"),
        Icon::Image => include_str!("../assets/icons/lucide/image.svg"),
        Icon::Link => include_str!("../assets/icons/lucide/link.svg"),
        Icon::Folder => include_str!("../assets/icons/lucide/folder.svg"),
        Icon::FolderOpen => include_str!("../assets/icons/lucide/folder-open.svg"),
        Icon::FolderPlus => include_str!("../assets/icons/lucide/folder-plus.svg"),
        Icon::FolderPen => include_str!("../assets/icons/lucide/folder-pen.svg"),
        Icon::MoveToFolder => include_str!("../assets/icons/lucide/folder-input.svg"),
        Icon::Inbox => include_str!("../assets/icons/lucide/inbox.svg"),
        Icon::ChevronUp => include_str!("../assets/icons/lucide/chevron-up.svg"),
        Icon::ChevronDown => include_str!("../assets/icons/lucide/chevron-down.svg"),
        Icon::More => include_str!("../assets/icons/lucide/ellipsis.svg"),
        Icon::Star => include_str!("../assets/icons/lucide/star.svg"),
        Icon::Tag => include_str!("../assets/icons/lucide/tag.svg"),
        Icon::ListView => include_str!("../assets/icons/lucide/list.svg"),
        Icon::GridView => include_str!("../assets/icons/lucide/layout-grid.svg"),
        Icon::Sort => include_str!("../assets/icons/lucide/arrow-up-down.svg"),
        Icon::MoveUp => include_str!("../assets/icons/lucide/arrow-up.svg"),
        Icon::MoveDown => include_str!("../assets/icons/lucide/arrow-down.svg"),
        Icon::Bookmark => include_str!("../assets/icons/lucide/bookmark.svg"),
        Icon::RotateCw => include_str!("../assets/icons/lucide/rotate-cw.svg"),
        Icon::CheckSquare => include_str!("../assets/icons/lucide/square-check.svg"),
        Icon::Transfer => include_str!("../assets/icons/lucide/arrow-right-left.svg"),
        Icon::Pencil => include_str!("../assets/icons/lucide/pencil-line.svg"),
        Icon::Lasso => include_str!("../assets/icons/lucide/lasso.svg"),
        Icon::PolygonShape => include_str!("../assets/icons/lucide/pentagon.svg"),
        Icon::FillBucket => include_str!("../assets/icons/lucide/paint-bucket.svg"),
        Icon::Paste => include_str!("../assets/icons/lucide/clipboard-paste.svg"),
        Icon::Bold => include_str!("../assets/icons/lucide/letter-b.svg"),
        Icon::Italic => include_str!("../assets/icons/lucide/italic.svg"),
        Icon::Underline => include_str!("../assets/icons/lucide/underline.svg"),
        Icon::AlignLeft => include_str!("../assets/icons/lucide/text-align-start.svg"),
        Icon::AlignCenter => include_str!("../assets/icons/lucide/text-align-center.svg"),
        Icon::AlignRight => include_str!("../assets/icons/lucide/text-align-end.svg"),
        Icon::CheckList => include_str!("../assets/icons/lucide/list-todo.svg"),
        Icon::Sticker => include_str!("../assets/icons/lucide/sticker.svg"),
        Icon::LaserPointer => include_str!("../assets/icons/lucide/laser.svg"),
        Icon::GripHorizontal => include_str!("../assets/icons/lucide/grip-horizontal.svg"),
        Icon::Text => include_str!("../assets/icons/lucide/type.svg"),
        Icon::GitHub => include_str!("../assets/icons/lucide/github.svg"),
        Icon::Home => include_str!("../assets/icons/lucide/house.svg"),
        Icon::Help => include_str!("../assets/icons/lucide/circle-question-mark.svg"),
        Icon::Keyboard => include_str!("../assets/icons/lucide/keyboard.svg"),
        Icon::Lock => include_str!("../assets/icons/lucide/lock.svg"),
        Icon::LockOpen => include_str!("../assets/icons/lucide/lock-open.svg"),
    }
}

#[derive(Clone)]
enum CachedIconTexture {
    Ready(egui::TextureHandle),
    Failed,
}

fn icon_texture(ctx: &egui::Context, icon: Icon) -> Option<egui::TextureHandle> {
    let cache_id = egui::Id::unique(("papercraft-lucide", icon));
    if let Some(cached) = ctx.data_mut(|data| data.get_temp::<CachedIconTexture>(cache_id)) {
        return match cached {
            CachedIconTexture::Ready(texture) => Some(texture),
            CachedIconTexture::Failed => None,
        };
    }

    // Lucide's currentColor is replaced with white so Painter's tint can apply
    // the active light/dark theme color without duplicating SVGs per theme.
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
        format!("papercraft-lucide-{icon:?}"),
        image,
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|data| {
        data.insert_temp(cache_id, CachedIconTexture::Ready(texture.clone()));
    });
    Some(texture)
}

/// Paints a theme-tinted Lucide SVG centered inside `rect`.
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

struct ButtonStyle {
    max_width: Option<f32>,
    height: f32,
    min_width: f32,
    corner_radius: u8,
    icon_dim: f32,
    font_size: f32,
    pad_left: f32,
    gap: f32,
    pad_right: f32,
    on_header: bool,
}

impl ButtonStyle {
    const fn standard(max_width: Option<f32>) -> Self {
        Self {
            max_width,
            height: 28.0,
            min_width: 28.0,
            corner_radius: 7,
            icon_dim: 15.0,
            font_size: 12.5,
            pad_left: 9.0,
            gap: 6.0,
            pad_right: 10.0,
            on_header: false,
        }
    }

    const fn compact() -> Self {
        Self {
            max_width: None,
            height: 28.0,
            min_width: 28.0,
            corner_radius: 7,
            icon_dim: 14.0,
            font_size: 11.8,
            pad_left: 7.0,
            gap: 4.5,
            pad_right: 8.0,
            on_header: false,
        }
    }

    const fn tray_compact() -> Self {
        Self {
            max_width: None,
            height: 32.0,
            min_width: 32.0,
            corner_radius: 8,
            icon_dim: 16.0,
            font_size: 13.0,
            pad_left: 8.5,
            gap: 5.5,
            pad_right: 9.5,
            on_header: false,
        }
    }

    const fn toolbar() -> Self {
        Self {
            max_width: None,
            height: TOOLBAR_CONTROL_HEIGHT,
            min_width: TOOLBAR_CONTROL_HEIGHT,
            corner_radius: 9,
            icon_dim: TOOLBAR_ICON_DIM,
            font_size: 13.0,
            pad_left: 9.0,
            gap: 6.0,
            pad_right: 10.0,
            on_header: true,
        }
    }

    fn square(size: f32, icon_dim: f32, corner_radius: u8) -> Self {
        let safe_size = if size.is_finite() {
            size.clamp(16.0, 96.0)
        } else {
            28.0
        };
        let safe_icon = if icon_dim.is_finite() {
            icon_dim.clamp(10.0, safe_size)
        } else {
            16.0
        };
        Self {
            max_width: None,
            height: safe_size,
            min_width: safe_size,
            corner_radius,
            icon_dim: safe_icon,
            font_size: 12.5,
            pad_left: 9.0,
            gap: 6.0,
            pad_right: 10.0,
            on_header: false,
        }
    }
}

fn layout_button_label_and_width(
    ui: &egui::Ui,
    label: Option<&str>,
    style: &ButtonStyle,
    text_color: Color32,
) -> (Option<std::sync::Arc<egui::Galley>>, f32) {
    let icon_dim = style.icon_dim;
    let min_width = style.min_width;
    let chrome_w = style.pad_left + icon_dim + style.gap + style.pad_right;
    let max_width = style
        .max_width
        .filter(|w| w.is_finite())
        .map(|w| w.max(min_width));
    let label = label.filter(|txt| !txt.is_empty());
    let galley = label.map(|txt| {
        let font = FontId::proportional(style.font_size);
        match max_width {
            Some(limit_w) => {
                let max_text_w = (limit_w - chrome_w).max(12.0);
                let mut job = egui::text::LayoutJob::single_section(
                    txt.to_string(),
                    egui::TextFormat {
                        font_id: font,
                        color: text_color,
                        ..Default::default()
                    },
                );
                job.wrap.max_width = max_text_w;
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                job.wrap.overflow_character = Some('…');
                ui.painter().layout_job(job)
            }
            None => ui
                .painter()
                .layout_no_wrap(txt.to_string(), font, text_color),
        }
    });

    let width = match (max_width, &galley) {
        (Some(limit_w), Some(galley)) => (chrome_w + galley.size().x).min(limit_w).max(min_width),
        (Some(limit_w), None) => min_width.min(limit_w),
        (None, Some(galley)) => (chrome_w + galley.size().x).max(min_width),
        (None, None) => min_width,
    };

    (galley, width)
}

/// Measures the rendered width of a [`compact_icon_button`] with `label`.
pub fn compact_icon_button_width(ui: &egui::Ui, label: &str) -> f32 {
    layout_button_label_and_width(ui, Some(label), &ButtonStyle::compact(), Color32::BLACK).1
}

/// Measures the rendered width of a [`tray_icon_button`] with `label`.
pub fn tray_icon_button_width(ui: &egui::Ui, label: &str) -> f32 {
    layout_button_label_and_width(
        ui,
        Some(label),
        &ButtonStyle::tray_compact(),
        Color32::BLACK,
    )
    .1
}

/// Measures the rendered width of a [`toolbar_icon_button`] with `label`.
pub fn toolbar_icon_button_width(ui: &egui::Ui, label: &str) -> f32 {
    layout_button_label_and_width(ui, Some(label), &ButtonStyle::toolbar(), Color32::BLACK).1
}

/// Measures the rendered width of a [`max_width_icon_button`] with `label` and `max_width`.
pub fn max_width_icon_button_width(ui: &egui::Ui, label: &str, max_width: f32) -> f32 {
    layout_button_label_and_width(
        ui,
        Some(label),
        &ButtonStyle::standard(Some(max_width)),
        Color32::BLACK,
    )
    .1
}

/// Tactile toolbar or segmented-tab button with an aligned icon and optional label.
pub fn icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: Option<&str>,
    selected: bool,
    enabled: bool,
    t: &Tokens,
) -> Response {
    let mut style = ButtonStyle::standard(None);
    if icon == Icon::Palette {
        style.icon_dim = 20.0;
    }
    draw_styled_icon_button(ui, icon, label, selected, enabled, &style, t)
}

/// Dimensions for [`sized_icon_button`].
#[derive(Clone, Copy, Debug)]
pub struct SquareButtonSpec {
    pub size: f32,
    pub icon_dim: f32,
    pub corner_radius: u8,
}

/// Tactile icon-only square button with configurable outer size, icon size, and corner radius.
pub fn sized_icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    spec: SquareButtonSpec,
    selected: bool,
    enabled: bool,
    t: &Tokens,
) -> Response {
    draw_styled_icon_button(
        ui,
        icon,
        None,
        selected,
        enabled,
        &ButtonStyle::square(spec.size, spec.icon_dim, spec.corner_radius),
        t,
    )
}

/// Compact tactile toolbar button used for denser segmented controls.
pub fn compact_icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: Option<&str>,
    selected: bool,
    enabled: bool,
    t: &Tokens,
) -> Response {
    draw_styled_icon_button(
        ui,
        icon,
        label,
        selected,
        enabled,
        &ButtonStyle::compact(),
        t,
    )
}

/// Tactile button scaled proportionally (`32.0` px tall) for the floating context tray.
pub fn tray_icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: Option<&str>,
    selected: bool,
    enabled: bool,
    t: &Tokens,
) -> Response {
    draw_styled_icon_button(
        ui,
        icon,
        label,
        selected,
        enabled,
        &ButtonStyle::tray_compact(),
        t,
    )
}

/// Outer height of every control in the main editor toolbar row.
pub const TOOLBAR_CONTROL_HEIGHT: f32 = 36.0;
/// Icon size used by main editor toolbar controls.
pub const TOOLBAR_ICON_DIM: f32 = 20.0;

/// Large tactile button (`36.0` px tall, `20.0` px icon) for the main editor toolbar row.
pub fn toolbar_icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: Option<&str>,
    selected: bool,
    enabled: bool,
    t: &Tokens,
) -> Response {
    draw_styled_icon_button(
        ui,
        icon,
        label,
        selected,
        enabled,
        &ButtonStyle::toolbar(),
        t,
    )
}

/// Tactile toolbar button that fits its label closely and truncates with an
/// ellipsis once it reaches `max_width`.
pub fn max_width_icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    max_width: f32,
    selected: bool,
    enabled: bool,
    t: &Tokens,
) -> Response {
    draw_styled_icon_button(
        ui,
        icon,
        Some(label),
        selected,
        enabled,
        &ButtonStyle::standard(Some(max_width)),
        t,
    )
}

fn draw_styled_icon_button(
    ui: &mut egui::Ui,
    icon: Icon,
    label: Option<&str>,
    selected: bool,
    enabled: bool,
    style: &ButtonStyle,
    t: &Tokens,
) -> Response {
    let icon_dim = style.icon_dim;
    let height = style.height;
    let (text_rest, text_muted_base) = if style.on_header {
        (t.header_fg, t.header_fg_muted)
    } else {
        (t.text, t.text_muted)
    };
    let base_color = if !enabled {
        text_muted_base.gamma_multiply(0.55)
    } else {
        text_rest
    };

    let (galley, width) = layout_button_label_and_width(ui, label, style, base_color);

    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), sense);
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let hovered = enabled && response.hovered();
        let hover_t =
            crate::motion::hover_factor(ui.ctx(), response.id.with("hov"), hovered && !selected);
        let select_t = if enabled {
            crate::motion::hover_factor(ui.ctx(), response.id.with("sel"), selected)
        } else {
            0.0
        };
        let (hover_fill, active_fill, rest_stroke_base, hover_stroke, active_stroke, active_fg) =
            if style.on_header {
                let rest_s = if t.is_dark {
                    t.header_border
                } else {
                    Color32::TRANSPARENT
                };
                let act_s = if t.is_dark {
                    t.accent
                } else {
                    t.header_active_bg
                };
                (
                    t.header_control_hover,
                    t.header_active_bg,
                    rest_s,
                    t.header_border,
                    act_s,
                    t.header_active_fg,
                )
            } else {
                (
                    t.border.gamma_multiply(0.45),
                    t.accent_soft,
                    t.border,
                    t.text_muted.gamma_multiply(0.5),
                    t.accent,
                    t.accent,
                )
            };
        let rest_fill = crate::motion::lerp_color(Color32::TRANSPARENT, hover_fill, hover_t);
        let fill = crate::motion::lerp_color(rest_fill, active_fill, select_t);
        let rest_stroke = crate::motion::lerp_color(rest_stroke_base, hover_stroke, hover_t);
        let stroke_color = crate::motion::lerp_color(rest_stroke, active_stroke, select_t);
        let stroke = Stroke::new(crate::motion::lerp_f32(1.0, 1.5, select_t), stroke_color);
        let foreground = if !enabled {
            base_color
        } else {
            crate::motion::lerp_color(text_rest, active_fg, select_t)
        };
        painter.rect_filled(rect, CornerRadius::same(style.corner_radius), fill);
        painter.rect_stroke(
            rect,
            CornerRadius::same(style.corner_radius),
            stroke,
            StrokeKind::Inside,
        );

        let icon_rect = match &galley {
            Some(galley) => {
                let icon_rect = Rect::from_center_size(
                    pos2(
                        rect.left() + style.pad_left + icon_dim * 0.5,
                        rect.center().y,
                    ),
                    Vec2::splat(icon_dim),
                );
                let text_pos = pos2(
                    icon_rect.right() + style.gap,
                    rect.center().y - galley.size().y * 0.5,
                );
                painter.galley_with_override_text_color(text_pos, galley.clone(), foreground);
                icon_rect
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

/// Size variant for [`sidebar_edge_button_sized`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SidebarEdgeSize {
    /// Standard `28.0` px pull-tab used by Home and Profile headers.
    #[default]
    Standard,
    /// Large `36.0` px pull-tab matching the main editor toolbar controls.
    Toolbar,
}

#[derive(Clone, Copy, Debug)]
struct EdgeMetrics {
    height: f32,
    min_width: f32,
    bar_w: f32,
    pad_outer: f32,
    icon_dim: f32,
    icon_gap: f32,
    chevron_gap: f32,
    chevron_dim: f32,
    pad_inner: f32,
    font_size: f32,
    inner_radius: u8,
}

impl EdgeMetrics {
    fn chrome_with_label(&self) -> f32 {
        self.pad_outer
            + self.icon_dim
            + self.icon_gap
            + self.chevron_gap
            + self.chevron_dim
            + self.pad_inner
    }

    fn chrome_no_label(&self) -> f32 {
        self.pad_outer + self.icon_dim + self.chevron_gap + self.chevron_dim + self.pad_inner
    }
}

impl SidebarEdgeSize {
    const fn metrics(self) -> EdgeMetrics {
        match self {
            Self::Standard => EdgeMetrics {
                height: 28.0,
                min_width: 36.0,
                bar_w: 3.0,
                pad_outer: 9.0,
                icon_dim: 14.0,
                icon_gap: 5.0,
                chevron_gap: 4.0,
                chevron_dim: 12.0,
                pad_inner: 8.0,
                font_size: 12.0,
                inner_radius: 14,
            },
            Self::Toolbar => EdgeMetrics {
                height: TOOLBAR_CONTROL_HEIGHT,
                min_width: 44.0,
                bar_w: 3.0,
                pad_outer: 11.0,
                icon_dim: TOOLBAR_ICON_DIM,
                icon_gap: 6.0,
                chevron_gap: 4.0,
                chevron_dim: 14.0,
                pad_inner: 9.0,
                font_size: 13.0,
                inner_radius: 18,
            },
        }
    }
}

/// Which window edge a flush sidebar drawer pull-tab attaches to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SidebarEdge {
    Left,
    Right,
}

/// Visual and behavioral configuration for [`sidebar_edge_button`].
#[derive(Clone, Copy, Debug)]
pub struct SidebarEdgeButtonSpec<'a> {
    pub edge: SidebarEdge,
    pub icon: Icon,
    pub label: &'a str,
    pub max_width: Option<f32>,
    pub open: bool,
    pub enabled: bool,
}

fn layout_sidebar_edge_button(
    ui: &egui::Ui,
    label: &str,
    max_width: Option<f32>,
    text_color: Color32,
    m: &EdgeMetrics,
) -> (Option<std::sync::Arc<egui::Galley>>, f32) {
    let chrome_with_label = m.chrome_with_label();
    let chrome_no_label = m.chrome_no_label();
    let max_width = max_width
        .filter(|w| w.is_finite())
        .map(|w| w.max(m.min_width));
    let show_label =
        !label.is_empty() && max_width.is_none_or(|limit| limit >= chrome_with_label + 12.0);
    let galley = if show_label {
        let font = FontId::proportional(m.font_size);
        Some(match max_width {
            Some(limit_w) => {
                let max_text_w = (limit_w - chrome_with_label).max(12.0);
                let mut job = egui::text::LayoutJob::single_section(
                    label.to_string(),
                    egui::TextFormat {
                        font_id: font,
                        color: text_color,
                        ..Default::default()
                    },
                );
                job.wrap.max_width = max_text_w;
                job.wrap.max_rows = 1;
                job.wrap.break_anywhere = true;
                job.wrap.overflow_character = Some('…');
                ui.painter().layout_job(job)
            }
            None => ui
                .painter()
                .layout_no_wrap(label.to_string(), font, text_color),
        })
    } else {
        None
    };

    let width = match (max_width, &galley) {
        (Some(limit_w), Some(galley)) => (chrome_with_label + galley.size().x)
            .min(limit_w)
            .max(m.min_width),
        (Some(limit_w), None) => chrome_no_label.min(limit_w).max(m.min_width),
        (None, Some(galley)) => (chrome_with_label + galley.size().x).max(m.min_width),
        (None, None) => chrome_no_label.max(m.min_width),
    };

    (galley, width)
}

/// Measures the rendered width of a [`sidebar_edge_button`].
pub fn sidebar_edge_button_width(ui: &egui::Ui, label: &str, max_width: Option<f32>) -> f32 {
    sidebar_edge_button_sized_width(ui, label, max_width, SidebarEdgeSize::Standard)
}

/// Measures the rendered width of a [`sidebar_edge_button_sized`].
pub fn sidebar_edge_button_sized_width(
    ui: &egui::Ui,
    label: &str,
    max_width: Option<f32>,
    size: SidebarEdgeSize,
) -> f32 {
    layout_sidebar_edge_button(ui, label, max_width, Color32::BLACK, &size.metrics()).1
}

struct SidebarEdgeChromeDraw<'a> {
    rect: Rect,
    edge: SidebarEdge,
    hover_t: f32,
    open_t: f32,
    on_header: bool,
    m: &'a EdgeMetrics,
}

fn paint_sidebar_edge_chrome(
    painter: &egui::Painter,
    chrome: &SidebarEdgeChromeDraw<'_>,
    t: &Tokens,
) {
    let rect = chrome.rect;
    let hover_t = chrome.hover_t;
    let open_t = chrome.open_t;
    let m = chrome.m;
    let (closed_fill, active_fill, closed_stroke, open_stroke, rest_rail, active_rail) =
        if chrome.on_header {
            let act_accent = if t.is_dark {
                t.accent
            } else {
                t.header_active_bg
            };
            (
                crate::motion::lerp_color(t.header_track_bg, t.header_control_hover, hover_t),
                t.header_active_bg,
                crate::motion::lerp_color(
                    t.header_border,
                    t.header_fg_muted.gamma_multiply(0.65),
                    hover_t,
                ),
                crate::motion::lerp_color(act_accent, t.accent_hover, hover_t * 0.5),
                t.header_fg_muted.gamma_multiply(0.45),
                act_accent,
            )
        } else {
            (
                crate::motion::lerp_color(t.field_bg, t.accent_soft, hover_t * 0.55),
                t.accent_soft,
                crate::motion::lerp_color(t.border, t.accent.gamma_multiply(0.65), hover_t),
                crate::motion::lerp_color(t.accent, t.accent_hover, hover_t * 0.5),
                t.text_muted
                    .gamma_multiply(if t.is_dark { 0.42 } else { 0.32 }),
                t.accent,
            )
        };
    let fill = crate::motion::lerp_color(closed_fill, active_fill, open_t);
    let stroke_color = crate::motion::lerp_color(closed_stroke, open_stroke, open_t);
    let stroke_w = crate::motion::lerp_f32(1.0, 1.35, open_t);
    let closed_rail =
        crate::motion::lerp_color(rest_rail, active_rail.gamma_multiply(0.85), hover_t);
    let rail_color = crate::motion::lerp_color(closed_rail, active_rail, open_t);

    let (draw_rect, radii, rail_rect) = match chrome.edge {
        SidebarEdge::Left => (
            Rect::from_min_max(pos2(rect.left() - 2.0, rect.top()), rect.max),
            CornerRadius {
                nw: 0,
                sw: 0,
                ne: m.inner_radius,
                se: m.inner_radius,
            },
            Rect::from_min_max(rect.min, pos2(rect.left() + m.bar_w, rect.bottom())),
        ),
        SidebarEdge::Right => (
            Rect::from_min_max(rect.min, pos2(rect.right() + 2.0, rect.bottom())),
            CornerRadius {
                nw: m.inner_radius,
                sw: m.inner_radius,
                ne: 0,
                se: 0,
            },
            Rect::from_min_max(pos2(rect.right() - m.bar_w, rect.top()), rect.max),
        ),
    };

    let clipped = painter.with_clip_rect(painter.clip_rect().intersect(rect));
    clipped.rect_filled(draw_rect, radii, fill);
    clipped.rect_stroke(
        draw_rect,
        radii,
        Stroke::new(stroke_w, stroke_color),
        StrokeKind::Inside,
    );
    clipped.rect_filled(rail_rect, CornerRadius::ZERO, rail_color);
}

struct SidebarEdgeContentDraw<'a> {
    rect: Rect,
    spec: &'a SidebarEdgeButtonSpec<'a>,
    galley: Option<&'a std::sync::Arc<egui::Galley>>,
    hover_t: f32,
    open_t: f32,
    on_header: bool,
    m: EdgeMetrics,
}

fn paint_sidebar_edge_contents(
    painter: &egui::Painter,
    draw: &SidebarEdgeContentDraw<'_>,
    t: &Tokens,
) {
    let m = &draw.m;
    let rect = draw.rect;
    let spec = draw.spec;
    let cy = rect.center().y;
    let (rest_fg, muted_fg, open_fg, chev_hover_fg) = if draw.on_header {
        (
            t.header_fg,
            t.header_fg_muted,
            t.header_active_fg,
            if t.is_dark { t.accent } else { t.header_fg },
        )
    } else {
        (t.text, t.text_muted, t.accent, t.accent)
    };
    let foreground = if !spec.enabled {
        muted_fg.gamma_multiply(0.55)
    } else {
        crate::motion::lerp_color(rest_fg, open_fg, draw.open_t)
    };
    let chevron_color = if !spec.enabled {
        muted_fg.gamma_multiply(0.45)
    } else {
        let rest_chev = crate::motion::lerp_color(muted_fg, chev_hover_fg, draw.hover_t);
        crate::motion::lerp_color(rest_chev, open_fg, draw.open_t)
    };

    let (chevron_icon, nudge_x) = match (spec.edge, spec.open) {
        (SidebarEdge::Left, false) => (Icon::NextPage, 1.5 * draw.hover_t),
        (SidebarEdge::Left, true) => (Icon::PrevPage, -1.5 * draw.hover_t),
        (SidebarEdge::Right, false) => (Icon::PrevPage, -1.5 * draw.hover_t),
        (SidebarEdge::Right, true) => (Icon::NextPage, 1.5 * draw.hover_t),
    };

    let (icon_rect, chevron_rect, text_clip_max_x) = match spec.edge {
        SidebarEdge::Left => {
            let icon_cx = rect.left() + m.pad_outer + m.icon_dim * 0.5;
            let chevron_cx = rect.right() - m.pad_inner - m.chevron_dim * 0.5 + nudge_x;
            let icon_rect = Rect::from_center_size(pos2(icon_cx, cy), Vec2::splat(m.icon_dim));
            let chevron_rect =
                Rect::from_center_size(pos2(chevron_cx, cy), Vec2::splat(m.chevron_dim));
            let clip_max_x = (rect.right() - m.pad_inner - m.chevron_dim + nudge_x.min(0.0) - 2.0)
                .max(icon_rect.right());
            (icon_rect, chevron_rect, clip_max_x)
        }
        SidebarEdge::Right => {
            let chevron_cx = rect.left() + m.pad_inner + m.chevron_dim * 0.5 + nudge_x;
            let icon_cx =
                rect.left() + m.pad_inner + m.chevron_dim + m.chevron_gap + m.icon_dim * 0.5;
            let chevron_rect =
                Rect::from_center_size(pos2(chevron_cx, cy), Vec2::splat(m.chevron_dim));
            let icon_rect = Rect::from_center_size(pos2(icon_cx, cy), Vec2::splat(m.icon_dim));
            let clip_max_x = (rect.right() - m.pad_outer + 2.0).max(icon_rect.right());
            (icon_rect, chevron_rect, clip_max_x)
        }
    };

    let content_painter = painter.with_clip_rect(painter.clip_rect().intersect(rect));
    paint_icon(&content_painter, icon_rect, spec.icon, foreground);
    if let Some(galley) = draw.galley {
        let text_x = icon_rect.right() + m.icon_gap;
        let text_pos = pos2(text_x, cy - galley.size().y * 0.5);
        let text_clip = Rect::from_min_max(
            pos2(text_x - 1.0, rect.top()),
            pos2(text_clip_max_x, rect.bottom()),
        );
        let text_painter = painter.with_clip_rect(painter.clip_rect().intersect(text_clip));
        text_painter.galley_with_override_text_color(text_pos, (*galley).clone(), foreground);
    }
    paint_icon(&content_painter, chevron_rect, chevron_icon, chevron_color);
}

/// Flush-to-edge sidebar pull-tab toggle button with docked anchor rail,
/// rounded inner half-pill end, optional truncated label, and directional chevron.
pub fn sidebar_edge_button(
    ui: &mut egui::Ui,
    spec: SidebarEdgeButtonSpec<'_>,
    t: &Tokens,
) -> Response {
    sidebar_edge_button_sized(ui, spec, SidebarEdgeSize::Standard, t)
}

/// [`sidebar_edge_button`] rendered at an explicit [`SidebarEdgeSize`].
pub fn sidebar_edge_button_sized(
    ui: &mut egui::Ui,
    spec: SidebarEdgeButtonSpec<'_>,
    size: SidebarEdgeSize,
    t: &Tokens,
) -> Response {
    let m = size.metrics();
    let on_header = matches!(size, SidebarEdgeSize::Toolbar);
    let base_color = if !spec.enabled {
        if on_header {
            t.header_fg_muted.gamma_multiply(0.55)
        } else {
            t.text_muted.gamma_multiply(0.55)
        }
    } else if on_header {
        t.header_fg
    } else {
        t.text
    };
    let (galley, width) =
        layout_sidebar_edge_button(ui, spec.label, spec.max_width, base_color, &m);

    let sense = if spec.enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, m.height), sense);
    if ui.is_rect_visible(rect) {
        let hovered = spec.enabled && response.hovered();
        let hover_t = crate::motion::hover_factor(ui.ctx(), response.id.with("hov"), hovered);
        let open_t = if spec.enabled {
            crate::motion::hover_factor(ui.ctx(), response.id.with("open"), spec.open)
        } else {
            0.0
        };
        let painter = ui.painter();
        paint_sidebar_edge_chrome(
            painter,
            &SidebarEdgeChromeDraw {
                rect,
                edge: spec.edge,
                hover_t,
                open_t,
                on_header,
                m: &m,
            },
            t,
        );
        paint_sidebar_edge_contents(
            painter,
            &SidebarEdgeContentDraw {
                rect,
                spec: &spec,
                galley: galley.as_ref(),
                hover_t,
                open_t,
                on_header,
                m,
            },
            t,
        );
    }
    if spec.enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

/// Layout strategy for [`segmented_switch`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwitchLayout {
    /// Inline compact switch (`28.0` px tall) hugging its segment labels and icons.
    Compact,
    /// Inline floating-tray switch (`32.0` px tall) proportionally scaled for the context tray.
    TrayCompact,
    /// Inline main-toolbar switch (`36.0` px tall) with large `20.0` px icons.
    Toolbar,
    /// Full-width switch dividing `ui.available_width()` equally across all segments (`32.0` px tall).
    EqualWidth,
}

struct SwitchMetrics {
    track_h: f32,
    track_radius: u8,
    pill_radius: u8,
    font_size: f32,
    icon_dim: f32,
    pad_x: f32,
    gap: f32,
    equal_width: bool,
}

impl SwitchLayout {
    const fn metrics(self) -> SwitchMetrics {
        match self {
            Self::Compact => SwitchMetrics {
                track_h: 28.0,
                track_radius: 7,
                pill_radius: 5,
                font_size: 11.8,
                icon_dim: 14.0,
                pad_x: 7.0,
                gap: 4.5,
                equal_width: false,
            },
            Self::TrayCompact => SwitchMetrics {
                track_h: 32.0,
                track_radius: 8,
                pill_radius: 6,
                font_size: 13.0,
                icon_dim: 16.0,
                pad_x: 8.5,
                gap: 5.5,
                equal_width: false,
            },
            Self::Toolbar => SwitchMetrics {
                track_h: TOOLBAR_CONTROL_HEIGHT,
                track_radius: 9,
                pill_radius: 7,
                font_size: 13.0,
                icon_dim: TOOLBAR_ICON_DIM,
                pad_x: 8.0,
                gap: 5.5,
                equal_width: false,
            },
            Self::EqualWidth => SwitchMetrics {
                track_h: 32.0,
                track_radius: 8,
                pill_radius: 6,
                font_size: 12.0,
                icon_dim: 14.0,
                pad_x: 7.0,
                gap: 4.5,
                equal_width: true,
            },
        }
    }
}

/// One mutually exclusive option rendered inside [`segmented_switch`].
#[derive(Clone, Copy, Debug)]
pub struct SwitchOption<'a> {
    pub icon: Option<Icon>,
    pub label: Option<&'a str>,
    pub tooltip: &'a str,
    pub enabled: bool,
}

impl<'a> SwitchOption<'a> {
    pub const fn new(
        icon: Option<Icon>,
        label: Option<&'a str>,
        tooltip: &'a str,
        enabled: bool,
    ) -> Self {
        Self {
            icon,
            label,
            tooltip,
            enabled,
        }
    }
}

struct MeasuredSegment {
    galley: Option<std::sync::Arc<egui::Galley>>,
    compact_width: f32,
}

fn measure_switch_segment(
    ui: &egui::Ui,
    opt: &SwitchOption<'_>,
    metrics: &SwitchMetrics,
    seg_h: f32,
) -> MeasuredSegment {
    let galley = opt.label.filter(|txt| !txt.is_empty()).map(|txt| {
        ui.painter().layout_no_wrap(
            txt.to_string(),
            FontId::proportional(metrics.font_size),
            Color32::WHITE,
        )
    });
    let compact_width = match (opt.icon.is_some(), &galley) {
        (true, Some(g)) => {
            (metrics.pad_x + metrics.icon_dim + metrics.gap + g.size().x + metrics.pad_x).max(seg_h)
        }
        (true, None) => seg_h,
        (false, Some(g)) => (metrics.pad_x + 2.0 + g.size().x + metrics.pad_x + 2.0).max(34.0),
        (false, None) => seg_h,
    };
    MeasuredSegment {
        galley,
        compact_width,
    }
}

fn paint_switch_segment_content(
    painter: &egui::Painter,
    seg_rect: Rect,
    icon: Option<Icon>,
    galley: Option<&std::sync::Arc<egui::Galley>>,
    metrics: &SwitchMetrics,
    fg: Color32,
) {
    let icon_dim = metrics.icon_dim;
    let gap = metrics.gap;
    let cy = seg_rect.center().y;
    let p = painter.with_clip_rect(seg_rect.shrink2(Vec2::new(2.0, 0.0)));
    match (icon, galley) {
        (Some(icon), Some(g)) => {
            let total_w = icon_dim + gap + g.size().x;
            if total_w <= seg_rect.width() - 4.0 {
                let start_x = seg_rect.center().x - total_w * 0.5;
                let icon_rect = Rect::from_center_size(
                    pos2(start_x + icon_dim * 0.5, cy),
                    Vec2::splat(icon_dim),
                );
                paint_icon(&p, icon_rect, icon, fg);
                let text_pos = pos2(start_x + icon_dim + gap, cy - g.size().y * 0.5);
                p.galley_with_override_text_color(text_pos, g.clone(), fg);
            } else if g.size().x <= seg_rect.width() - 4.0 {
                let text_pos = pos2(
                    seg_rect.center().x - g.size().x * 0.5,
                    cy - g.size().y * 0.5,
                );
                p.galley_with_override_text_color(text_pos, g.clone(), fg);
            } else {
                let start_x = seg_rect.left() + 3.0;
                let icon_rect = Rect::from_center_size(
                    pos2(start_x + icon_dim * 0.5, cy),
                    Vec2::splat(icon_dim),
                );
                paint_icon(&p, icon_rect, icon, fg);
                let text_pos = pos2(start_x + icon_dim + gap, cy - g.size().y * 0.5);
                p.galley_with_override_text_color(text_pos, g.clone(), fg);
            }
        }
        (Some(icon), None) => {
            let icon_rect = Rect::from_center_size(seg_rect.center(), Vec2::splat(icon_dim));
            paint_icon(&p, icon_rect, icon, fg);
        }
        (None, Some(g)) => {
            let text_pos = pos2(
                seg_rect.center().x - g.size().x * 0.5,
                cy - g.size().y * 0.5,
            );
            p.galley_with_override_text_color(text_pos, g.clone(), fg);
        }
        (None, None) => {}
    }
}

fn compute_switch_track_width(
    ui: &egui::Ui,
    measured: &[MeasuredSegment],
    metrics: &SwitchMetrics,
    track_pad: f32,
    gaps_total: f32,
) -> f32 {
    let n = measured.len();
    if metrics.equal_width {
        ui.available_width().max(n as f32 * 20.0)
    } else {
        let sum_w: f32 = measured.iter().map(|m| m.compact_width).sum();
        track_pad * 2.0 + sum_w + gaps_total
    }
}

/// Measures the exact rendered track width of a [`segmented_switch`].
pub fn segmented_switch_width(
    ui: &egui::Ui,
    options: &[SwitchOption<'_>],
    layout: SwitchLayout,
) -> f32 {
    if options.is_empty() {
        return 0.0;
    }
    let metrics = layout.metrics();
    let track_pad = 2.0_f32;
    let seg_gap = 2.0_f32;
    let seg_h = metrics.track_h - track_pad * 2.0;
    let measured: Vec<MeasuredSegment> = options
        .iter()
        .map(|opt| measure_switch_segment(ui, opt, &metrics, seg_h))
        .collect();
    let gaps_total = seg_gap * options.len().saturating_sub(1) as f32;
    compute_switch_track_width(ui, &measured, &metrics, track_pad, gaps_total)
}

/// Renders an animated segmented switch for 2, 3, or more mutually exclusive options.
///
/// Returns `(clicked_index, track_response)` so callers can handle both switching and
/// optional re-click actions while animating the active pill smoothly across segments.
pub fn segmented_switch(
    ui: &mut egui::Ui,
    id_salt: impl egui::AsIdSalt,
    options: &[SwitchOption<'_>],
    active: Option<usize>,
    layout: SwitchLayout,
    t: &Tokens,
) -> (Option<usize>, Response) {
    if options.is_empty() {
        let (_, empty_resp) = ui.allocate_exact_size(Vec2::ZERO, Sense::hover());
        return (None, empty_resp);
    }

    let metrics = layout.metrics();
    let track_h = metrics.track_h;
    let track_radius = metrics.track_radius;
    let pill_radius = metrics.pill_radius;
    let track_pad = 2.0_f32;
    let seg_gap = 2.0_f32;
    let seg_h = track_h - track_pad * 2.0;

    let measured: Vec<MeasuredSegment> = options
        .iter()
        .map(|opt| measure_switch_segment(ui, opt, &metrics, seg_h))
        .collect();

    let n = options.len();
    let gaps_total = seg_gap * n.saturating_sub(1) as f32;
    let track_w = compute_switch_track_width(ui, &measured, &metrics, track_pad, gaps_total);
    let inner_track_w = (track_w - track_pad * 2.0 - gaps_total).max(20.0);
    let equal_seg_w = inner_track_w / n as f32;
    let max_compact_w = measured
        .iter()
        .map(|m| m.compact_width)
        .fold(0.0_f32, f32::max);
    let sum_compact_w: f32 = measured.iter().map(|m| m.compact_width).sum();
    let use_proportional = metrics.equal_width && max_compact_w > equal_seg_w + 0.5;

    let track_id = ui.make_persistent_id(id_salt);
    let (track_rect, track_resp) =
        ui.allocate_exact_size(Vec2::new(track_w, track_h), Sense::hover());

    let mut seg_rects = Vec::with_capacity(n);
    let mut x = track_rect.left() + track_pad;
    let seg_top = track_rect.top() + track_pad;
    for item in &measured {
        let w = if use_proportional {
            if sum_compact_w <= inner_track_w {
                item.compact_width + (inner_track_w - sum_compact_w) / n as f32
            } else if sum_compact_w > 0.0 {
                inner_track_w * (item.compact_width / sum_compact_w)
            } else {
                equal_seg_w
            }
        } else if metrics.equal_width {
            equal_seg_w
        } else {
            item.compact_width
        };
        seg_rects.push(Rect::from_min_size(pos2(x, seg_top), Vec2::new(w, seg_h)));
        x += w + seg_gap;
    }

    let mut clicked = None;
    let mut hovered_flags = Vec::with_capacity(n);
    for (idx, opt) in options.iter().enumerate() {
        let seg_id = track_id.with(("seg", idx));
        let sense = if opt.enabled {
            Sense::click()
        } else {
            Sense::hover()
        };
        let mut resp = ui.interact(seg_rects[idx], seg_id, sense);
        if opt.enabled {
            resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
        if !opt.tooltip.is_empty() {
            resp = resp.on_hover_text(opt.tooltip);
        }
        if opt.enabled && resp.clicked() {
            clicked = Some(idx);
        }
        hovered_flags.push(opt.enabled && resp.hovered());
    }

    if ui.is_rect_visible(track_rect) {
        let on_header = matches!(layout, SwitchLayout::Toolbar);
        let (track_bg, track_border, active_fill, active_border, fg_muted, fg_rest, active_fg) =
            if on_header {
                let act_border = if t.is_dark {
                    t.accent
                } else {
                    t.header_active_bg
                };
                (
                    t.header_track_bg,
                    t.header_border,
                    t.header_active_bg,
                    act_border,
                    t.header_fg_muted,
                    t.header_fg,
                    t.header_active_fg,
                )
            } else {
                (
                    t.toolbar_bg,
                    t.border,
                    t.accent_soft,
                    t.accent,
                    t.text_muted,
                    t.text,
                    t.accent,
                )
            };
        let p = ui.painter();
        p.rect(
            track_rect,
            CornerRadius::same(track_radius),
            track_bg,
            Stroke::new(1.0, track_border),
            StrokeKind::Inside,
        );

        for (idx, opt) in options.iter().enumerate() {
            let is_active = active == Some(idx);
            let hover_t = crate::motion::hover_factor(
                ui.ctx(),
                track_id.with(("hov", idx)),
                hovered_flags[idx] && !is_active,
            );
            if opt.enabled && hover_t > 0.02 {
                let hov_fill = if on_header {
                    t.header_control_hover.gamma_multiply(hover_t)
                } else {
                    t.border.gamma_multiply(0.35 * hover_t)
                };
                p.rect_filled(seg_rects[idx], CornerRadius::same(pill_radius), hov_fill);
            }
        }

        if let Some(active_idx) = active.filter(|&i| i < n) {
            let target_r = seg_rects[active_idx];
            let rel_x0 = target_r.left() - track_rect.left();
            let rel_x1 = target_r.right() - track_rect.left();
            let anim_x0 = track_rect.left()
                + ui.ctx().animate_value_with_time(
                    track_id.with("pill_x0"),
                    rel_x0,
                    crate::motion::MICRO_DURATION_S,
                );
            let anim_x1 = track_rect.left()
                + ui.ctx().animate_value_with_time(
                    track_id.with("pill_x1"),
                    rel_x1,
                    crate::motion::MICRO_DURATION_S,
                );
            let pill_rect = Rect::from_min_max(
                pos2(anim_x0, target_r.top()),
                pos2(anim_x1, target_r.bottom()),
            );
            p.rect(
                pill_rect,
                CornerRadius::same(pill_radius),
                active_fill,
                Stroke::new(1.2, active_border),
                StrokeKind::Inside,
            );
        }

        for (idx, opt) in options.iter().enumerate() {
            let is_active = active == Some(idx);
            let fg = if !opt.enabled {
                fg_muted.gamma_multiply(0.55)
            } else {
                let select_t =
                    crate::motion::hover_factor(ui.ctx(), track_id.with(("sel", idx)), is_active);
                let hover_t = crate::motion::hover_factor(
                    ui.ctx(),
                    track_id.with(("hov", idx)),
                    hovered_flags[idx] && !is_active,
                );
                let mix = if on_header { 0.7 } else { 0.35 };
                let rest_fg = crate::motion::lerp_color(fg_muted, fg_rest, mix);
                let hovered_fg = crate::motion::lerp_color(rest_fg, fg_rest, hover_t);
                crate::motion::lerp_color(hovered_fg, active_fg, select_t)
            };
            paint_switch_segment_content(
                p,
                seg_rects[idx],
                opt.icon,
                measured[idx].galley.as_ref(),
                &metrics,
                fg,
            );
        }
    }

    (clicked, track_resp)
}

/// Renders a compact Lucide close/remove icon button (used inside tag and template chips).
pub fn chip_close_button(ui: &mut egui::Ui, tooltip: &str, color: Color32, t: &Tokens) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
    if ui.is_rect_visible(rect) {
        if resp.hovered() {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(4), t.border.gamma_multiply(0.45));
        }
        let fg = if resp.hovered() { t.text } else { color };
        paint_icon(ui.painter(), rect.shrink(2.5), Icon::Close, fg);
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tooltip)
}

/// Renders an animated boolean pill toggle switch (`36x20`) styled with active theme tokens.
pub fn toggle_switch(
    ui: &mut egui::Ui,
    id_salt: impl egui::AsIdSalt,
    on: &mut bool,
    enabled: bool,
    t: &Tokens,
) -> Response {
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
            crate::motion::MICRO_DURATION_S,
        );
        let alpha = if enabled { 1.0 } else { 0.55 };
        let track_fill =
            crate::motion::lerp_color(t.field_bg, t.accent, anim_t).gamma_multiply(alpha);
        let track_stroke =
            crate::motion::lerp_color(t.border, t.accent, anim_t).gamma_multiply(alpha);
        let knob_fill =
            crate::motion::lerp_color(t.text_muted, t.on_accent, anim_t).gamma_multiply(alpha);

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
) -> Response {
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

/// Renders a section heading with an icon and muted plain-English subtitle.
pub fn section_card_header(ui: &mut egui::Ui, icon: Icon, title: &str, subtitle: &str, t: &Tokens) {
    ui.horizontal(|ui| {
        let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(18.0), Sense::hover());
        paint_icon(ui.painter(), icon_rect, icon, t.accent);
        ui.add(
            egui::Label::new(egui::RichText::new(title).size(15.0).strong().color(t.text)).wrap(),
        );
    });
    ui.add(egui::Label::new(egui::RichText::new(subtitle).size(11.5).color(t.text_muted)).wrap());
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use super::{
        Icon, SidebarEdge, SidebarEdgeButtonSpec, SidebarEdgeSize, SwitchLayout, SwitchOption,
        TOOLBAR_CONTROL_HEIGHT, max_width_icon_button, max_width_icon_button_width, paint_icon,
        segmented_switch, sidebar_edge_button, sidebar_edge_button_sized,
        sidebar_edge_button_sized_width, sidebar_edge_button_width, toggle_switch,
        toggle_switch_row, toolbar_icon_button,
    };
    use crate::theme::Tokens;
    use egui::{Color32, Pos2, RawInput, Rect, Vec2, epaint::Primitive};

    fn all_icons() -> &'static [(Icon, &'static str)] {
        &[
            (Icon::FountainPen, "FountainPen"),
            (Icon::BallPen, "BallPen"),
            (Icon::BrushPen, "BrushPen"),
            (Icon::Highlighter, "Highlighter"),
            (Icon::HighlighterFreehand, "HighlighterFreehand"),
            (Icon::HighlighterStraight, "HighlighterStraight"),
            (Icon::Eraser, "Eraser"),
            (Icon::StrokeEraser, "StrokeEraser"),
            (Icon::PrecisionEraser, "PrecisionEraser"),
            (Icon::Undo, "Undo"),
            (Icon::Redo, "Redo"),
            (Icon::ClearPage, "ClearPage"),
            (Icon::FitPage, "FitPage"),
            (Icon::ZoomIn, "ZoomIn"),
            (Icon::ZoomOut, "ZoomOut"),
            (Icon::Settings, "Settings"),
            (Icon::Preset, "Preset"),
            (Icon::Stabilizer, "Stabilizer"),
            (Icon::Pressure, "Pressure"),
            (Icon::Paper, "Paper"),
            (Icon::Reset, "Reset"),
            (Icon::Info, "Info"),
            (Icon::Notebook, "Notebook"),
            (Icon::Library, "Library"),
            (Icon::Search, "Search"),
            (Icon::Plus, "Plus"),
            (Icon::Export, "Export"),
            (Icon::AddPage, "AddPage"),
            (Icon::Duplicate, "Duplicate"),
            (Icon::PrevPage, "PrevPage"),
            (Icon::NextPage, "NextPage"),
            (Icon::Close, "Close"),
            (Icon::Palette, "Palette"),
            (Icon::Edit, "Edit"),
            (Icon::Trash, "Trash"),
            (Icon::Check, "Check"),
            (Icon::Sun, "Sun"),
            (Icon::Moon, "Moon"),
            (Icon::Account, "Account"),
            (Icon::Sync, "Sync"),
            (Icon::CheckCircle, "CheckCircle"),
            (Icon::AlertCircle, "AlertCircle"),
            (Icon::Server, "Server"),
            (Icon::Eye, "Eye"),
            (Icon::EyeOff, "EyeOff"),
            (Icon::ShapeTool, "ShapeTool"),
            (Icon::LineShape, "LineShape"),
            (Icon::RectangleShape, "RectangleShape"),
            (Icon::EllipseShape, "EllipseShape"),
            (Icon::TriangleShape, "TriangleShape"),
            (Icon::ArrowShape, "ArrowShape"),
            (Icon::Ruler, "Ruler"),
            (Icon::Select, "Select"),
            (Icon::Image, "Image"),
            (Icon::Link, "Link"),
            (Icon::Folder, "Folder"),
            (Icon::FolderOpen, "FolderOpen"),
            (Icon::FolderPlus, "FolderPlus"),
            (Icon::FolderPen, "FolderPen"),
            (Icon::MoveToFolder, "MoveToFolder"),
            (Icon::Inbox, "Inbox"),
            (Icon::ChevronUp, "ChevronUp"),
            (Icon::ChevronDown, "ChevronDown"),
            (Icon::More, "More"),
            (Icon::Star, "Star"),
            (Icon::Tag, "Tag"),
            (Icon::ListView, "ListView"),
            (Icon::GridView, "GridView"),
            (Icon::Sort, "Sort"),
            (Icon::MoveUp, "MoveUp"),
            (Icon::MoveDown, "MoveDown"),
            (Icon::Bookmark, "Bookmark"),
            (Icon::RotateCw, "RotateCw"),
            (Icon::CheckSquare, "CheckSquare"),
            (Icon::Transfer, "Transfer"),
            (Icon::Pencil, "Pencil"),
            (Icon::Lasso, "Lasso"),
            (Icon::PolygonShape, "PolygonShape"),
            (Icon::FillBucket, "FillBucket"),
            (Icon::Paste, "Paste"),
            (Icon::Bold, "Bold"),
            (Icon::Italic, "Italic"),
            (Icon::Underline, "Underline"),
            (Icon::AlignLeft, "AlignLeft"),
            (Icon::AlignCenter, "AlignCenter"),
            (Icon::AlignRight, "AlignRight"),
            (Icon::CheckList, "CheckList"),
            (Icon::Sticker, "Sticker"),
            (Icon::LaserPointer, "LaserPointer"),
            (Icon::GripHorizontal, "GripHorizontal"),
            (Icon::Text, "Text"),
            (Icon::GitHub, "GitHub"),
            (Icon::Home, "Home"),
            (Icon::Lock, "Lock"),
            (Icon::LockOpen, "LockOpen"),
        ]
    }

    #[test]
    fn all_icons_tessellate_within_bounds_without_miter_spikes() {
        let icon_rect = Rect::from_min_size(Pos2::new(20.0, 20.0), Vec2::splat(15.0));
        let safe_bounds = icon_rect.expand(2.0);
        let moon_bounds = icon_rect.expand(0.5);
        for &(icon, name) in all_icons() {
            let ctx = egui::Context::default();
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::splat(80.0))),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                paint_icon(ui.painter(), icon_rect, icon, Color32::WHITE);
            });
            let primitives = ctx.tessellate(out.shapes.clone(), 1.0);
            out.drop_without_applying_deltas();

            let bounds = if icon == Icon::Moon {
                moon_bounds
            } else {
                safe_bounds
            };
            let mut vertex_count = 0usize;
            for clipped in &primitives {
                if let Primitive::Mesh(mesh) = &clipped.primitive {
                    for vertex in &mesh.vertices {
                        vertex_count += 1;
                        assert!(
                            bounds.contains(vertex.pos),
                            "icon {name} emitted out-of-bounds vertex {:?} outside {:?}",
                            vertex.pos,
                            bounds
                        );
                    }
                }
            }
            assert!(vertex_count > 0, "icon {name} emitted zero mesh vertices");
        }
    }

    fn collect_galleys(shapes: &[egui::epaint::ClippedShape]) -> Vec<std::sync::Arc<egui::Galley>> {
        let mut galleys = Vec::new();
        fn visit(shape: &egui::Shape, out: &mut Vec<std::sync::Arc<egui::Galley>>) {
            match shape {
                egui::Shape::Text(text) => out.push(text.galley.clone()),
                egui::Shape::Vec(items) => {
                    for s in items {
                        visit(s, out);
                    }
                }
                _ => {}
            }
        }
        for clipped in shapes {
            visit(&clipped.shape, &mut galleys);
        }
        galleys
    }

    fn galley_rendered_text(galley: &egui::Galley) -> String {
        let mut out = String::new();
        for placed in &galley.rows {
            for glyph in &placed.row.glyphs {
                out.push(glyph.chr);
            }
        }
        out
    }

    #[test]
    fn max_width_icon_button_fits_short_text_and_truncates_long_text() {
        let t = crate::theme::Tokens::dark();
        let max_w = 132.0_f32;
        let run_case = |label: &str, cap: f32| -> (Rect, f32, String) {
            let ctx = egui::Context::default();
            let raw = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 80.0))),
                ..Default::default()
            };
            let mut btn_rect = Rect::NOTHING;
            let mut measured_w = 0.0_f32;
            let out = ctx.run_ui(raw, |ui| {
                btn_rect =
                    max_width_icon_button(ui, Icon::Notebook, label, cap, true, true, &t).rect;
                measured_w = max_width_icon_button_width(ui, label, cap);
            });
            let galleys = collect_galleys(&out.shapes);
            let rendered = galleys
                .first()
                .map(|g| galley_rendered_text(g))
                .unwrap_or_default();
            out.drop_without_applying_deltas();
            (btn_rect, measured_w, rendered)
        };

        let (math_rect, math_measured, math_text) = run_case("Math", max_w);
        assert!(
            (math_rect.width() - math_measured).abs() < 0.01,
            "drawn width {} != measured width {}",
            math_rect.width(),
            math_measured
        );
        assert!(
            math_rect.width() < 90.0,
            "expected short label 'Math' to shrink-to-fit below 90px, got {:.1}",
            math_rect.width()
        );
        assert_eq!(math_text, "Math");

        let (calc_rect, _, calc_text) = run_case("Calculus II", max_w);
        assert!(
            calc_rect.width() > math_rect.width() && calc_rect.width() <= max_w,
            "expected 'Calculus II' ({:.1}) wider than 'Math' ({:.1}) and <= {max_w}",
            calc_rect.width(),
            math_rect.width()
        );
        assert_eq!(calc_text, "Calculus II");

        let long = "Mathsssssssssssssssssssssssssssssssssssssssssssssssssssssssss";
        let (long_rect, long_measured, long_text) = run_case(long, max_w);
        assert!(
            (long_rect.width() - long_measured).abs() < 0.01,
            "drawn width {} != measured width {}",
            long_rect.width(),
            long_measured
        );
        assert!(
            long_rect.width() <= max_w + 0.01 && long_rect.width() > 110.0,
            "expected long label to cap near {max_w}, got {:.1}",
            long_rect.width()
        );
        assert!(
            long_text.ends_with('…'),
            "expected truncated text to end with ellipsis, got {long_text:?}"
        );

        // Hostile inputs never panic and remain bounded.
        let (empty_rect, _, empty_text) = run_case("", max_w);
        assert!((empty_rect.width() - 28.0).abs() < 0.01);
        assert!(empty_text.is_empty());

        let (neg_rect, _, _) = run_case("Math", -10.0);
        assert!((neg_rect.width() - 28.0).abs() < 0.01);

        let (nan_rect, _, nan_text) = run_case("Math", f32::NAN);
        assert!(nan_rect.width().is_finite() && nan_rect.width() >= 28.0);
        assert_eq!(nan_text, "Math");

        let (inf_rect, _, inf_text) = run_case("Math", f32::INFINITY);
        assert!(inf_rect.width().is_finite() && inf_rect.width() >= 28.0);
        assert_eq!(inf_text, "Math");
    }

    fn collect_fill_rects(shapes: &[egui::epaint::ClippedShape], fill: Color32) -> Vec<Rect> {
        fn walk(shape: &egui::Shape, fill: Color32, out: &mut Vec<Rect>) {
            match shape {
                egui::Shape::Rect(r) if r.fill == fill => out.push(r.rect),
                egui::Shape::Vec(v) => {
                    for s in v {
                        walk(s, fill, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in shapes {
            walk(&clipped.shape, fill, &mut out);
        }
        out
    }

    #[test]
    fn segmented_switch_layouts_heights_and_active_none() {
        let t = crate::theme::Tokens::light();
        let ctx = egui::Context::default();
        let opts_2 = [
            SwitchOption::new(Some(Icon::GridView), Some("Grid"), "Grid view", true),
            SwitchOption::new(Some(Icon::ListView), Some("List"), "List view", true),
        ];
        let opts_3 = [
            SwitchOption::new(None, Some("One"), "Option 1", true),
            SwitchOption::new(None, Some("Two"), "Option 2", true),
            SwitchOption::new(None, Some("Three"), "Option 3", true),
        ];
        let opts_5 = [
            SwitchOption::new(Some(Icon::LineShape), None, "Line", true),
            SwitchOption::new(Some(Icon::RectangleShape), None, "Rect", true),
            SwitchOption::new(Some(Icon::EllipseShape), None, "Ellipse", true),
            SwitchOption::new(Some(Icon::TriangleShape), None, "Triangle", true),
            SwitchOption::new(Some(Icon::ArrowShape), None, "Arrow", true),
        ];

        let mut compact_rect = Rect::NOTHING;
        let mut equal_rect = Rect::NOTHING;
        let mut five_rect = Rect::NOTHING;

        let out = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 200.0))),
                ..Default::default()
            },
            |ui| {
                compact_rect = segmented_switch(
                    ui,
                    "sw_compact_2",
                    &opts_2,
                    Some(0),
                    SwitchLayout::Compact,
                    &t,
                )
                .1
                .rect;
                equal_rect = segmented_switch(
                    ui,
                    "sw_equal_3",
                    &opts_3,
                    None,
                    SwitchLayout::EqualWidth,
                    &t,
                )
                .1
                .rect;
                five_rect = segmented_switch(
                    ui,
                    "sw_compact_5",
                    &opts_5,
                    Some(2),
                    SwitchLayout::Compact,
                    &t,
                )
                .1
                .rect;
            },
        );
        let pill_rects = collect_fill_rects(&out.shapes, t.accent_soft);
        out.drop_without_applying_deltas();

        assert!((compact_rect.height() - 28.0).abs() < 0.01);
        assert!((equal_rect.height() - 32.0).abs() < 0.01);
        assert!((five_rect.height() - 28.0).abs() < 0.01);
        assert!(equal_rect.width() >= 300.0);
        // sw_compact_2 and sw_compact_5 draw active pills; sw_equal_3 (active=None) draws none.
        assert_eq!(pill_rects.len(), 2);
        assert!(pill_rects[0].width() < compact_rect.width() * 0.65);
        assert!(pill_rects[1].width() < five_rect.width() * 0.35);
    }

    #[test]
    fn segmented_switch_clicks_disabled_handling_and_smooth_pill_slide() {
        let t = crate::theme::Tokens::light();
        let ctx = egui::Context::default();
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(360.0, 120.0));
        let options = [
            SwitchOption::new(None, Some("Copy"), "Copy", true),
            SwitchOption::new(None, Some("Disabled"), "Disabled", false),
            SwitchOption::new(None, Some("Move"), "Move", true),
        ];

        let run_frame = |time: f64, events: Vec<egui::Event>, active: Option<usize>| {
            let mut track_rect = Rect::NOTHING;
            let mut clicked = None;
            let out = ctx.run_ui(
                RawInput {
                    screen_rect: Some(screen),
                    time: Some(time),
                    events,
                    ..Default::default()
                },
                |ui| {
                    ui.set_width(300.0);
                    let (c, r) = segmented_switch(
                        ui,
                        "sw_anim_test",
                        &options,
                        active,
                        SwitchLayout::EqualWidth,
                        &t,
                    );
                    clicked = c;
                    track_rect = r.rect;
                },
            );
            let pills = collect_fill_rects(&out.shapes, t.accent_soft);
            out.drop_without_applying_deltas();
            (track_rect, clicked, pills)
        };

        // Initial settle on segment 0.
        let (track_rect, _, pills_0) = run_frame(0.0, vec![], Some(0));
        assert_eq!(pills_0.len(), 1);
        let pill_start_x = pills_0[0].left();

        // Click disabled middle segment -> must not fire.
        let mid_pt = track_rect.center();
        run_frame(
            0.05,
            vec![
                egui::Event::PointerMoved(mid_pt),
                egui::Event::PointerButton {
                    pos: mid_pt,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            Some(0),
        );
        let (_, disabled_click, _) = run_frame(
            0.06,
            vec![egui::Event::PointerButton {
                pos: mid_pt,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            Some(0),
        );
        assert_eq!(disabled_click, None);

        // Re-clicking active segment 0 still reports Some(0).
        let left_pt = Pos2::new(track_rect.left() + 30.0, track_rect.center().y);
        run_frame(
            0.10,
            vec![
                egui::Event::PointerMoved(left_pt),
                egui::Event::PointerButton {
                    pos: left_pt,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            Some(0),
        );
        let (_, re_click, _) = run_frame(
            0.11,
            vec![egui::Event::PointerButton {
                pos: left_pt,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            Some(0),
        );
        assert_eq!(re_click, Some(0));

        // Click segment 2 and switch active to Some(2), checking intermediate pill slide.
        let right_pt = Pos2::new(track_rect.right() - 30.0, track_rect.center().y);
        run_frame(
            0.20,
            vec![
                egui::Event::PointerMoved(right_pt),
                egui::Event::PointerButton {
                    pos: right_pt,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            Some(0),
        );
        let (_, third_click, _) = run_frame(
            0.21,
            vec![egui::Event::PointerButton {
                pos: right_pt,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
            Some(0),
        );
        assert_eq!(third_click, Some(2));

        // Frame switching active to 2 (mid-animation ~0.04s later) then settled.
        run_frame(0.22, vec![], Some(2));
        let (_, _, pills_mid) = run_frame(0.26, vec![], Some(2));
        let (_, _, pills_end) = run_frame(0.50, vec![], Some(2));
        assert_eq!(pills_mid.len(), 1);
        assert_eq!(pills_end.len(), 1);
        assert!(
            pills_mid[0].left() > pill_start_x + 10.0 && pills_mid[0].left() < pills_end[0].left(),
            "expected smooth mid-slide between {pill_start_x:.1} and {:.1}, got {:.1}",
            pills_end[0].left(),
            pills_mid[0].left()
        );
    }

    #[test]
    fn toggle_switch_and_row_render_and_toggle_on_click() {
        for theme in [
            papercraft_doc::UiTheme::Light,
            papercraft_doc::UiTheme::Dark,
        ] {
            let ctx = egui::Context::default();
            let t = crate::theme::Tokens::for_theme(theme);
            let mut value = true;
            let mut switch_rect = Rect::NOTHING;

            let out = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(360.0, 120.0))),
                    time: Some(0.05),
                    ..Default::default()
                },
                |ui| {
                    switch_rect = toggle_switch(ui, ("ts_test", theme), &mut value, true, &t).rect;
                },
            );
            assert!(!out.shapes.is_empty());
            out.drop_without_applying_deltas();

            let switch_pt = switch_rect.center();
            for (pressed, time) in [(true, 0.08), (false, 0.12)] {
                let click_out = ctx.run_ui(
                    RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(360.0, 120.0))),
                        time: Some(time),
                        events: vec![
                            egui::Event::PointerMoved(switch_pt),
                            egui::Event::PointerButton {
                                pos: switch_pt,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                        ..Default::default()
                    },
                    |ui| {
                        let _ = toggle_switch(ui, ("ts_test", theme), &mut value, true, &t);
                    },
                );
                click_out.drop_without_applying_deltas();
            }
            assert!(!value, "expected toggle_switch click to turn switch off");

            let mut row_rect = Rect::NOTHING;
            let warm_row = ctx.run_ui(
                RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(360.0, 120.0))),
                    time: Some(0.16),
                    ..Default::default()
                },
                |ui| {
                    row_rect = toggle_switch_row(
                        ui,
                        ("ts_row_test", theme),
                        "Create Desktop shortcut automatically",
                        "Tooltip",
                        &mut value,
                        &t,
                    )
                    .rect;
                },
            );
            warm_row.drop_without_applying_deltas();

            let row_pt = Pos2::new(row_rect.right() - 10.0, row_rect.center().y);
            for (pressed, time) in [(true, 0.20), (false, 0.24)] {
                let click_out = ctx.run_ui(
                    RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(360.0, 120.0))),
                        time: Some(time),
                        events: vec![
                            egui::Event::PointerMoved(row_pt),
                            egui::Event::PointerButton {
                                pos: row_pt,
                                button: egui::PointerButton::Primary,
                                pressed,
                                modifiers: egui::Modifiers::NONE,
                            },
                        ],
                        ..Default::default()
                    },
                    |ui| {
                        let _ = toggle_switch_row(
                            ui,
                            ("ts_row_test", theme),
                            "Create Desktop shortcut automatically",
                            "Tooltip",
                            &mut value,
                            &t,
                        );
                    },
                );
                click_out.drop_without_applying_deltas();
            }
            assert!(
                value,
                "expected toggle_switch_row click to turn switch back on"
            );
        }
    }

    #[test]
    fn sidebar_edge_button_matches_measured_width_truncates_and_handles_hostile_inputs() {
        for (edge, theme, open) in [
            (SidebarEdge::Left, papercraft_doc::UiTheme::Dark, true),
            (SidebarEdge::Left, papercraft_doc::UiTheme::Light, false),
            (SidebarEdge::Right, papercraft_doc::UiTheme::Dark, false),
            (SidebarEdge::Right, papercraft_doc::UiTheme::Light, true),
        ] {
            let t = crate::theme::Tokens::for_theme(theme);
            let max_w = Some(148.0_f32);
            let run_case = |label: &str, cap: Option<f32>| -> (Rect, f32, String) {
                let ctx = egui::Context::default();
                let raw = RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 80.0))),
                    ..Default::default()
                };
                let mut btn_rect = Rect::NOTHING;
                let mut measured_w = 0.0_f32;
                let out = ctx.run_ui(raw, |ui| {
                    btn_rect = sidebar_edge_button(
                        ui,
                        SidebarEdgeButtonSpec {
                            edge,
                            icon: if edge == SidebarEdge::Left {
                                Icon::Notebook
                            } else {
                                Icon::Settings
                            },
                            label,
                            max_width: cap,
                            open,
                            enabled: true,
                        },
                        &t,
                    )
                    .rect;
                    measured_w = sidebar_edge_button_width(ui, label, cap);
                });
                let galleys = collect_galleys(&out.shapes);
                let rendered = galleys
                    .first()
                    .map(|g| galley_rendered_text(g))
                    .unwrap_or_default();
                out.drop_without_applying_deltas();
                (btn_rect, measured_w, rendered)
            };

            let (math_rect, math_measured, math_text) = run_case("Math", max_w);
            assert!(
                (math_rect.width() - math_measured).abs() < 0.01,
                "drawn width {} != measured width {}",
                math_rect.width(),
                math_measured
            );
            assert!(
                math_rect.width() < 90.0,
                "expected short label 'Math' to stay below 90px, got {:.1}",
                math_rect.width()
            );
            assert_eq!(math_text, "Math");

            let (settings_rect, settings_measured, settings_text) = run_case("Settings", None);
            assert!((settings_rect.width() - settings_measured).abs() < 0.01);
            assert_eq!(settings_text, "Settings");

            let long = "Differential Calculus & Linear Algebra Study Notes";
            let (long_rect, long_measured, long_text) = run_case(long, max_w);
            assert!((long_rect.width() - long_measured).abs() < 0.01);
            assert!(long_rect.width() <= 148.01 && long_rect.width() > 120.0);
            assert!(
                long_text.ends_with('…'),
                "expected truncated text to end with ellipsis, got {long_text:?}"
            );

            // Empty label and hostile max_width values never panic and remain bounded.
            let (empty_rect, _, empty_text) = run_case("", max_w);
            assert!((empty_rect.width() - 47.0).abs() < 0.01);
            assert!(empty_text.is_empty());

            let (neg_rect, _, neg_text) = run_case("Math", Some(-10.0));
            assert!((neg_rect.width() - 36.0).abs() < 0.01);
            assert!(neg_text.is_empty());

            let (nan_rect, _, nan_text) = run_case("Math", Some(f32::NAN));
            assert!(nan_rect.width().is_finite() && nan_rect.width() >= 36.0);
            assert_eq!(nan_text, "Math");

            let (inf_rect, _, inf_text) = run_case("Math", Some(f32::INFINITY));
            assert!(inf_rect.width().is_finite() && inf_rect.width() >= 36.0);
            assert_eq!(inf_text, "Math");
        }
    }

    #[test]
    fn toolbar_sized_controls_scale_up_and_omit_text_when_unlabeled() {
        let ctx = egui::Context::default();
        let t = Tokens::light();
        let mut btn_rect = Rect::NOTHING;
        let mut switch_rect = Rect::NOTHING;
        let mut edge_left = Rect::NOTHING;
        let mut edge_right = Rect::NOTHING;
        let mut edge_left_w = 0.0_f32;
        let out = ctx.run_ui(
            RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(640.0, 120.0))),
                ..Default::default()
            },
            |ui| {
                ui.horizontal(|ui| {
                    edge_left = sidebar_edge_button_sized(
                        ui,
                        SidebarEdgeButtonSpec {
                            edge: SidebarEdge::Left,
                            icon: Icon::Notebook,
                            label: "",
                            max_width: None,
                            open: false,
                            enabled: true,
                        },
                        SidebarEdgeSize::Toolbar,
                        &t,
                    )
                    .rect;
                    edge_left_w =
                        sidebar_edge_button_sized_width(ui, "", None, SidebarEdgeSize::Toolbar);
                    btn_rect = toolbar_icon_button(ui, Icon::Library, None, false, true, &t).rect;
                    let opts = [
                        SwitchOption::new(Some(Icon::Select), None, "Select", true),
                        SwitchOption::new(Some(Icon::FountainPen), None, "Pen", true),
                    ];
                    switch_rect = segmented_switch(
                        ui,
                        "tb_sized_sw",
                        &opts,
                        Some(0),
                        SwitchLayout::Toolbar,
                        &t,
                    )
                    .1
                    .rect;
                    edge_right = sidebar_edge_button_sized(
                        ui,
                        SidebarEdgeButtonSpec {
                            edge: SidebarEdge::Right,
                            icon: Icon::Settings,
                            label: "",
                            max_width: None,
                            open: false,
                            enabled: true,
                        },
                        SidebarEdgeSize::Toolbar,
                        &t,
                    )
                    .rect;
                });
            },
        );
        assert!(collect_galleys(&out.shapes).is_empty());
        out.drop_without_applying_deltas();

        assert!((btn_rect.height() - TOOLBAR_CONTROL_HEIGHT).abs() < 0.01);
        assert!((btn_rect.width() - TOOLBAR_CONTROL_HEIGHT).abs() < 0.01);
        assert!((switch_rect.height() - TOOLBAR_CONTROL_HEIGHT).abs() < 0.01);
        assert!((edge_left.height() - TOOLBAR_CONTROL_HEIGHT).abs() < 0.01);
        assert!((edge_right.height() - TOOLBAR_CONTROL_HEIGHT).abs() < 0.01);
        assert!((edge_left.width() - edge_left_w).abs() < 0.01);
        assert!((edge_left.width() - 58.0).abs() < 0.01);
    }
}
