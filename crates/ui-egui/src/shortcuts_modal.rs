//! Minimalist bottom-right shortcuts trigger button and single-page Keyboard Shortcuts modal.

use egui::{CornerRadius, FontId, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2};
use papercraft_doc::PenKind;
use papercraft_engine::ToolKind;

use crate::icons::{Icon, paint_icon};
use crate::modal::{ActiveModal, secondary_action_button};
use crate::theme::Tokens;
use crate::{AppScreen, PaperCraftApp};

const TRIGGER_BTN_SIZE: f32 = 28.0;
const TRIGGER_ICON_SIZE: f32 = 18.0;
const TRIGGER_MARGIN_RIGHT: f32 = 14.0;
const TRIGGER_MARGIN_BOTTOM: f32 = 12.0;
const MIN_WORKSPACE_DIM: f32 = 96.0;

const ROW_HEIGHT: f32 = 25.0;
const ROW_GAP: f32 = 3.0;
const KEYCAP_HEIGHT: f32 = 20.0;
const KEYCAP_MIN_WIDTH: f32 = 20.0;
const KEYCAP_PAD_X: f32 = 5.5;
const KEYCAP_GAP: f32 = 4.0;

#[cfg(target_os = "macos")]
const CMD_KEY: &str = "Cmd";
#[cfg(not(target_os = "macos"))]
const CMD_KEY: &str = "Ctrl";

#[derive(Clone, Copy)]
struct ShortcutEntry {
    icon: Icon,
    label: &'static str,
    keys: &'static [&'static str],
    tool: Option<ToolKind>,
}

const TOOL_SHORTCUTS: &[ShortcutEntry] = &[
    ShortcutEntry {
        icon: Icon::Select,
        label: "Select",
        keys: &["V"],
        tool: Some(ToolKind::Select),
    },
    ShortcutEntry {
        icon: Icon::FountainPen,
        label: "Fountain Pen",
        keys: &["1"],
        tool: Some(ToolKind::Pen(PenKind::FountainPen)),
    },
    ShortcutEntry {
        icon: Icon::BallPen,
        label: "Ballpoint Pen",
        keys: &["2"],
        tool: Some(ToolKind::Pen(PenKind::BallPen)),
    },
    ShortcutEntry {
        icon: Icon::BrushPen,
        label: "Brush Pen",
        keys: &["3"],
        tool: Some(ToolKind::Pen(PenKind::BrushPen)),
    },
    ShortcutEntry {
        icon: Icon::Highlighter,
        label: "Highlighter",
        keys: &["4"],
        tool: Some(ToolKind::Pen(PenKind::Highlighter)),
    },
    ShortcutEntry {
        icon: Icon::Eraser,
        label: "Eraser",
        keys: &["5"],
        tool: Some(ToolKind::Eraser),
    },
    ShortcutEntry {
        icon: Icon::Pencil,
        label: "Pencil",
        keys: &["6"],
        tool: Some(ToolKind::Pen(PenKind::Pencil)),
    },
    ShortcutEntry {
        icon: Icon::LaserPointer,
        label: "Laser Pointer",
        keys: &["7"],
        tool: Some(ToolKind::LaserPointer),
    },
    ShortcutEntry {
        icon: Icon::LineShape,
        label: "Nib Size",
        keys: &["[", "]"],
        tool: None,
    },
];

