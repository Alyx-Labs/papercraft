//! Strip of open notebook tabs shown above the editor toolbar and the Home header.
//!
//! The strip keeps a Home tab pinned to the left edge, one tab per open notebook
//! (cover swatch, title, and close button), and a New Notebook button at the right
//! edge. Open tabs are view-only state: switching and closing go through the same
//! notebook commands used everywhere else in the app.

use std::sync::Arc;

use egui::{Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, Vec2, pos2};
use papercraft_doc::Document;

use crate::icons::{Icon, paint_icon};
use crate::theme::Tokens;
use crate::{AppScreen, PaperCraftApp};

/// Total height of the tab strip panel.
pub(crate) const TAB_STRIP_HEIGHT: f32 = 40.0;
const TAB_TOP_GAP: f32 = 6.0;
const TAB_HEIGHT: f32 = TAB_STRIP_HEIGHT - TAB_TOP_GAP;
const HOME_TAB_WIDTH: f32 = 46.0;
const TAB_MAX_WIDTH: f32 = 224.0;
const TAB_MIN_WIDTH: f32 = 56.0;
const TAB_CLOSE_MIN_WIDTH: f32 = 84.0;
const TAB_GAP: f32 = 2.0;
const TAB_RADIUS: u8 = 10;
const TAB_PAD_X: f32 = 12.0;
const TAB_COVER_SIZE: Vec2 = Vec2::new(12.0, 16.0);
const TAB_COVER_GAP: f32 = 8.0;
const TAB_CLOSE_SIZE: f32 = 20.0;
const TAB_CLOSE_ICON_DIM: f32 = 14.0;
const TAB_FONT_SIZE: f32 = 13.0;
const TAB_ICON_DIM: f32 = 18.0;
const NEW_TAB_SIZE: f32 = 30.0;
const SEPARATOR_HALF_H: f32 = 9.0;
/// Upper bound on simultaneously open notebook tabs.
pub(crate) const MAX_OPEN_TABS: usize = 32;

/// One open notebook tab, keyed by notebook id with a cached notebook slot index.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenTab {
    pub id: String,
    pub index: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TabAction {
    Home,
    Activate(usize),
    Close(usize),
    NewNotebook,
}

/// Rendered strip geometry, used to merge the active tab into the panel below it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct TabStripOutput {
    pub rect: Rect,
    pub active_span: Option<(f32, f32)>,
}

impl Default for TabStripOutput {
    fn default() -> Self {
        Self {
            rect: Rect::NOTHING,
            active_span: None,
        }
    }
}

fn resolve_tab(notebooks: &[Arc<Document>], tab: &OpenTab) -> Option<usize> {
    if tab.id.is_empty() {
        return notebooks
            .get(tab.index)
            .filter(|doc| doc.id.is_empty())
            .map(|_| tab.index);
    }
    if notebooks.get(tab.index).is_some_and(|doc| doc.id == tab.id) {
        return Some(tab.index);
    }
    notebooks.iter().position(|doc| doc.id == tab.id)
}

/// Re-resolves open tabs against the current notebook list, drops tabs whose notebook
/// no longer exists, and makes sure the notebook open in the editor has a tab.
pub(crate) fn sync_open_tabs(app: &mut PaperCraftApp) {
    let notebooks = &app.session.notebooks;
    let mut seen: Vec<usize> = Vec::with_capacity(app.view.open_tabs.len() + 1);
    app.view
        .open_tabs
        .retain_mut(|tab| match resolve_tab(notebooks, tab) {
            Some(index) if !seen.contains(&index) => {
                tab.index = index;
                seen.push(index);
                true
            }
            _ => false,
        });

    let active = app.session.active_notebook;
    if app.view.screen != AppScreen::Editor || active >= app.session.notebooks.len() {
        return;
    }
    if !seen.contains(&active) {
        app.view.open_tabs.push(OpenTab {
            id: app.session.notebooks[active].id.clone(),
            index: active,
        });
    }
    while app.view.open_tabs.len() > MAX_OPEN_TABS {
        match app
            .view
            .open_tabs
            .iter()
            .position(|tab| tab.index != active)
        {
            Some(oldest) => {
                app.view.open_tabs.remove(oldest);
            }
            None => break,
        }
    }
}