const EDITING_SHORTCUTS: &[ShortcutEntry] = &[
    ShortcutEntry {
        icon: Icon::Undo,
        label: "Undo",
        keys: &[CMD_KEY, "Z"],
        tool: None,
    },
    ShortcutEntry {
        icon: Icon::Redo,
        label: "Redo",
        keys: &[CMD_KEY, "Y"],
        tool: None,
    },
    ShortcutEntry {
        icon: Icon::Duplicate,
        label: "Copy",
        keys: &[CMD_KEY, "C"],
        tool: None,
    },
    ShortcutEntry {
        icon: Icon::StrokeEraser,
        label: "Cut",
        keys: &[CMD_KEY, "X"],
        tool: None,
    },
    ShortcutEntry {
        icon: Icon::Paste,
        label: "Paste",
        keys: &[CMD_KEY, "V"],
        tool: None,
    },
    ShortcutEntry {
        icon: Icon::Duplicate,
        label: "Duplicate",
        keys: &[CMD_KEY, "D"],
        tool: None,
    },
    ShortcutEntry {
        icon: Icon::Trash,
        label: "Delete Selection",
        keys: &["Del"],
        tool: None,
    },
    ShortcutEntry {
        icon: Icon::Bold,
        label: "Bold / Italic / Underline",
        keys: &[CMD_KEY, "B / I / U"],
        tool: None,
    },
    ShortcutEntry {
        icon: Icon::LineShape,
        label: "Straight Line",
        keys: &["Shift", "Drag"],
        tool: None,
    },
    ShortcutEntry {
        icon: Icon::ShapeTool,
        label: "Snap Shape",
        keys: &["Draw & Hold"],
        tool: None,
    },
];

const NAVIGATION_SHORTCUTS: &[ShortcutEntry] = &[
    ShortcutEntry {
        icon: Icon::GripHorizontal,
        label: "Pan Canvas",
        keys: &["Space", "Drag"],
        tool: None,
    },
    ShortcutEntry {
        icon: Icon::ZoomIn,
        label: "Zoom Canvas",
        keys: &["Ctrl", "Wheel"],
        tool: None,
    },
    ShortcutEntry {
        icon: Icon::PrevPage,
        label: "Previous / Next Page",
        keys: &["PgUp", "PgDn"],
        tool: None,
    },
    ShortcutEntry {
        icon: Icon::Paper,
        label: "Pages Sidebar",
        keys: &["P"],
        tool: None,
    },
    ShortcutEntry {
        icon: Icon::Settings,
        label: "Settings Drawer",
        keys: &["S"],
        tool: None,
    },
    ShortcutEntry {
        icon: Icon::Home,
        label: "Notebooks Library",
        keys: &["H"],
        tool: None,
    },
    ShortcutEntry {
        icon: Icon::Plus,
        label: "New Notebook",
        keys: &[CMD_KEY, "N"],
        tool: None,
    },
    ShortcutEntry {
        icon: Icon::FolderOpen,
        label: "Open Notebook",
        keys: &[CMD_KEY, "O"],
        tool: None,
    },
    ShortcutEntry {
        icon: Icon::Close,
        label: "Close Tab",
        keys: &[CMD_KEY, "W"],
        tool: None,
    },
    ShortcutEntry {
        icon: Icon::FitPage,
        label: "Interface Scale",
        keys: &[CMD_KEY, "+ / - / 0"],
        tool: None,
    },
    ShortcutEntry {
        icon: Icon::Help,
        label: "Keyboard Shortcuts",
        keys: &["?"],
        tool: None,
    },
    ShortcutEntry {
        icon: Icon::Close,
        label: "Close / Exit Mode",
        keys: &["Esc"],
        tool: None,
    },
];

/// Renders the minimalist bottom-right `?` button that opens the Keyboard Shortcuts modal.
pub(crate) fn show_trigger_button(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    workspace_rect: Rect,
    t: &Tokens,
) {
    if !workspace_rect.is_positive()
        || workspace_rect.width() < MIN_WORKSPACE_DIM
        || workspace_rect.height() < MIN_WORKSPACE_DIM
    {
        return;
    }
    let top_left = pos2(
        workspace_rect.right() - TRIGGER_MARGIN_RIGHT - TRIGGER_BTN_SIZE,
        workspace_rect.bottom() - TRIGGER_MARGIN_BOTTOM - TRIGGER_BTN_SIZE,
    );
    let modal_open = app.view.active_modal == Some(ActiveModal::KeyboardShortcuts);
    let interactive = app.view.active_modal.is_none() && !app.view.motion.screen.is_exiting();
    let area_id = ui.make_persistent_id("papercraft_shortcuts_trigger");

    let clicked = egui::Area::new(area_id)
        .order(egui::Order::Middle)
        .fixed_pos(top_left)
        .default_size(Vec2::splat(TRIGGER_BTN_SIZE))
        .constrain_to(workspace_rect.shrink(4.0))
        .fade_in(false)
        .interactable(interactive)
        .show(ui.ctx(), |ui| {
            let (rect, resp) = ui.allocate_exact_size(
                Vec2::splat(TRIGGER_BTN_SIZE),
                if interactive {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            if ui.is_rect_visible(rect) {
                let hover_t = crate::motion::hover_factor(
                    ui.ctx(),
                    resp.id.with("hov"),
                    resp.hovered() && interactive && !modal_open,
                );
                let active_t =
                    crate::motion::hover_factor(ui.ctx(), resp.id.with("act"), modal_open);
                let base_fill = t.toolbar_bg.gamma_multiply(0.90);
                let hovered_fill =
                    crate::motion::lerp_color(base_fill, t.border.gamma_multiply(0.55), hover_t);
                let fill = crate::motion::lerp_color(hovered_fill, t.accent_soft, active_t);
                let fg = crate::motion::lerp_color(
                    crate::motion::lerp_color(t.text_muted, t.text, hover_t),
                    t.accent,
                    active_t,
                );
                let radius = CornerRadius::same((TRIGGER_BTN_SIZE * 0.5).round() as u8);
                ui.painter().rect_filled(rect, radius, fill);
                let icon_rect =
                    Rect::from_center_size(rect.center(), Vec2::splat(TRIGGER_ICON_SIZE));
                paint_icon(ui.painter(), icon_rect, Icon::Help, fg);
            }
            let resp = if interactive {
                resp.on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text("Keyboard Shortcuts (?)")
            } else {
                resp
            };
            interactive && resp.clicked()
        })
        .inner;

    if clicked {
        crate::modal::open(app, ActiveModal::KeyboardShortcuts);
    }
}

/// Renders the single-page Keyboard Shortcuts modal body. Returns `true` when the modal should close.
pub(crate) fn render_body(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) -> bool {
    let active_tool = active_editor_tool(app);
    let avail_w = ui.available_width().max(160.0);

    if avail_w >= 680.0 {
        let gap = 22.0;
        let col_w = ((avail_w - gap * 2.0) / 3.0).max(160.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(gap, 0.0);
            draw_section_column(ui, "TOOLS", TOOL_SHORTCUTS, col_w, active_tool, t);
            draw_section_column(ui, "EDITING", EDITING_SHORTCUTS, col_w, active_tool, t);
            draw_section_column(
                ui,
                "NAVIGATION",
                NAVIGATION_SHORTCUTS,
                col_w,
                active_tool,
                t,
            );
        });
    } else if avail_w >= 430.0 {
        let gap = 18.0;
        let col_w = ((avail_w - gap) * 0.5).max(160.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(gap, 0.0);
            draw_section_column(ui, "TOOLS", TOOL_SHORTCUTS, col_w, active_tool, t);
            draw_section_column(ui, "EDITING", EDITING_SHORTCUTS, col_w, active_tool, t);
        });
        ui.add_space(12.0);
        draw_two_column_section(
            ui,
            "NAVIGATION",
            NAVIGATION_SHORTCUTS,
            col_w,
            gap,
            active_tool,
            t,
        );
    } else {
        draw_section_column(ui, "TOOLS", TOOL_SHORTCUTS, avail_w, active_tool, t);
        ui.add_space(12.0);
        draw_section_column(ui, "EDITING", EDITING_SHORTCUTS, avail_w, active_tool, t);
        ui.add_space(12.0);
        draw_section_column(
            ui,
            "NAVIGATION",
            NAVIGATION_SHORTCUTS,
            avail_w,
            active_tool,
            t,
        );
    }

    false
}

/// Renders the Keyboard Shortcuts modal footer (`Close`). Returns `true` when closed.
pub(crate) fn render_footer(_app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) -> bool {
    let mut should_close = false;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new("Press ? anytime to toggle shortcuts")
                .size(11.0)
                .color(t.text_muted),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if secondary_action_button(ui, "Close", true, t).clicked() {
                should_close = true;
            }
        });
    });
    should_close
}

fn active_editor_tool(app: &PaperCraftApp) -> Option<ToolKind> {
    if app.view.screen != AppScreen::Editor
        || app.view.text_insert_mode
        || app.view.text_editing
        || app.view.shape_insert_mode
    {
        return None;
    }
    Some(app.session.active_tool)
}