/// Closes the tab at `pos`; closing the active tab switches to its neighbor, or
/// returns Home when no other tabs remain open.
pub(crate) fn close_tab(app: &mut PaperCraftApp, pos: usize) {
    if pos >= app.view.open_tabs.len() {
        return;
    }
    let closing = app.view.open_tabs.remove(pos);
    let was_active =
        app.view.screen == AppScreen::Editor && closing.index == app.session.active_notebook;
    if !was_active {
        return;
    }
    if app.view.open_tabs.is_empty() {
        crate::return_to_home(app);
        return;
    }
    let next_pos = pos.min(app.view.open_tabs.len() - 1);
    let next_index = app.view.open_tabs[next_pos].index;
    crate::open_notebook_in_editor(app, next_index);
}

/// Closes whichever tab corresponds to the active notebook, or returns Home if none.
pub(crate) fn close_active_tab(app: &mut PaperCraftApp) {
    sync_open_tabs(app);
    if let Some(pos) = app
        .view
        .open_tabs
        .iter()
        .position(|tab| tab.index == app.session.active_notebook)
    {
        close_tab(app, pos);
    } else if app.view.screen == AppScreen::Editor {
        crate::return_to_home(app);
    }
}

fn apply_action(app: &mut PaperCraftApp, action: TabAction) {
    match action {
        TabAction::Home => {
            if app.view.screen != AppScreen::Home {
                crate::return_to_home(app);
            }
        }
        TabAction::Activate(index) => {
            if app.view.screen != AppScreen::Editor || app.session.active_notebook != index {
                crate::open_notebook_in_editor(app, index);
            }
        }
        TabAction::Close(pos) => close_tab(app, pos),
        TabAction::NewNotebook => {
            crate::modal::open(app, crate::modal::ActiveModal::NewNotebook);
        }
    }
}

/// Renders the tab strip as a top panel and applies any tab interaction.
pub(crate) fn show(app: &mut PaperCraftApp, ui: &mut egui::Ui) -> TabStripOutput {
    sync_open_tabs(app);
    let t = Tokens::for_prefs(&app.session.prefs);
    let frame = egui::Frame::new()
        .fill(t.tab_strip_bg)
        .inner_margin(egui::Margin {
            left: 6,
            right: 6,
            top: 6,
            bottom: 0,
        });
    let mut out = TabStripOutput::default();
    let mut action = None;
    let panel = egui::Panel::top("papercraft_tab_strip")
        .frame(frame)
        .show(ui, |ui| {
            let (strip_rect, _) = ui.allocate_exact_size(
                Vec2::new(ui.available_width().max(0.0), TAB_HEIGHT),
                Sense::hover(),
            );
            action = draw_strip(app, ui, strip_rect, &t, &mut out.active_span);
        });
    out.rect = panel.response.rect;
    if let Some(action) = action {
        apply_action(app, action);
        ui.ctx().request_repaint();
    }
    out
}

/// Hides the border between the active tab and the header panel directly below it.
pub(crate) fn paint_active_tab_bridge(ui: &egui::Ui, out: &TabStripOutput, t: &Tokens) {
    let Some((left, right)) = out.active_span else {
        return;
    };
    if !out.rect.is_finite() || right - left <= 2.0 {
        return;
    }
    let y = out.rect.bottom();
    let bridge = Rect::from_min_max(pos2(left + 1.0, y - 1.0), pos2(right - 1.0, y + 1.0));
    ui.painter()
        .rect_filled(bridge, CornerRadius::ZERO, t.header_bg);
}

fn tab_corner_radius() -> CornerRadius {
    CornerRadius {
        nw: TAB_RADIUS,
        ne: TAB_RADIUS,
        sw: 0,
        se: 0,
    }
}