fn draw_section_column(
    ui: &mut egui::Ui,
    heading: &str,
    entries: &[ShortcutEntry],
    col_w: f32,
    active_tool: Option<ToolKind>,
    t: &Tokens,
) {
    ui.allocate_ui_with_layout(
        Vec2::new(col_w, 0.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_min_width(col_w);
            ui.set_max_width(col_w);
            ui.spacing_mut().item_spacing = Vec2::new(0.0, ROW_GAP);
            draw_section_heading(ui, heading, t);
            ui.add_space(3.0);
            for entry in entries {
                let highlighted = active_tool.is_some() && entry.tool == active_tool;
                draw_shortcut_row(ui, entry, col_w, highlighted, t);
            }
        },
    );
}

fn draw_two_column_section(
    ui: &mut egui::Ui,
    heading: &str,
    entries: &[ShortcutEntry],
    col_w: f32,
    gap: f32,
    active_tool: Option<ToolKind>,
    t: &Tokens,
) {
    draw_section_heading(ui, heading, t);
    ui.add_space(5.0);
    let split = entries.len().div_ceil(2);
    let (left, right) = entries.split_at(split);
    ui.horizontal_top(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(gap, 0.0);
        draw_entry_stack(ui, left, col_w, active_tool, t);
        draw_entry_stack(ui, right, col_w, active_tool, t);
    });
}

fn draw_entry_stack(
    ui: &mut egui::Ui,
    entries: &[ShortcutEntry],
    col_w: f32,
    active_tool: Option<ToolKind>,
    t: &Tokens,
) {
    ui.allocate_ui_with_layout(
        Vec2::new(col_w, 0.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_min_width(col_w);
            ui.set_max_width(col_w);
            ui.spacing_mut().item_spacing = Vec2::new(0.0, ROW_GAP);
            for entry in entries {
                let highlighted = active_tool.is_some() && entry.tool == active_tool;
                draw_shortcut_row(ui, entry, col_w, highlighted, t);
            }
        },
    );
}

fn draw_section_heading(ui: &mut egui::Ui, heading: &str, t: &Tokens) {
    ui.label(
        RichText::new(heading)
            .size(10.5)
            .strong()
            .color(t.text_muted),
    );
}