fn paint_tab_chrome(painter: &egui::Painter, rect: Rect, active: bool, hover_t: f32, t: &Tokens) {
    if active {
        painter.rect(
            rect,
            tab_corner_radius(),
            t.header_bg,
            Stroke::new(1.0, t.header_border),
            StrokeKind::Inside,
        );
    } else if hover_t > 0.01 {
        let fill = crate::motion::lerp_color(
            Color32::TRANSPARENT,
            t.header_bg.gamma_multiply(0.72),
            hover_t,
        );
        painter.rect_filled(
            rect.shrink2(Vec2::new(0.0, 2.0)),
            CornerRadius::same(8),
            fill,
        );
    }
}

fn paint_separator(painter: &egui::Painter, x: f32, cy: f32, t: &Tokens) {
    painter.line_segment(
        [
            pos2(x, cy - SEPARATOR_HALF_H),
            pos2(x, cy + SEPARATOR_HALF_H),
        ],
        Stroke::new(1.0, t.header_border),
    );
}

fn tab_title_galley(ui: &egui::Ui, title: &str, max_w: f32, color: Color32) -> Arc<egui::Galley> {
    let label = if title.trim().is_empty() {
        "Untitled"
    } else {
        title
    };
    let mut job = egui::text::LayoutJob::single_section(
        label.to_string(),
        egui::TextFormat {
            font_id: FontId::proportional(TAB_FONT_SIZE),
            color,
            ..Default::default()
        },
    );
    job.wrap.max_width = max_w.max(8.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    ui.painter().layout_job(job)
}

struct TabSlot {
    pos: usize,
    rect: Rect,
    active: bool,
    hovered: bool,
}

fn inactive_tab_fg(hover_t: f32, muted_at_rest: bool, t: &Tokens) -> Color32 {
    let rest = if muted_at_rest {
        t.tab_strip_fg_muted
    } else {
        t.tab_strip_fg
    };
    let hover_bg = crate::motion::lerp_color(t.tab_strip_bg, t.header_bg, 0.72);
    let hover_target = if crate::theme::wcag_contrast(t.header_fg, hover_bg)
        > crate::theme::wcag_contrast(t.tab_strip_fg, hover_bg)
    {
        t.header_fg
    } else {
        t.tab_strip_fg
    };
    crate::motion::lerp_color(rest, hover_target, hover_t)
}

fn draw_home_tab(
    ui: &mut egui::Ui,
    base_id: egui::Id,
    home_rect: Rect,
    on_home: bool,
    t: &Tokens,
) -> (egui::Response, Option<TabAction>) {
    let home_resp = ui
        .interact(home_rect, base_id.with("home"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Home (Shortcut: H)\nBrowse, search, and organize all of your notebooks.");
    let home_hover_t = crate::motion::hover_factor(
        ui.ctx(),
        base_id.with("home_hov"),
        home_resp.contains_pointer() && !on_home,
    );
    paint_tab_chrome(ui.painter(), home_rect, on_home, home_hover_t, t);
    let home_fg = if on_home {
        if t.is_dark { t.accent } else { t.header_fg }
    } else {
        inactive_tab_fg(home_hover_t, true, t)
    };
    paint_icon(
        ui.painter(),
        Rect::from_center_size(home_rect.center(), Vec2::splat(TAB_ICON_DIM)),
        Icon::Home,
        home_fg,
    );
    let action = (home_resp.clicked() && !on_home).then_some(TabAction::Home);
    (home_resp, action)
}

fn draw_new_tab_button(
    ui: &mut egui::Ui,
    base_id: egui::Id,
    new_rect: Rect,
    t: &Tokens,
) -> Option<TabAction> {
    let new_resp = ui
        .interact(new_rect, base_id.with("new"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("New Notebook\nCreate a notebook and open it in a new tab.");
    let new_hover_t =
        crate::motion::hover_factor(ui.ctx(), base_id.with("new_hov"), new_resp.hovered());
    if new_hover_t > 0.01 {
        ui.painter().rect_filled(
            new_rect,
            CornerRadius::same(8),
            crate::motion::lerp_color(
                Color32::TRANSPARENT,
                t.header_bg.gamma_multiply(0.75),
                new_hover_t,
            ),
        );
    }
    paint_icon(
        ui.painter(),
        Rect::from_center_size(new_rect.center(), Vec2::splat(TAB_ICON_DIM)),
        Icon::Plus,
        inactive_tab_fg(new_hover_t, true, t),
    );
    new_resp.clicked().then_some(TabAction::NewNotebook)
}

struct NotebookTabDraw<'a> {
    pos: usize,
    tab: &'a OpenTab,
    rect: Rect,
    active: bool,
    title: &'a str,
    cover: papercraft_doc::NotebookCover,
}

fn draw_notebook_tab(
    ui: &mut egui::Ui,
    base_id: egui::Id,
    spec: NotebookTabDraw<'_>,
    t: &Tokens,
) -> (TabSlot, Option<TabAction>) {
    let rect = spec.rect;
    let active = spec.active;
    let tab_id = base_id.with(("tab", spec.tab.id.as_str(), spec.tab.index));
    let display_title = if spec.title.trim().is_empty() {
        "Untitled"
    } else {
        spec.title
    };
    let tooltip = format!(
        "{display_title}\nClick to switch notebooks. Middle-click or press × to close the tab."
    );
    let resp = ui
        .interact(rect, tab_id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tooltip);
    let hovered = resp.contains_pointer();
    let allow_close = active || rect.width() >= TAB_CLOSE_MIN_WIDTH;
    let close_rect = Rect::from_center_size(
        pos2(
            rect.right() - TAB_PAD_X * 0.5 - TAB_CLOSE_SIZE * 0.5,
            rect.center().y,
        ),
        Vec2::splat(TAB_CLOSE_SIZE),
    );
    let close_resp = allow_close.then(|| {
        ui.interact(close_rect, tab_id.with("close"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Close tab")
    });

    let hover_t = crate::motion::hover_factor(ui.ctx(), tab_id.with("hov"), hovered && !active);
    let painter = ui.painter();
    paint_tab_chrome(painter, rect, active, hover_t, t);

    let content_painter = painter.with_clip_rect(painter.clip_rect().intersect(rect));
    let cover_left = if rect.width() < TAB_CLOSE_MIN_WIDTH && !active {
        rect.center().x - TAB_COVER_SIZE.x * 0.5
    } else {
        rect.left() + TAB_PAD_X
    };
    let cover_rect = Rect::from_min_size(
        pos2(cover_left, rect.center().y - TAB_COVER_SIZE.y * 0.5),
        TAB_COVER_SIZE,
    );
    crate::customization_modals::paint_hardcover_book_cover(
        &content_painter,
        cover_rect,
        spec.cover,
        None,
        t.header_border,
    );

    let text_left = cover_rect.right() + TAB_COVER_GAP;
    let text_right = if allow_close {
        close_rect.left() - 4.0
    } else {
        rect.right() - TAB_PAD_X
    };
    if text_right - text_left >= 16.0 {
        let text_color = if active {
            t.header_fg
        } else {
            inactive_tab_fg(hover_t, true, t)
        };
        let galley = tab_title_galley(ui, spec.title, text_right - text_left, text_color);
        let text_pos = pos2(text_left, rect.center().y - galley.size().y * 0.5);
        content_painter.galley_with_override_text_color(text_pos, galley, text_color);
    }

    if let Some(close_resp) = close_resp.as_ref()
        && (active || hovered)
    {
        let close_hovered = close_resp.hovered();
        if close_hovered {
            ui.painter().rect_filled(
                close_rect,
                CornerRadius::same(6),
                t.header_border.gamma_multiply(0.7),
            );
        }
        let close_fg = if active {
            if close_hovered {
                t.header_fg
            } else {
                t.header_fg_muted
            }
        } else {
            inactive_tab_fg(hover_t, !close_hovered, t)
        };
        paint_icon(
            ui.painter(),
            Rect::from_center_size(close_rect.center(), Vec2::splat(TAB_CLOSE_ICON_DIM)),
            Icon::Close,
            close_fg,
        );
    }

    let close_clicked = close_resp.as_ref().is_some_and(|r| r.clicked());
    let action = if close_clicked || resp.middle_clicked() {
        Some(TabAction::Close(spec.pos))
    } else if resp.clicked() && !active {
        Some(TabAction::Activate(spec.tab.index))
    } else {
        None
    };
    let slot = TabSlot {
        pos: spec.pos,
        rect,
        active,
        hovered,
    };
    (slot, action)
}

fn draw_tab_separators(
    painter: &egui::Painter,
    slots: &[TabSlot],
    home_quiet: bool,
    cy: f32,
    t: &Tokens,
) {
    if let Some(first_slot) = slots.first()
        && home_quiet
        && !first_slot.active
        && !first_slot.hovered
    {
        paint_separator(painter, first_slot.rect.left() - TAB_GAP, cy, t);
    }
    for pair in slots.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        if a.pos + 1 == b.pos && !a.active && !a.hovered && !b.active && !b.hovered {
            paint_separator(painter, a.rect.right() + TAB_GAP * 0.5, cy, t);
        }
    }
}

fn draw_strip(
    app: &PaperCraftApp,
    ui: &mut egui::Ui,
    strip: Rect,
    t: &Tokens,
    active_span: &mut Option<(f32, f32)>,
) -> Option<TabAction> {
    let base_id = egui::Id::unique("papercraft_tab_strip");
    let screen = if app.view.motion.screen.is_animating() {
        app.view.motion.screen.rendered
    } else {
        app.view.screen
    };
    let on_home = screen == AppScreen::Home;
    let in_editor = screen == AppScreen::Editor;
    let cy = strip.center().y;

    let home_rect = Rect::from_min_size(strip.min, Vec2::new(HOME_TAB_WIDTH, TAB_HEIGHT));
    let (home_resp, mut action) = draw_home_tab(ui, base_id, home_rect, on_home, t);
    if on_home {
        *active_span = Some((home_rect.left(), home_rect.right()));
    }

    let new_rect = Rect::from_center_size(
        pos2(strip.right() - NEW_TAB_SIZE * 0.5, cy),
        Vec2::splat(NEW_TAB_SIZE),
    );
    if let Some(new_action) = draw_new_tab_button(ui, base_id, new_rect, t) {
        action = Some(new_action);
    }

    let tabs = &app.view.open_tabs;
    let area_left = home_rect.right() + TAB_GAP * 2.0;
    let area_right = (new_rect.left() - 6.0).max(area_left);
    let area_w = area_right - area_left;
    let n = tabs.len();
    let mut slots: Vec<TabSlot> = Vec::new();
    if n > 0 && area_w.is_finite() && area_w >= TAB_MIN_WIDTH {
        let fit = ((area_w + TAB_GAP) / (TAB_MIN_WIDTH + TAB_GAP))
            .floor()
            .max(1.0) as usize;
        let visible = n.min(fit);
        let active_pos = in_editor
            .then(|| {
                tabs.iter()
                    .position(|tab| tab.index == app.session.active_notebook)
            })
            .flatten();
        let first = match active_pos {
            Some(p) if p >= visible => p + 1 - visible,
            _ => 0,
        };
        let gaps = TAB_GAP * visible.saturating_sub(1) as f32;
        let tab_w = ((area_w - gaps) / visible as f32).clamp(TAB_MIN_WIDTH, TAB_MAX_WIDTH);

        for (slot_idx, pos) in (first..(first + visible).min(n)).enumerate() {
            let x = area_left + slot_idx as f32 * (tab_w + TAB_GAP);
            let rect = Rect::from_min_size(pos2(x, strip.top()), Vec2::new(tab_w, TAB_HEIGHT));
            let tab = &tabs[pos];
            let active = active_pos == Some(pos);
            let (title, cover) = if tab.index == app.session.active_notebook
                && app.session.active_notebook < app.session.notebooks.len()
            {
                (app.session.doc.title.as_str(), app.session.doc.cover)
            } else if let Some(doc) = app.session.notebooks.get(tab.index) {
                (doc.title.as_str(), doc.cover)
            } else {
                continue;
            };

            let (slot, tab_action) = draw_notebook_tab(
                ui,
                base_id,
                NotebookTabDraw {
                    pos,
                    tab,
                    rect,
                    active,
                    title,
                    cover,
                },
                t,
            );
            if let Some(tab_action) = tab_action {
                action = Some(tab_action);
            }
            if active {
                *active_span = Some((rect.left(), rect.right()));
            }
            slots.push(slot);
        }
    }

    let home_quiet = !on_home && !home_resp.contains_pointer();
    draw_tab_separators(ui.painter(), &slots, home_quiet, cy, t);

    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn run_frame(
        app: &mut PaperCraftApp,
        ctx: &egui::Context,
        events: Vec<egui::Event>,
        time: f64,
    ) {
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(1280.0, 800.0),
            )),
            time: Some(time),
            events,
            ..Default::default()
        };
        let out = ctx.run_ui(raw, |ui| app.show_ui(ui));
        out.drop_without_applying_deltas();
    }

    fn click(app: &mut PaperCraftApp, ctx: &egui::Context, pt: egui::Pos2, time: f64) {
        for (pressed, dt) in [(true, 0.0), (false, 0.05)] {
            run_frame(
                app,
                ctx,
                vec![
                    egui::Event::PointerMoved(pt),
                    egui::Event::PointerButton {
                        pos: pt,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                time + dt,
            );
        }
    }

    fn app_with_three_notebooks() -> PaperCraftApp {
        let mut app = PaperCraftApp::default();
        for title in ["Second", "Third"] {
            let _ = app
                .session
                .execute("notebook.create", &json!({ "title": title }));
        }
        app
    }

    #[test]
    fn opening_notebooks_adds_tabs_and_prunes_missing_notebooks() {
        let mut app = app_with_three_notebooks();
        let count = app.session.notebooks.len();
        assert!(count >= 3);
        for index in 0..count {
            crate::open_notebook_in_editor(&mut app, index);
            sync_open_tabs(&mut app);
        }
        assert_eq!(app.view.open_tabs.len(), count);
        // Re-opening an already open notebook never duplicates its tab.
        crate::open_notebook_in_editor(&mut app, 0);
        sync_open_tabs(&mut app);
        assert_eq!(app.view.open_tabs.len(), count);

        // Stale or hostile tab references are dropped instead of crashing.
        app.view.open_tabs.push(OpenTab {
            id: "missing-notebook".to_string(),
            index: usize::MAX,
        });
        sync_open_tabs(&mut app);
        assert_eq!(app.view.open_tabs.len(), count);
    }

    #[test]
    fn closing_tabs_switches_to_neighbor_and_returns_home_when_last_closes() {
        let mut app = app_with_three_notebooks();
        for index in 0..3 {
            crate::open_notebook_in_editor(&mut app, index);
            sync_open_tabs(&mut app);
        }
        assert_eq!(app.session.active_notebook, 2);
        let active_pos = app
            .view
            .open_tabs
            .iter()
            .position(|tab| tab.index == 2)
            .unwrap_or(0);
        close_tab(&mut app, active_pos);
        assert_eq!(app.view.screen, AppScreen::Editor);
        assert_eq!(app.view.open_tabs.len(), 2);
        assert_ne!(app.session.active_notebook, 2);

        // Closing an inactive tab keeps the current notebook open.
        let active = app.session.active_notebook;
        let inactive_pos = app
            .view
            .open_tabs
            .iter()
            .position(|tab| tab.index != active)
            .unwrap_or(0);
        close_tab(&mut app, inactive_pos);
        assert_eq!(app.session.active_notebook, active);
        assert_eq!(app.view.open_tabs.len(), 1);

        close_tab(&mut app, 0);
        assert!(app.view.open_tabs.is_empty());
        assert_eq!(app.view.screen, AppScreen::Home);

        // Out-of-range closes are ignored.
        close_tab(&mut app, 99);
    }

    #[test]
    fn strip_tabs_switch_notebooks_and_home_tab_returns_home() {
        let mut app = app_with_three_notebooks();
        crate::open_notebook_in_editor(&mut app, 0);
        sync_open_tabs(&mut app);
        crate::open_notebook_in_editor(&mut app, 1);
        sync_open_tabs(&mut app);
        app.view.motion.screen = crate::motion::ScreenTransition::new(AppScreen::Editor);

        let ctx = egui::Context::default();
        run_frame(&mut app, &ctx, Vec::new(), 0.0);
        run_frame(&mut app, &ctx, Vec::new(), 0.05);

        // First notebook tab sits right of the Home tab.
        let first_tab = pos2(
            6.0 + HOME_TAB_WIDTH + TAB_GAP * 2.0 + 40.0,
            TAB_TOP_GAP + TAB_HEIGHT * 0.5,
        );
        click(&mut app, &ctx, first_tab, 0.2);
        assert_eq!(app.session.active_notebook, 0);
        assert_eq!(app.view.screen, AppScreen::Editor);

        let home = pos2(6.0 + HOME_TAB_WIDTH * 0.5, TAB_TOP_GAP + TAB_HEIGHT * 0.5);
        click(&mut app, &ctx, home, 0.5);
        assert_eq!(app.view.screen, AppScreen::Home);
        assert_eq!(
            app.view.open_tabs.len(),
            2,
            "tabs stay open while browsing Home"
        );

        // Clicking '+' opens the New Notebook modal.
        let new_btn = pos2(
            1280.0 - 6.0 - NEW_TAB_SIZE * 0.5,
            TAB_TOP_GAP + TAB_HEIGHT * 0.5,
        );
        click(&mut app, &ctx, new_btn, 0.7);
        assert_eq!(
            app.view.active_modal,
            Some(crate::modal::ActiveModal::NewNotebook)
        );
    }

    #[test]
    fn clicking_tab_close_button_closes_tab_and_deleting_notebook_syncs_tabs() {
        let mut app = app_with_three_notebooks();
        for index in 0..3 {
            crate::open_notebook_in_editor(&mut app, index);
        }
        app.view.motion.screen = crate::motion::ScreenTransition::new(AppScreen::Editor);

        let ctx = egui::Context::default();
        run_frame(&mut app, &ctx, Vec::new(), 0.0);
        run_frame(&mut app, &ctx, Vec::new(), 0.05);

        // Tab width for 3 tabs at 1280px hits TAB_MAX_WIDTH (224.0).
        let area_left = 6.0 + HOME_TAB_WIDTH + TAB_GAP * 2.0;
        let first_tab_right = area_left + TAB_MAX_WIDTH;
        let close_pt = pos2(
            first_tab_right - TAB_PAD_X * 0.5 - TAB_CLOSE_SIZE * 0.5,
            TAB_TOP_GAP + TAB_HEIGHT * 0.5,
        );
        click(&mut app, &ctx, close_pt, 0.2);
        assert_eq!(app.view.open_tabs.len(), 2);
        assert_eq!(
            app.session.active_notebook, 2,
            "closing inactive tab keeps active tab"
        );

        // Deleting a remaining open notebook drops its tab and updates indices safely.
        let _ = app
            .session
            .execute("notebook.delete", &json!({ "index": 1 }));
        sync_open_tabs(&mut app);
        assert_eq!(app.view.open_tabs.len(), 1);
    }

    #[test]
    fn long_tab_title_truncates_inside_tab_strip_bounds() {
        let mut app = PaperCraftApp::default();
        let _ = app.session.execute(
            "notebook.create",
            &json!({ "title": "Very Long Multi-Semester Differential Geometry Study Notebook" }),
        );
        let idx = app.session.notebooks.len() - 1;
        crate::open_notebook_in_editor(&mut app, idx);
        app.view.motion.screen = crate::motion::ScreenTransition::new(AppScreen::Editor);

        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(
                egui::Pos2::ZERO,
                Vec2::new(1280.0, 800.0),
            )),
            ..Default::default()
        };
        let mut tab_texts = Vec::new();
        let out = ctx.run_ui(raw, |ui| app.show_ui(ui));
        fn collect_strip_texts(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Text(text) if text.pos.y < TAB_STRIP_HEIGHT => {
                    let rendered: String = text
                        .galley
                        .rows
                        .iter()
                        .flat_map(|r| r.row.glyphs.iter().map(|g| g.chr))
                        .collect();
                    out.push(rendered);
                }
                egui::Shape::Vec(items) => {
                    for s in items {
                        collect_strip_texts(s, out);
                    }
                }
                _ => {}
            }
        }
        for clipped in &out.shapes {
            collect_strip_texts(&clipped.shape, &mut tab_texts);
        }
        out.drop_without_applying_deltas();
        assert!(
            tab_texts.iter().any(|t| t.ends_with('…')),
            "expected truncated notebook title in tab strip, got {tab_texts:?}"
        );
    }

    #[test]
    fn editor_top_strip_omits_profile_and_sync_status_pill() {
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = crate::motion::ScreenTransition::new(AppScreen::Editor);

        let collect_top_text = |app: &mut PaperCraftApp| -> String {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(1280.0, 800.0),
                )),
                ..Default::default()
            };
            let mut buf = String::new();
            let out = ctx.run_ui(raw, |ui| app.show_ui(ui));
            fn visit(shape: &egui::Shape, buf: &mut String) {
                match shape {
                    egui::Shape::Text(text) if text.pos.y < TAB_STRIP_HEIGHT => {
                        buf.push_str(text.galley.text());
                        buf.push('\n');
                    }
                    egui::Shape::Vec(items) => {
                        for item in items {
                            visit(item, buf);
                        }
                    }
                    _ => {}
                }
            }
            for c in &out.shapes {
                visit(&c.shape, &mut buf);
            }
            out.drop_without_applying_deltas();
            buf
        };

        let local_text = collect_top_text(&mut app);
        assert!(
            !local_text.contains("Saved on this device"),
            "unexpected local sync pill in editor tab strip:\n{local_text}"
        );

        let acct = papercraft_sync::ActiveAccountSession {
            server_url: papercraft_sync_protocol::DEFAULT_SERVER_URL.to_string(),
            user: papercraft_sync_protocol::UserProfile {
                user_id: "u-ind".to_string(),
                email: Some("ind@example.com".to_string()),
                username: "induser".to_string(),
                display_name: "IndUser".to_string(),
                created_at_ms: 1000,
                must_change_password: false,
            },
            access_token: "a".to_string(),
            refresh_token: "r".to_string(),
            access_expires_at_ms: 99999,
            refresh_expires_at_ms: 99999,
            device_name: "PC".to_string(),
            last_auth_ms: 1000,
        };
        let pull = papercraft_sync_protocol::SyncPullResponse {
            server_cursor: 1,
            user: acct.user.clone(),
            notebooks: Vec::new(),
            folders: Vec::new(),
            studio_prefs: None,
        };
        app.session.enter_online_server_session(acct, pull);

        let synced_text = collect_top_text(&mut app);
        assert!(
            !synced_text.contains("Synced to cloud") && !synced_text.contains("Synced to server"),
            "unexpected server sync pill in editor tab strip:\n{synced_text}"
        );
    }
}