fn draw_shortcut_row(
    ui: &mut egui::Ui,
    entry: &ShortcutEntry,
    col_w: f32,
    highlighted: bool,
    t: &Tokens,
) {
    let (row_rect, _) = ui.allocate_exact_size(Vec2::new(col_w, ROW_HEIGHT), Sense::hover());
    if !ui.is_rect_visible(row_rect) {
        return;
    }

    let font = FontId::proportional(11.0);
    let key_fg = if highlighted { t.on_accent } else { t.text };
    let key_galleys: Vec<(std::sync::Arc<egui::Galley>, f32)> = entry
        .keys
        .iter()
        .map(|k| {
            let galley = ui
                .painter()
                .layout_no_wrap((*k).to_string(), font.clone(), key_fg);
            let width = (galley.size().x + KEYCAP_PAD_X * 2.0).max(KEYCAP_MIN_WIDTH);
            (galley, width)
        })
        .collect();

    let keys_total_w = key_galleys.iter().map(|(_, w)| *w).sum::<f32>()
        + key_galleys.len().saturating_sub(1) as f32 * KEYCAP_GAP;

    let icon_dim = 14.5_f32;
    let icon_rect = Rect::from_center_size(
        pos2(row_rect.left() + icon_dim * 0.5, row_rect.center().y),
        Vec2::splat(icon_dim),
    );
    let icon_color = if highlighted { t.accent } else { t.text_muted };
    paint_icon(ui.painter(), icon_rect, entry.icon, icon_color);

    let label_left = icon_rect.right() + 7.5;
    let label_right = (row_rect.right() - keys_total_w - 8.0).max(label_left + 16.0);
    let label_rect = Rect::from_min_max(
        pos2(label_left, row_rect.top()),
        pos2(label_right, row_rect.bottom()),
    );
    let label_color = if highlighted { t.accent } else { t.text };
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(label_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
        |ui| {
            ui.add(
                egui::Label::new(RichText::new(entry.label).size(12.0).color(label_color))
                    .truncate(),
            );
        },
    );

    let p = ui.painter();
    let cap_fill = if highlighted { t.accent } else { t.toolbar_bg };
    let cap_stroke = Stroke::new(1.0, if highlighted { t.accent } else { t.border });
    let mut cursor_right = row_rect.right();
    for (galley, cap_w) in key_galleys.into_iter().rev() {
        let cap_left = cursor_right - cap_w;
        let cap_rect = Rect::from_center_size(
            pos2((cap_left + cursor_right) * 0.5, row_rect.center().y),
            Vec2::new(cap_w, KEYCAP_HEIGHT),
        );
        p.rect_filled(cap_rect, CornerRadius::same(4), cap_fill);
        p.rect_stroke(
            cap_rect,
            CornerRadius::same(4),
            cap_stroke,
            StrokeKind::Inside,
        );
        let text_pos = pos2(
            cap_rect.center().x - galley.size().x * 0.5,
            cap_rect.center().y - galley.size().y * 0.5,
        );
        p.galley(text_pos, galley, key_fg);
        cursor_right = cap_left - KEYCAP_GAP;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::{AppScreen, modal, motion};
    use egui::{Pos2, RawInput, Shape};

    fn collect_texts(shapes: &[egui::epaint::ClippedShape]) -> Vec<String> {
        fn visit(shape: &Shape, texts: &mut Vec<String>) {
            match shape {
                Shape::Text(text) => {
                    texts.push(text.galley.text().to_string());
                }
                Shape::Vec(items) => {
                    for item in items {
                        visit(item, texts);
                    }
                }
                _ => {}
            }
        }
        let mut texts = Vec::new();
        for clipped in shapes {
            visit(&clipped.shape, &mut texts);
        }
        texts
    }

    fn run_app_step(
        ctx: &egui::Context,
        app: &mut PaperCraftApp,
        viewport: Vec2,
        time: f64,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        let raw = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
            time: Some(time),
            predicted_dt: 0.016,
            events,
            ..Default::default()
        };
        ctx.run_ui(raw, |ui| app.show_ui(ui))
    }

    fn trigger_button_center(workspace_rect: Rect) -> Pos2 {
        pos2(
            workspace_rect.right() - TRIGGER_MARGIN_RIGHT - TRIGGER_BTN_SIZE * 0.5,
            workspace_rect.bottom() - TRIGGER_MARGIN_BOTTOM - TRIGGER_BTN_SIZE * 0.5,
        )
    }

    #[test]
    fn bottom_right_button_opens_single_page_shortcuts_modal_on_home_and_editor() {
        for screen in [AppScreen::Home, AppScreen::Editor] {
            let ctx = egui::Context::default();
            let mut app = PaperCraftApp::default();
            app.view.screen = screen;
            app.view.motion.screen = motion::ScreenTransition::new(screen);
            let viewport = egui::vec2(1280.0, 800.0);

            // Two warmup frames register the floating Area's layout & hit-test rect in egui.
            for frame in 0..2 {
                let out = run_app_step(
                    &ctx,
                    &mut app,
                    viewport,
                    0.02 + frame as f64 * 0.016,
                    Vec::new(),
                );
                out.drop_without_applying_deltas();
            }
            let ws_rect = match screen {
                AppScreen::Home => app.view.last_home_rect,
                AppScreen::Editor => app.view.last_canvas_rect,
            }
            .expect("workspace rect");
            let btn_pos = trigger_button_center(ws_rect);
            let strokes_before = app.session.doc.strokes.len();

            // Click the minimalist bottom-right '?' button.
            let out_down = run_app_step(
                &ctx,
                &mut app,
                viewport,
                0.06,
                vec![
                    egui::Event::PointerMoved(btn_pos),
                    egui::Event::PointerButton {
                        pos: btn_pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
            out_down.drop_without_applying_deltas();

            let out_up = run_app_step(
                &ctx,
                &mut app,
                viewport,
                0.08,
                vec![egui::Event::PointerButton {
                    pos: btn_pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            out_up.drop_without_applying_deltas();

            assert_eq!(
                app.view.active_modal,
                Some(ActiveModal::KeyboardShortcuts),
                "expected clicking bottom-right button on {screen:?} to open shortcuts modal"
            );
            assert_eq!(
                app.session.doc.strokes.len(),
                strokes_before,
                "clicking shortcuts button must not draw ink on canvas"
            );

            // Settle modal reveal and check single-page sections and keycaps without tabs.
            let mut last_texts = Vec::new();
            for step in 0..10 {
                let out = run_app_step(
                    &ctx,
                    &mut app,
                    viewport,
                    0.08 + step as f64 * 0.03,
                    Vec::new(),
                );
                last_texts = collect_texts(&out.shapes);
                out.drop_without_applying_deltas();
            }
            assert!(last_texts.iter().any(|t| t == "Keyboard Shortcuts"));
            assert!(last_texts.iter().any(|t| t == "TOOLS"));
            assert!(last_texts.iter().any(|t| t == "EDITING"));
            assert!(last_texts.iter().any(|t| t == "NAVIGATION"));
            for entry in TOOL_SHORTCUTS
                .iter()
                .chain(EDITING_SHORTCUTS.iter())
                .chain(NAVIGATION_SHORTCUTS.iter())
            {
                assert!(
                    last_texts.iter().any(|t| t == entry.label),
                    "expected untruncated label '{}' in shortcuts modal: {last_texts:?}",
                    entry.label
                );
            }
            assert!(last_texts.iter().any(|t| t == "V"));
            assert!(last_texts.iter().any(|t| t == "Close"));
            assert!(!last_texts.iter().any(|t| t == "Essential"));
            assert!(!last_texts.iter().any(|t| t == "Details & Cover"));

            // Pressing Escape closes the modal cleanly.
            let out_esc = run_app_step(
                &ctx,
                &mut app,
                viewport,
                0.50,
                vec![egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
            out_esc.drop_without_applying_deltas();
            assert_eq!(app.view.active_modal, None);
        }
    }

    #[test]
    fn question_mark_shortcut_toggles_modal_and_trigger_respects_open_settings_drawer() {
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        app.view.screen = AppScreen::Home;
        app.view.motion.screen = motion::ScreenTransition::new(AppScreen::Home);
        app.view.inspector_open = true;
        let viewport = egui::vec2(1280.0, 800.0);

        let out_init = run_app_step(&ctx, &mut app, viewport, 0.02, Vec::new());
        out_init.drop_without_applying_deltas();
        let home_rect = app.view.last_home_rect.expect("home rect");
        assert!(
            home_rect.right() < 1280.0 - 300.0,
            "workspace rect should shrink left of open Settings drawer: {home_rect:?}"
        );

        // Pressing '?' opens the Keyboard Shortcuts modal.
        let out_open = run_app_step(
            &ctx,
            &mut app,
            viewport,
            0.04,
            vec![egui::Event::Text("?".to_string())],
        );
        out_open.drop_without_applying_deltas();
        assert_eq!(app.view.active_modal, Some(ActiveModal::KeyboardShortcuts));

        // Pressing '?' again closes it.
        let out_close = run_app_step(
            &ctx,
            &mut app,
            viewport,
            0.06,
            vec![egui::Event::Text("?".to_string())],
        );
        out_close.drop_without_applying_deltas();
        assert_eq!(app.view.active_modal, None);

        // Compact viewport + Dark theme renders cleanly without panicking or clipping.
        let _ = app
            .session
            .execute("theme.set", &serde_json::json!({ "theme": "dark" }));
        modal::open(&mut app, ActiveModal::KeyboardShortcuts);
        for narrow_w in [540.0, 360.0] {
            let compact = egui::vec2(narrow_w, 420.0);
            for i in 0..4 {
                let out = run_app_step(&ctx, &mut app, compact, 0.1 + i as f64 * 0.03, Vec::new());
                let texts = collect_texts(&out.shapes);
                assert!(texts.iter().any(|t| t == "Keyboard Shortcuts"));
                out.drop_without_applying_deltas();
            }
        }
    }
}
