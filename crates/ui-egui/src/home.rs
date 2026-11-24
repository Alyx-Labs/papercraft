//! Notebooks Home & Library screen displaying all notebooks with visual covers
//! and live first-page previews. Selecting any notebook opens it in the editor.

use egui::{Color32, CornerRadius, Margin, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2};
use papercraft_doc::{
    Document, LibrarySortBy, LibraryViewMode, MAX_FOLDER_DEPTH, MAX_LIBRARY_FOLDERS,
};
use papercraft_sync::EntitySyncStatus;
use serde_json::json;
use std::sync::Arc;

use crate::PaperCraftApp;
use crate::customization_modals::{paint_hardcover_book_cover, paint_paper_preview_colors};
use crate::icons::{
    Icon, SwitchLayout, SwitchOption, compact_icon_button, icon_button, paint_icon,
    segmented_switch,
};
use crate::library_folders::{
    FolderNode, LibraryAction, LibraryScope, NotebookDragPayload, SIDEBAR_MIN_VIEWPORT,
    apply_library_action, current_scope, display_folder_path, draw_breadcrumb, draw_browse_button,
    draw_subfolder_tiles, folder_depth, folder_leaf, folder_tree, folder_visual_colors, menu_item,
    move_to_folder_menu_contents, path_within, plural, show_sidebar_panel, subfolders_of,
};
use crate::modal::{self, ActiveModal};
use crate::pages_sidebar::paint_thumbnail_strokes_cached;
use crate::theme::Tokens;

const HEADER_ROW_HEIGHT: f32 = 28.0;
const MIN_CARD_WIDTH: f32 = 232.0;
const CARD_HEIGHT: f32 = 272.0;
const CARD_MARGIN: i8 = 12;
const CARD_GAP: f32 = 16.0;
const ROW_GAP: f32 = 16.0;
const MIN_SIDE_PAD: f32 = 24.0;
const SCROLLBAR_RESERVE: f32 = 16.0;
const SEARCH_INPUT_WIDTH: f32 = 220.0;

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Debug)]
pub(crate) struct HomeMetrics {
    pub(crate) header_rect: Rect,
    pub(crate) sidebar_rect: Option<Rect>,
    pub(crate) content_rect: Rect,
    pub(crate) folder_tile_rects: Vec<Rect>,
    pub(crate) card_rects: Vec<Rect>,
    pub(crate) create_card_rect: Option<Rect>,
    pub(crate) first_card_actions_rect: Rect,
}

#[derive(Clone, Copy, Debug)]
struct HomeGridLayout {
    cols: usize,
    card_w: f32,
    content_w: f32,
    side_pad: f32,
}

#[derive(Clone, Copy, Debug)]
enum GridSlot {
    Notebook(usize),
    Trash(usize),
    CreateNew,
}

/// Renders the Notebooks Library home screen.
pub fn show(app: &mut PaperCraftApp, ui: &mut egui::Ui) {
    let _ = show_inner(app, ui);
}

pub(crate) fn show_inner(app: &mut PaperCraftApp, ui: &mut egui::Ui) -> HomeMetrics {
    let t = Tokens::for_theme(app.session.prefs.theme);
    let header_rect = draw_home_header(app, ui, &t);
    let show_sidebar = ui.available_width() >= SIDEBAR_MIN_VIEWPORT;
    let sidebar_rect = show_sidebar.then(|| show_sidebar_panel(app, ui, &t));
    crate::inspector::show(app, ui);

    let mut folder_tile_rects = Vec::new();
    let mut grid_metrics = (Vec::new(), None, Rect::NOTHING);
    let mut content_rect = Rect::NOTHING;

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.fill(t.panel_bg))
        .show(ui, |ui| {
            let outer_rect = ui.available_rect_before_wrap();
            let layout = compute_grid_layout(outer_rect.width());
            let content_left = outer_rect.left() + layout.side_pad;
            let content_right =
                (outer_rect.right() - layout.side_pad).max(content_left + layout.content_w);
            let content_top = outer_rect.top() + 20.0;
            let content_bottom = (outer_rect.bottom() - 20.0).max(content_top + 40.0);
            content_rect = Rect::from_min_max(
                pos2(content_left, content_top),
                pos2(content_right, content_bottom),
            );
            let scope = current_scope(app);
            let scope_key = scope.motion_key();
            let tree = folder_tree(app);
            let searching = !app.view.library_search.trim().is_empty();
            let show_trash = scope == LibraryScope::Trash;
            let subfolders = if !searching && let LibraryScope::Folder(ref path) = scope {
                subfolders_of(&tree, path)
            } else {
                Vec::new()
            };
            let matching = if show_trash {
                matching_trash_indices(app)
            } else {
                matching_notebook_indices(app)
            };
            let show_create_card = !searching
                && app.session.notebooks.len() < papercraft_engine::MAX_NOTEBOOKS
                && matches!(
                    scope,
                    LibraryScope::All | LibraryScope::Folder(_) | LibraryScope::Unfiled
                );
            let show_empty = matching.is_empty() && subfolders.is_empty() && !show_create_card;
            let view_key = (
                app.session.prefs.library_view_mode,
                app.session.prefs.library_sort_by,
                app.session.prefs.library_sort_ascending,
                show_empty,
            );
            let dt = ui.ctx().input(|i| i.stable_dt);
            let scope_changed = app
                .view
                .motion
                .home_scope
                .last_key
                .as_deref()
                .is_some_and(|prev| prev != scope_key);
            if app.view.motion.home_scope.advance(scope_key, dt) {
                ui.ctx().request_repaint();
            }
            if scope_changed {
                app.view.motion.home_view.sync(view_key);
            } else if app.view.motion.home_view.advance(view_key, dt) {
                ui.ctx().request_repaint();
            }
            let scope_opacity = app.view.motion.home_scope.opacity();
            let view_opacity = app.view.motion.home_view.opacity();

            ui.scope_builder(egui::UiBuilder::new().max_rect(content_rect), |ui| {
                ui.multiply_opacity(scope_opacity);
                ui.allocate_ui_with_layout(
                    Vec2::new(layout.content_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        if let Some(err_msg) = app.view.pdf_import_error.clone() {
                            draw_home_pdf_error_alert(app, ui, &err_msg, &t);
                            ui.add_space(10.0);
                        }
                        let header_info = SectionHeaderInfo {
                            scope: &scope,
                            tree: &tree,
                            subfolder_count: subfolders.len(),
                            shown_count: matching.len(),
                            searching,
                        };
                        draw_library_section_header(app, ui, &header_info, &t);
                        if !show_trash || !show_sidebar || searching {
                            ui.add_space(10.0);
                            draw_library_utility_bar(
                                app,
                                ui,
                                show_sidebar,
                                show_trash,
                                searching,
                                &t,
                            );
                        }
                    },
                );
                ui.add_space(14.0);

                ui.style_mut().spacing.scroll.floating = true;
                egui::ScrollArea::vertical()
                    .id_salt("papercraft_home_library_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.multiply_opacity(view_opacity);
                        if show_empty {
                            ui.set_width(layout.content_w);
                            if show_trash {
                                draw_empty_trash_state(ui, searching, &t);
                            } else if searching {
                                draw_empty_search_state(app, ui, &scope, &t);
                            } else {
                                draw_empty_scope_state(ui, &scope, &t);
                            }
                            return;
                        }

                        if !subfolders.is_empty() {
                            ui.label(
                                RichText::new("FOLDERS")
                                    .size(10.5)
                                    .strong()
                                    .color(t.text_muted),
                            );
                            ui.add_space(6.0);
                            folder_tile_rects = draw_subfolder_tiles(
                                app,
                                ui,
                                &subfolders,
                                layout.cols,
                                layout.card_w,
                                CARD_GAP,
                                &t,
                            );
                            if !matching.is_empty() || show_create_card {
                                ui.add_space(18.0);
                                ui.label(
                                    RichText::new("NOTEBOOKS")
                                        .size(10.5)
                                        .strong()
                                        .color(t.text_muted),
                                );
                                ui.add_space(6.0);
                            }
                        }

                        let mut slots: Vec<GridSlot> = if show_trash {
                            matching.into_iter().map(GridSlot::Trash).collect()
                        } else {
                            matching.into_iter().map(GridSlot::Notebook).collect()
                        };
                        if show_create_card {
                            slots.push(GridSlot::CreateNew);
                        }

                        grid_metrics = if !show_trash
                            && app.session.prefs.library_view_mode == LibraryViewMode::List
                        {
                            draw_notebook_list(app, ui, &slots, &tree, layout.content_w, &t)
                        } else {
                            draw_notebook_grid(
                                app,
                                ui,
                                &slots,
                                &tree,
                                layout.cols,
                                layout.card_w,
                                &t,
                            )
                        };
                        ui.add_space(12.0);
                    });
            });
        });

    if let Some(payload) = egui::DragAndDrop::payload::<NotebookDragPayload>(ui.ctx())
        && let Some(doc) = app.session.notebooks.get(payload.index)
    {
        crate::library_folders::paint_drag_ghost(ui.ctx(), &doc.title, &t);
    }

    HomeMetrics {
        header_rect,
        sidebar_rect,
        content_rect,
        folder_tile_rects,
        card_rects: grid_metrics.0,
        create_card_rect: grid_metrics.1,
        first_card_actions_rect: grid_metrics.2,
    }
}

fn compute_grid_layout(avail_w: f32) -> HomeGridLayout {
    let budget = (avail_w - MIN_SIDE_PAD * 2.0 - SCROLLBAR_RESERVE).max(180.0);
    let cols = ((budget + CARD_GAP) / (MIN_CARD_WIDTH + CARD_GAP))
        .floor()
        .max(1.0) as usize;
    let total_gaps = cols.saturating_sub(1) as f32 * CARD_GAP;
    let card_w = ((budget - total_gaps) / cols as f32).max(180.0);
    let grid_w = cols as f32 * card_w + total_gaps;
    let content_w = budget.max(grid_w);
    let raw_pad = ((avail_w - (content_w + SCROLLBAR_RESERVE)) * 0.5).max(MIN_SIDE_PAD);
    let max_pad = ((avail_w - content_w) * 0.5).max(0.0);
    let side_pad = raw_pad.min(max_pad);
    HomeGridLayout {
        cols,
        card_w,
        content_w,
        side_pad,
    }
}

fn draw_notebook_grid(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    slots: &[GridSlot],
    folder_tree: &[FolderNode],
    cols: usize,
    card_w: f32,
    t: &Tokens,
) -> (Vec<Rect>, Option<Rect>, Rect) {
    let mut card_rects = Vec::new();
    let mut create_card_rect = None;
    let mut first_actions_rect = Rect::NOTHING;
    let cols = cols.max(1);
    let grid_w = cols as f32 * card_w + cols.saturating_sub(1) as f32 * CARD_GAP;

    ui.spacing_mut().item_spacing = Vec2::new(CARD_GAP, ROW_GAP);
    for row in slots.chunks(cols) {
        let (row_rect, _) = ui.allocate_exact_size(Vec2::new(grid_w, CARD_HEIGHT), Sense::hover());
        for (col_idx, &slot) in row.iter().enumerate() {
            let cell_min = pos2(
                row_rect.left() + col_idx as f32 * (card_w + CARD_GAP),
                row_rect.top(),
            );
            let cell_rect = Rect::from_min_size(cell_min, Vec2::new(card_w, CARD_HEIGHT));
            let mut cell_ui = ui.new_child(
                egui::UiBuilder::new()
                    .max_rect(cell_rect)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            match slot {
                GridSlot::Notebook(idx) => {
                    let (card_rect, actions_rect) =
                        draw_notebook_card(app, &mut cell_ui, idx, card_w, folder_tree, t);
                    if first_actions_rect == Rect::NOTHING {
                        first_actions_rect = actions_rect;
                    }
                    card_rects.push(card_rect);
                }
                GridSlot::Trash(idx) => {
                    card_rects.push(draw_trash_card(app, &mut cell_ui, idx, card_w, t));
                }
                GridSlot::CreateNew => {
                    create_card_rect =
                        Some(draw_create_notebook_card(app, &mut cell_ui, card_w, t));
                }
            }
        }
    }
    (card_rects, create_card_rect, first_actions_rect)
}

#[derive(Clone)]
enum CachedBrandLogo {
    Ready(egui::TextureHandle),
    Failed,
}

fn brand_logo_texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
    let cache_id = egui::Id::unique("papercraft-brand-logo-128");
    if let Some(cached) = ctx.data_mut(|d| d.get_temp::<CachedBrandLogo>(cache_id)) {
        return match cached {
            CachedBrandLogo::Ready(tex) => Some(tex),
            CachedBrandLogo::Failed => None,
        };
    }
    const LOGO_PNG: &[u8] =
        include_bytes!("../../../apps/papercraft/assets/branding/papercraft-128.png");
    let decoded = match image::load_from_memory(LOGO_PNG) {
        Ok(img) => img.into_rgba8(),
        Err(err) => {
            log::warn!("Could not decode embedded PaperCraft logo PNG: {err}");
            ctx.data_mut(|d| d.insert_temp(cache_id, CachedBrandLogo::Failed));
            return None;
        }
    };
    let size = [decoded.width() as usize, decoded.height() as usize];
    let color_image = egui::ColorImage::from_rgba_unmultiplied(size, decoded.as_raw());
    let tex = ctx.load_texture(
        "papercraft-brand-logo",
        color_image,
        egui::TextureOptions::LINEAR,
    );
    ctx.data_mut(|d| d.insert_temp(cache_id, CachedBrandLogo::Ready(tex.clone())));
    Some(tex)
}

fn draw_home_header(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) -> Rect {
    let frame = egui::Frame::new()
        .fill(t.toolbar_bg)
        .inner_margin(Margin::symmetric(12, 8))
        .stroke(Stroke::new(1.0, t.border));

    egui::Panel::top("papercraft_home_header")
        .frame(frame)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.set_height(HEADER_ROW_HEIGHT);
                ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);

                let (badge_rect, _) =
                    ui.allocate_exact_size(Vec2::splat(HEADER_ROW_HEIGHT), Sense::hover());
                if let Some(logo_tex) = brand_logo_texture(ui.ctx()) {
                    ui.painter().image(
                        logo_tex.id(),
                        badge_rect,
                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                        Color32::WHITE,
                    );
                } else {
                    ui.painter()
                        .rect_filled(badge_rect, CornerRadius::same(7), t.accent_soft);
                    paint_icon(
                        ui.painter(),
                        badge_rect.shrink(6.5),
                        Icon::Library,
                        t.accent,
                    );
                }

                ui.add_space(2.0);
                ui.label(
                    RichText::new("PaperCraft")
                        .size(14.5)
                        .strong()
                        .color(t.text),
                );
                if ui.available_width() >= 920.0 {
                    ui.separator();
                    ui.label(
                        RichText::new("Notebooks Library")
                            .size(12.5)
                            .color(t.text_muted),
                    );
                }

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
                        "Settings (Shortcut: S)\nStudio theme, app scale, pen stabilization, and live telemetry.",
                    )
                    .clicked()
                    {
                        if !app.view.inspector_open {
                            app.view.settings_tab = crate::SettingsTab::Appearance;
                        }
                        app.view.inspector_open = !app.view.inspector_open;
                    }

                    #[cfg(not(target_arch = "wasm32"))]
                    draw_home_update_pill(app, ui, t);

                    let pkg_resp =
                        icon_button(ui, Icon::Library, Some("Package"), false, true, t)
                            .on_hover_text(
                                "Export or import a complete backup package (.pcraftpkg) with all saves and notebooks",
                            );
                    let mut export_pkg = false;
                    let mut import_pkg = false;
                    egui::Popup::menu(&pkg_resp).width(270.0).show(|ui| {
                        if menu_item(
                            ui,
                            Icon::Export,
                            "Export Complete Package (.pcraftpkg)...",
                            true,
                            false,
                            t,
                        )
                        .clicked()
                        {
                            export_pkg = true;
                        }
                        if menu_item(
                            ui,
                            Icon::FolderOpen,
                            "Import Complete Package (.pcraftpkg)...",
                            true,
                            false,
                            t,
                        )
                        .clicked()
                        {
                            import_pkg = true;
                        }
                    });
                    if export_pkg {
                        let _ = app.trigger_export_package_dialog();
                    } else if import_pkg {
                        let _ = app.trigger_import_package_dialog();
                    }

                    if icon_button(ui, Icon::AddPage, Some("Import PDF"), false, true, t)
                        .on_hover_text("Import a PDF file as a new annotated notebook")
                        .clicked()
                    {
                        app.trigger_import_pdf_dialog(crate::PdfImportTarget::NewNotebook);
                    }

                    if icon_button(ui, Icon::Export, Some("Open .pcraft"), false, true, t)
                        .on_hover_text("Open a .pcraft notebook file from disk (Ctrl+O)")
                        .clicked()
                    {
                        app.trigger_open_notebook_dialog();
                    }

                    draw_account_sync_pill(app, ui, t);

                    draw_search_input(app, ui, t);
                });
            });
        })
        .response
        .rect
}

fn truncate_label(raw: &str, max_chars: usize) -> String {
    let mut chars = raw.chars();
    let collected: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{collected}…")
    } else {
        collected
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn draw_home_update_pill(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    use papercraft_updater::UpdateStatus;

    let label = match app.updater.status() {
        UpdateStatus::Available(u) => format!("Update v{}", u.version),
        UpdateStatus::Downloading { .. } => "Updating...".to_string(),
        UpdateStatus::ReadyToInstall { .. } => "Restart to Update".to_string(),
        _ => return,
    };
    let modal_open = app.view.active_modal == Some(ActiveModal::AppUpdate);
    if compact_icon_button(ui, Icon::MoveDown, Some(&label), modal_open, true, t)
        .on_hover_text("A newer PaperCraft release is available. Click to review and install.")
        .clicked()
    {
        modal::open(app, ActiveModal::AppUpdate);
    }
}

fn draw_account_sync_pill(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let summary = app.session.sync_status_summary();
    let cloud_pending = app.session.cloud_pending_changes_count();
    let (state_text, cloud_dot) = crate::account_modal::status_badge_style(summary.connection, t);
    let (icon, label, dot_color) = if summary.viewing_cloud {
        let short = truncate_label(&summary.workspace_badge_text(), 18);
        (Icon::Account, short, cloud_dot)
    } else {
        let prof = summary
            .active_local_profile_name
            .as_deref()
            .unwrap_or("Personal");
        let short = truncate_label(prof, 12);
        (Icon::Inbox, format!("Local · {short}"), t.text_muted)
    };

    let font = egui::FontId::proportional(12.0);
    let galley = ui.painter().layout_no_wrap(label, font, t.text);
    let icon_dim = 14.0_f32;
    let width = 10.0 + icon_dim + 6.0 + galley.size().x + 8.0 + 7.0 + 10.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, HEADER_ROW_HEIGHT), Sense::click());

    if ui.is_rect_visible(rect) {
        let active = app.view.screen == crate::AppScreen::Profile
            || app.view.active_modal == Some(ActiveModal::AccountSync);
        let hover_t = crate::motion::hover_factor(ui.ctx(), resp.id.with("hov"), resp.hovered());
        let active_t = crate::motion::hover_factor(ui.ctx(), resp.id.with("act"), active);
        let fill = crate::motion::lerp_color(t.field_bg, t.accent_soft, hover_t.max(active_t));
        let stroke = crate::motion::lerp_color(t.border, t.accent, active_t);
        let p = ui.painter();
        p.rect(
            rect,
            CornerRadius::same(7),
            fill,
            egui::Stroke::new(1.0, stroke),
            egui::StrokeKind::Inside,
        );
        let ir = Rect::from_center_size(
            pos2(rect.left() + 10.0 + icon_dim * 0.5, rect.center().y),
            Vec2::splat(icon_dim),
        );
        paint_icon(p, ir, icon, t.text);
        let gx = ir.right() + 6.0;
        p.galley(
            pos2(gx, rect.center().y - galley.size().y * 0.5),
            galley,
            t.text,
        );
        let dot_c = pos2(rect.right() - 12.5, rect.center().y);
        p.circle_filled(dot_c, 3.5, dot_color);
    }

    let tip = if summary.viewing_cloud {
        let handle = summary
            .username
            .as_deref()
            .map_or_else(String::new, |u| format!(" · @{u}"));
        format!(
            "Profile: {}{handle}\nStatus: {} ({cloud_pending} waiting to sync)\nServer: {}\nClick to open Profile & Cloud Sync.",
            summary.workspace_badge_text(),
            state_text,
            summary.server_url
        )
    } else if summary.signed_in {
        let prof = summary
            .active_local_profile_name
            .as_deref()
            .unwrap_or("Personal");
        let cloud_acct = summary
            .email
            .as_deref()
            .or(summary.username.as_deref())
            .unwrap_or("connected account");
        format!(
            "Workspace: Local Only ({prof})\nCloud account ({cloud_acct}) is connected separately.\nClick to open Profile & Workspaces."
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

    if resp
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tip)
        .clicked()
    {
        crate::open_profile_screen(app);
    }
}

fn draw_search_input(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let avail_w = ui.available_width();
    let search_w = if avail_w.is_finite() {
        avail_w.clamp(120.0, SEARCH_INPUT_WIDTH)
    } else {
        SEARCH_INPUT_WIDTH
    };
    let (box_rect, box_resp) =
        ui.allocate_exact_size(Vec2::new(search_w, HEADER_ROW_HEIGHT), Sense::hover());

    let edit_id = ui.make_persistent_id("papercraft_home_search_input");
    let has_focus = ui.ctx().memory(|m| m.has_focus(edit_id));
    let focus_t = crate::motion::hover_factor(ui.ctx(), edit_id.with("focus_t"), has_focus);
    let hover_t = crate::motion::hover_factor(
        ui.ctx(),
        edit_id.with("hov_t"),
        box_resp.hovered() && !has_focus,
    );
    let rest_stroke =
        crate::motion::lerp_color(t.border, t.text_muted.gamma_multiply(0.5), hover_t);
    let stroke = Stroke::new(
        crate::motion::lerp_f32(1.0, 1.5, focus_t),
        crate::motion::lerp_color(rest_stroke, t.accent, focus_t),
    );

    let p = ui.painter();
    p.rect_filled(box_rect, CornerRadius::same(7), t.field_bg);
    p.rect_stroke(box_rect, CornerRadius::same(7), stroke, StrokeKind::Inside);

    let inner_rect = box_rect.shrink2(Vec2::new(9.0, 2.0));
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner_rect)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.spacing_mut().item_spacing.x = 6.0;

    let (icon_rect, _) = child.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
    paint_icon(child.painter(), icon_rect, Icon::Search, t.text_muted);

    let clear_slot_w = 16.0_f32;
    let text_w = (inner_rect.width() - 14.0 - clear_slot_w - 12.0).max(60.0);
    child.add(
        egui::TextEdit::singleline(&mut app.view.library_search)
            .id(edit_id)
            .desired_width(text_w)
            .frame(egui::Frame::NONE)
            .margin(egui::Margin::ZERO)
            .align(egui::Align2::LEFT_CENTER)
            .hint_text(if app.view.library_show_trash {
                "Search Trash..."
            } else {
                "Search notebooks..."
            }),
    );

    if !app.view.library_search.is_empty() {
        let (clear_rect, clear_resp) =
            child.allocate_exact_size(Vec2::splat(clear_slot_w), Sense::click());
        let clear_hover_t = crate::motion::hover_factor(
            child.ctx(),
            clear_resp.id.with("hov"),
            clear_resp.hovered(),
        );
        let clear_color = crate::motion::lerp_color(t.text_muted, t.text, clear_hover_t);
        paint_icon(child.painter(), clear_rect, Icon::Close, clear_color);
        if clear_resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Clear search filter")
            .clicked()
        {
            app.view.library_search.clear();
        }
    }
}

struct SectionHeaderInfo<'a> {
    scope: &'a LibraryScope,
    tree: &'a [FolderNode],
    subfolder_count: usize,
    shown_count: usize,
    searching: bool,
}

fn scope_matches_notebook(doc: &Document, scope: &LibraryScope, searching: bool) -> bool {
    match scope {
        LibraryScope::All => true,
        LibraryScope::Favorites => doc.favorite,
        LibraryScope::Unfiled => doc.folder_path.is_empty(),
        LibraryScope::Trash => false,
        LibraryScope::Folder(path) => {
            if searching {
                path_within(&doc.folder_path, path)
            } else {
                doc.folder_path.eq_ignore_ascii_case(path)
            }
        }
        LibraryScope::Tag(tag) => doc.tags.iter().any(|t| t.eq_ignore_ascii_case(tag)),
    }
}

fn matching_notebook_indices(app: &PaperCraftApp) -> Vec<usize> {
    let query = app.view.library_search.trim().to_ascii_lowercase();
    let searching = !query.is_empty();
    let scope = current_scope(app);

    let mut matched: Vec<usize> = app
        .session
        .notebooks
        .iter()
        .enumerate()
        .filter(|(_, doc)| {
            if !scope_matches_notebook(doc, &scope, searching) {
                return false;
            }
            if !searching {
                return true;
            }
            doc.title.to_ascii_lowercase().contains(&query)
                || doc
                    .cover
                    .display_label()
                    .to_ascii_lowercase()
                    .contains(&query)
                || doc
                    .paper_template
                    .label()
                    .to_ascii_lowercase()
                    .contains(&query)
                || doc
                    .paper_color
                    .label()
                    .to_ascii_lowercase()
                    .contains(&query)
                || doc.folder_path.to_ascii_lowercase().contains(&query)
                || display_folder_path(&doc.folder_path)
                    .to_ascii_lowercase()
                    .contains(&query)
                || doc
                    .tags
                    .iter()
                    .any(|t| t.to_ascii_lowercase().contains(&query))
        })
        .map(|(idx, _)| idx)
        .collect();

    let sort_by = app.session.prefs.library_sort_by;
    let asc = app.session.prefs.library_sort_ascending;
    let notebooks = &app.session.notebooks;
    matched.sort_by(|&a, &b| {
        let da = &notebooks[a];
        let db = &notebooks[b];
        let ord = match sort_by {
            LibrarySortBy::ModifiedDate => da.updated_at_ms.cmp(&db.updated_at_ms),
            LibrarySortBy::CreatedDate => da.created_at_ms.cmp(&db.created_at_ms),
            LibrarySortBy::Name => da
                .title
                .to_ascii_lowercase()
                .cmp(&db.title.to_ascii_lowercase()),
        };
        let ordered = if asc { ord } else { ord.reverse() };
        ordered.then_with(|| a.cmp(&b))
    });
    matched
}

fn matching_trash_indices(app: &PaperCraftApp) -> Vec<usize> {
    let query = app.view.library_search.trim().to_ascii_lowercase();
    app.session
        .trashed_notebooks
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            query.is_empty() || entry.doc.title.to_ascii_lowercase().contains(&query)
        })
        .map(|(idx, _)| idx)
        .collect()
}

fn scope_badge_text(app: &PaperCraftApp, info: &SectionHeaderInfo<'_>) -> String {
    if info.searching {
        return format!("{} matching", info.shown_count);
    }
    match info.scope {
        LibraryScope::All => plural(app.session.notebooks.len(), "notebook", "notebooks"),
        LibraryScope::Favorites => plural(info.shown_count, "favorite", "favorites"),
        LibraryScope::Unfiled => plural(info.shown_count, "unfiled", "unfiled"),
        LibraryScope::Trash => plural(app.session.trashed_notebooks.len(), "in trash", "in trash"),
        LibraryScope::Tag(_) => plural(info.shown_count, "notebook", "notebooks"),
        LibraryScope::Folder(path) => {
            if info.subfolder_count == 0 {
                plural(info.shown_count, "notebook", "notebooks")
            } else if let Some(node) = info
                .tree
                .iter()
                .find(|node| node.path.eq_ignore_ascii_case(path))
            {
                node.summary()
            } else {
                plural(info.shown_count, "notebook", "notebooks")
            }
        }
    }
}

#[derive(Clone, Debug)]
enum SectionHeaderButton {
    NewNotebook { tip: &'static str, can_create: bool },
    Subfolder { path: String, can_nest: bool },
    RenameFolder(String),
    ColorFolder(String),
    ColorTag(String),
    DeleteFolder(String),
}

fn section_header_buttons(
    app: &PaperCraftApp,
    info: &SectionHeaderInfo<'_>,
) -> Vec<SectionHeaderButton> {
    let can_create = app.session.notebooks.len() < papercraft_engine::MAX_NOTEBOOKS;
    let new_notebook = |tip: &'static str| SectionHeaderButton::NewNotebook { tip, can_create };
    match info.scope {
        LibraryScope::Folder(path) => {
            let can_nest =
                folder_depth(path) < MAX_FOLDER_DEPTH && info.tree.len() < MAX_LIBRARY_FOLDERS;
            vec![
                new_notebook("Create a new notebook inside this folder"),
                SectionHeaderButton::Subfolder {
                    path: path.clone(),
                    can_nest,
                },
                SectionHeaderButton::RenameFolder(path.clone()),
                SectionHeaderButton::ColorFolder(path.clone()),
                SectionHeaderButton::DeleteFolder(path.clone()),
            ]
        }
        LibraryScope::Tag(tag) => vec![
            new_notebook("Create a new notebook that already carries this tag"),
            SectionHeaderButton::ColorTag(tag.clone()),
        ],
        LibraryScope::Unfiled => vec![new_notebook("Create a new unfiled notebook")],
        LibraryScope::All => vec![new_notebook("Create a new notebook")],
        LibraryScope::Favorites => vec![new_notebook(
            "Create a new notebook that is already marked as a favorite",
        )],
        LibraryScope::Trash => Vec::new(),
    }
}

fn draw_section_header_button(
    ui: &mut egui::Ui,
    button: &SectionHeaderButton,
    t: &Tokens,
    actions: &mut Vec<LibraryAction>,
    open_new_notebook: &mut bool,
) {
    match button {
        SectionHeaderButton::NewNotebook { tip, can_create } => {
            let tip = if *can_create {
                *tip
            } else {
                "Maximum number of notebooks reached"
            };
            if compact_icon_button(ui, Icon::Plus, Some("New Notebook"), false, *can_create, t)
                .on_hover_text(tip)
                .clicked()
            {
                *open_new_notebook = true;
            }
        }
        SectionHeaderButton::Subfolder { path, can_nest } => {
            if compact_icon_button(ui, Icon::FolderPlus, Some("Subfolder"), false, *can_nest, t)
                .on_hover_text(if *can_nest {
                    "Create a subfolder inside this folder"
                } else {
                    "Folder limit or maximum depth reached"
                })
                .clicked()
            {
                actions.push(LibraryAction::NewFolder {
                    parent: path.clone(),
                    move_notebook: None,
                });
            }
        }
        SectionHeaderButton::RenameFolder(path) => {
            if compact_icon_button(ui, Icon::FolderPen, Some("Rename"), false, true, t)
                .on_hover_text("Rename or move this folder")
                .clicked()
            {
                actions.push(LibraryAction::RenameFolder(path.clone()));
            }
        }
        SectionHeaderButton::ColorFolder(path) => {
            if compact_icon_button(ui, Icon::Palette, Some("Color"), false, true, t)
                .on_hover_text("Choose a color for this folder")
                .clicked()
            {
                actions.push(LibraryAction::OpenFolderColor(path.clone()));
            }
        }
        SectionHeaderButton::ColorTag(tag) => {
            if compact_icon_button(ui, Icon::Palette, Some("Color"), false, true, t)
                .on_hover_text("Choose a color for this tag")
                .clicked()
            {
                actions.push(LibraryAction::OpenTagColor(tag.clone()));
            }
        }
        SectionHeaderButton::DeleteFolder(path) => {
            if compact_icon_button(ui, Icon::Trash, None, false, true, t)
                .on_hover_text("Delete folder (notebooks inside stay safe)")
                .clicked()
            {
                actions.push(LibraryAction::DeleteFolder(path.clone()));
            }
        }
    }
}

fn draw_section_header_title(
    ui: &mut egui::Ui,
    scope: &LibraryScope,
    badge_text: &str,
    folder_color: Option<papercraft_color::Color>,
    t: &Tokens,
) {
    let vis = folder_visual_colors(folder_color, t);
    let (icon_box, _) = ui.allocate_exact_size(Vec2::splat(28.0), Sense::hover());
    ui.painter()
        .rect_filled(icon_box, CornerRadius::same(8), vis.soft_fill);
    paint_icon(ui.painter(), icon_box.shrink(6.0), scope.icon(), vis.icon);

    ui.label(
        RichText::new(scope.title())
            .size(20.0)
            .strong()
            .color(t.text),
    );

    egui::Frame::NONE
        .fill(vis.soft_fill)
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::symmetric(9, 3))
        .show(ui, |ui| {
            ui.label(
                RichText::new(badge_text)
                    .size(11.0)
                    .strong()
                    .color(vis.selected_text),
            );
        });
}

fn draw_library_section_header(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    info: &SectionHeaderInfo<'_>,
    t: &Tokens,
) {
    let mut actions = Vec::new();
    let mut open_new_notebook = false;
    let scope_color = match info.scope {
        LibraryScope::Folder(path) => {
            draw_breadcrumb(ui, path, t, &mut actions);
            ui.add_space(4.0);
            app.session.prefs.folder_color(path)
        }
        LibraryScope::Tag(tag) => app.session.prefs.tag_color(tag),
        _ => None,
    };

    let badge_text = scope_badge_text(app, info);
    let buttons = section_header_buttons(app, info);
    let wide_row = ui.available_width() >= 620.0;

    if wide_row || buttons.is_empty() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = Vec2::new(10.0, 4.0);
            draw_section_header_title(ui, info.scope, &badge_text, scope_color, t);
            if !buttons.is_empty() {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.spacing_mut().item_spacing.x = 6.0;
                    for button in buttons.iter().rev() {
                        draw_section_header_button(
                            ui,
                            button,
                            t,
                            &mut actions,
                            &mut open_new_notebook,
                        );
                    }
                });
            }
        });
    } else {
        crate::theme::studio_wrapped_row(ui, Vec2::new(10.0, 4.0), 0.0, |ui| {
            draw_section_header_title(ui, info.scope, &badge_text, scope_color, t);
        });
        ui.add_space(6.0);
        crate::theme::studio_wrapped_row(ui, Vec2::new(6.0, 4.0), 28.0, |ui| {
            for button in &buttons {
                draw_section_header_button(ui, button, t, &mut actions, &mut open_new_notebook);
            }
        });
    }

    ui.add_space(4.0);
    ui.add(
        egui::Label::new(
            RichText::new(info.scope.description())
                .size(12.0)
                .color(t.text_muted),
        )
        .wrap(),
    );

    if open_new_notebook {
        modal::open(app, ActiveModal::NewNotebook);
    }
    for action in actions {
        apply_library_action(app, action);
    }
}

fn draw_library_utility_bar(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    show_sidebar: bool,
    show_trash: bool,
    searching: bool,
    t: &Tokens,
) {
    let draw_left = |app: &mut PaperCraftApp, ui: &mut egui::Ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if !show_sidebar {
            draw_browse_button(app, ui, t);
        }
        if searching {
            let query = app.view.library_search.trim().to_string();
            if compact_icon_button(
                ui,
                Icon::Close,
                Some(&format!("Clear \"{query}\"")),
                false,
                true,
                t,
            )
            .on_hover_text("Clear search text")
            .clicked()
            {
                app.view.library_search.clear();
            }
        }
    };

    if ui.available_width() >= 540.0 {
        ui.horizontal(|ui| {
            ui.set_height(HEADER_ROW_HEIGHT);
            draw_left(app, ui);
            if !show_trash {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    draw_library_view_controls_rtl(app, ui, t);
                });
            }
        });
    } else {
        crate::theme::studio_wrapped_row(ui, Vec2::new(6.0, 4.0), 0.0, |ui| {
            draw_left(app, ui);
        });
        if !show_trash {
            ui.add_space(6.0);
            crate::theme::studio_wrapped_row(ui, Vec2::new(6.0, 4.0), 0.0, |ui| {
                draw_library_view_controls_ltr(app, ui, t);
            });
        }
    }
}

fn draw_library_view_switch(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let view_mode = app.session.prefs.library_view_mode;
    let active = match view_mode {
        LibraryViewMode::Grid => 0,
        LibraryViewMode::List => 1,
    };
    let options = [
        SwitchOption::new(
            Some(Icon::GridView),
            Some("Grid"),
            "Switch to visual book grid layout",
            true,
        ),
        SwitchOption::new(
            Some(Icon::ListView),
            Some("List"),
            "Switch to detailed list layout",
            true,
        ),
    ];
    let (clicked, _) = segmented_switch(
        ui,
        "home_library_view_switch",
        &options,
        Some(active),
        SwitchLayout::Compact,
        t,
    );
    if let Some(idx) = clicked {
        let next = if idx == 0 { "grid" } else { "list" };
        let _ = app
            .session
            .execute("library.setViewPrefs", &json!({ "view_mode": next }));
    }
}

fn draw_library_sort_dir_switch(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let sort_by = app.session.prefs.library_sort_by;
    let asc = app.session.prefs.library_sort_ascending;
    let (labels, active, asc_for_idx) = if sort_by == LibrarySortBy::Name {
        (["A-Z", "Z-A"], usize::from(!asc), [true, false])
    } else {
        (["Newest", "Oldest"], usize::from(asc), [false, true])
    };
    let options = [
        SwitchOption::new(None, Some(labels[0]), "Sort ascending / newest first", true),
        SwitchOption::new(
            None,
            Some(labels[1]),
            "Sort descending / oldest first",
            true,
        ),
    ];
    let (clicked, _) = segmented_switch(
        ui,
        "home_library_sort_dir_switch",
        &options,
        Some(active),
        SwitchLayout::Compact,
        t,
    );
    if let Some(idx) = clicked {
        let next_asc = asc_for_idx[idx];
        if next_asc != asc {
            let _ = app.session.execute(
                "library.setViewPrefs",
                &json!({ "sort_ascending": next_asc }),
            );
        }
    }
}

fn draw_library_sort_combo(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let sort_by = app.session.prefs.library_sort_by;
    let mut selected_sort = sort_by;
    crate::theme::studio_combo_box(
        ui,
        "home_sort_by_combo",
        RichText::new(sort_by.label()).size(11.8).color(t.text),
        128.0,
        |ui| {
            for &opt in LibrarySortBy::ALL {
                ui.selectable_value(&mut selected_sort, opt, opt.label());
            }
        },
    )
    .response
    .on_hover_text("Choose how notebooks are sorted");
    if selected_sort != sort_by {
        let default_asc = selected_sort == LibrarySortBy::Name;
        let _ = app.session.execute(
            "library.setViewPrefs",
            &json!({ "sort_by": selected_sort, "sort_ascending": default_asc }),
        );
    }
}

fn draw_library_view_controls_rtl(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    ui.spacing_mut().item_spacing.x = 6.0;
    draw_library_view_switch(app, ui, t);
    draw_library_sort_dir_switch(app, ui, t);
    draw_library_sort_combo(app, ui, t);
}

fn draw_library_view_controls_ltr(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    ui.spacing_mut().item_spacing = Vec2::new(6.0, 4.0);
    draw_library_sort_combo(app, ui, t);
    draw_library_sort_dir_switch(app, ui, t);
    draw_library_view_switch(app, ui, t);
}

fn draw_notebook_list(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    slots: &[GridSlot],
    folder_tree: &[FolderNode],
    list_w: f32,
    t: &Tokens,
) -> (Vec<Rect>, Option<Rect>, Rect) {
    let mut rects = Vec::new();
    let mut create_rect = None;
    for &slot in slots {
        match slot {
            GridSlot::Notebook(idx) => {
                rects.push(draw_notebook_list_row(app, ui, idx, list_w, folder_tree, t));
            }
            GridSlot::CreateNew => {
                create_rect = Some(draw_create_notebook_list_row(app, ui, list_w, t));
            }
            GridSlot::Trash(_) => {}
        }
        ui.add_space(8.0);
    }
    (rects, create_rect, Rect::NOTHING)
}

#[derive(Default)]
struct CardInteraction {
    open: bool,
    settings: bool,
    export: bool,
    favorite: bool,
    transfer_to_cloud_move: Option<bool>,
    deferred: Vec<LibraryAction>,
}

struct NotebookMenuCtx<'a> {
    idx: usize,
    doc: &'a Document,
    folder_tree: &'a [FolderNode],
    can_transfer_to_cloud: bool,
}

fn draw_notebook_context_menu(
    ui: &mut egui::Ui,
    menu_ctx: NotebookMenuCtx<'_>,
    t: &Tokens,
    out: &mut CardInteraction,
) {
    if menu_item(ui, Icon::Notebook, "Open Notebook", true, false, t).clicked() {
        out.open = true;
    }
    if menu_item(ui, Icon::Settings, "Notebook Settings...", true, false, t).clicked() {
        out.settings = true;
    }
    if menu_item(ui, Icon::Export, "Export Notebook...", true, false, t).clicked() {
        out.export = true;
    }
    let fav_label = if menu_ctx.doc.favorite {
        "Remove Favorite"
    } else {
        "Mark Favorite"
    };
    if menu_item(ui, Icon::Star, fav_label, true, false, t).clicked() {
        out.favorite = true;
    }
    ui.add_space(4.0);
    move_to_folder_menu_contents(
        ui,
        menu_ctx.idx,
        &menu_ctx.doc.folder_path,
        menu_ctx.folder_tree,
        t,
        &mut out.deferred,
    );
    if menu_ctx.can_transfer_to_cloud && !menu_ctx.doc.id.is_empty() {
        ui.add_space(4.0);
        if menu_item(ui, Icon::Duplicate, "Copy to Cloud", true, false, t).clicked() {
            out.transfer_to_cloud_move = Some(false);
        }
        if menu_item(ui, Icon::Transfer, "Move to Cloud", true, false, t).clicked() {
            out.transfer_to_cloud_move = Some(true);
        }
    }
    ui.add_space(4.0);
    if menu_item(ui, Icon::Trash, "Move to Trash...", true, true, t).clicked() {
        out.deferred
            .push(LibraryAction::TrashNotebook(menu_ctx.idx));
    }
}

fn apply_notebook_card_interaction(
    app: &mut PaperCraftApp,
    idx: usize,
    doc: &Document,
    action: CardInteraction,
) {
    if action.favorite {
        let _ = app.session.execute(
            "notebook.setMetadata",
            &json!({ "index": idx, "favorite": !doc.favorite }),
        );
    } else if action.settings {
        if app
            .session
            .execute("notebook.select", &json!({ "index": idx }))
            .is_ok()
        {
            modal::open(app, ActiveModal::NotebookSettings);
        }
    } else if action.export {
        if app
            .session
            .execute("notebook.select", &json!({ "index": idx }))
            .is_ok()
        {
            app.view.export_target_pages = None;
            app.view.export_format = crate::FileDialogKind::Pcraft;
            modal::open(app, ActiveModal::Export);
        }
    } else if action.open {
        crate::open_notebook_in_editor(app, idx);
    }
    if let Some(move_notebook) = action.transfer_to_cloud_move
        && let Some(prof_id) = app
            .session
            .active_local_profile()
            .map(|p| p.local_profile_id.clone())
        && !doc.id.is_empty()
    {
        let _ = app.session.execute(
            "sync.transferNotebook",
            &json!({
                "source_profile_id": prof_id,
                "notebook_id": doc.id,
                "move_notebook": move_notebook,
            }),
        );
        crate::sync_view_to_active_notebook(app);
    }
    for deferred in action.deferred {
        apply_library_action(app, deferred);
    }
}

fn paint_notebook_sync_corner_badge(
    ui: &egui::Ui,
    well_rect: Rect,
    status: EntitySyncStatus,
    t: &Tokens,
) {
    if !ui.is_rect_visible(well_rect) {
        return;
    }
    let (label, fg, fill, border) = crate::account_modal::notebook_status_badge_style(status, t);
    let p = ui.painter();
    let galley = p.layout_no_wrap(label.to_string(), egui::FontId::proportional(9.5), fg);
    let pill_w =
        (8.0 + 5.0 + 4.0 + galley.size().x + 7.0).min((well_rect.width() - 16.0).max(40.0));
    let pill_h = 18.0_f32;
    let pill_rect = Rect::from_min_size(
        pos2(well_rect.right() - pill_w - 7.0, well_rect.top() + 7.0),
        Vec2::new(pill_w, pill_h),
    );
    p.rect(
        pill_rect,
        CornerRadius::same(9),
        fill.gamma_multiply(0.92),
        Stroke::new(1.0, border),
        StrokeKind::Inside,
    );
    p.circle_filled(pos2(pill_rect.left() + 8.5, pill_rect.center().y), 2.5, fg);
    p.galley(
        pos2(
            pill_rect.left() + 14.5,
            pill_rect.center().y - galley.size().y * 0.5,
        ),
        galley,
        fg,
    );
}

fn draw_notebook_sync_pill(ui: &mut egui::Ui, status: EntitySyncStatus, t: &Tokens) {
    let (label, fg, fill, border) = crate::account_modal::notebook_status_badge_style(status, t);
    egui::Frame::NONE
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(9))
        .inner_margin(Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                let (dr, _) = ui.allocate_exact_size(Vec2::splat(6.0), Sense::hover());
                ui.painter().circle_filled(dr.center(), 2.5, fg);
                ui.label(RichText::new(label).size(10.0).strong().color(fg));
            });
        });
}

fn rect_hover_factor(ctx: &egui::Context, id: egui::Id) -> (bool, f32) {
    let was_hovered = ctx.data(|d| d.get_temp::<bool>(id).unwrap_or(false));
    let hover_t = crate::motion::hover_factor(ctx, id.with("t"), was_hovered);
    (was_hovered, hover_t)
}

fn update_rect_hover(ui: &egui::Ui, id: egui::Id, rect: Rect, was_hovered: bool) {
    let hovered_now = ui.rect_contains_pointer(rect);
    if hovered_now != was_hovered {
        ui.ctx().data_mut(|d| d.insert_temp(id, hovered_now));
    }
}

fn folder_icon_color_for_path(folder_tree: &[FolderNode], path: &str, t: &Tokens) -> Color32 {
    let custom = folder_tree
        .iter()
        .find(|n| n.path.eq_ignore_ascii_case(path))
        .and_then(|n| n.color);
    folder_visual_colors(custom, t).icon
}

fn draw_notebook_list_row(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    idx: usize,
    list_w: f32,
    folder_tree: &[FolderNode],
    t: &Tokens,
) -> Rect {
    let Some(doc) = app.session.notebooks.get(idx).cloned() else {
        return Rect::NOTHING;
    };
    let status = app.session.notebook_sync_status(idx);
    let can_transfer_to_cloud = !app.session.is_viewing_cloud() && app.session.is_signed_in();
    let hover_id = ui.make_persistent_id(("papercraft_home_list_hover", idx));
    let (was_hovered, hover_t) = rect_hover_factor(ui.ctx(), hover_id);
    let row_h = 64.0_f32;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(list_w, row_h), Sense::hover());
    let border = Stroke::new(
        crate::motion::lerp_f32(1.0, 1.4, hover_t),
        crate::motion::lerp_color(t.border, t.accent, hover_t),
    );
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(10), t.field_bg);
        p.rect_stroke(rect, CornerRadius::same(10), border, StrokeKind::Inside);
    }

    let inner = rect.shrink2(Vec2::new(12.0, 8.0));
    let mut interaction = CardInteraction::default();

    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(egui::Layout::left_to_right(egui::Align::Center)),
    );
    child.spacing_mut().item_spacing.x = 10.0;

    let (cover_rect, _) = child.allocate_exact_size(Vec2::new(30.0, 42.0), Sense::hover());
    paint_hardcover_book_cover(child.painter(), cover_rect, doc.cover, None, t.border);

    let info_w = (inner.width() - 300.0).max(120.0);
    let mut info_resp = child
        .allocate_ui_with_layout(
            Vec2::new(info_w, 46.0),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                ui.set_width(info_w);
                ui.horizontal(|ui| {
                    ui.set_min_height(20.0);
                    crate::theme::studio_row_label(
                        ui,
                        RichText::new(&doc.title)
                            .size(13.5)
                            .strong()
                            .color(crate::motion::lerp_color(t.text, t.accent, hover_t)),
                        20.0,
                    );
                    if !doc.folder_path.is_empty() {
                        egui::Frame::NONE
                            .fill(t.panel_bg)
                            .stroke(Stroke::new(1.0, t.border))
                            .corner_radius(CornerRadius::same(5))
                            .inner_margin(Margin::symmetric(6, 2))
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    let (ir, _) =
                                        ui.allocate_exact_size(Vec2::splat(11.0), Sense::hover());
                                    paint_icon(
                                        ui.painter(),
                                        ir,
                                        Icon::Folder,
                                        folder_icon_color_for_path(
                                            folder_tree,
                                            &doc.folder_path,
                                            t,
                                        ),
                                    );
                                    ui.label(
                                        RichText::new(display_folder_path(&doc.folder_path))
                                            .size(10.0)
                                            .color(t.text_muted),
                                    );
                                });
                            });
                    }
                    for tag in doc.tags.iter().take(3) {
                        let vis = folder_visual_colors(app.session.prefs.tag_color(tag), t);
                        egui::Frame::NONE
                            .fill(vis.soft_fill)
                            .corner_radius(CornerRadius::same(5))
                            .inner_margin(Margin::symmetric(5, 2))
                            .show(ui, |ui| {
                                ui.label(
                                    RichText::new(format!("#{tag}"))
                                        .size(10.0)
                                        .color(vis.selected_text),
                                );
                            });
                    }
                });
                let page_count = doc.page_count();
                let total_strokes = doc.total_stroke_count();
                ui.label(
                    RichText::new(format!(
                        "{} {}  •  {}  •  {} strokes",
                        page_count,
                        if page_count == 1 { "page" } else { "pages" },
                        doc.paper_template.label(),
                        total_strokes
                    ))
                    .size(11.0)
                    .color(t.text_muted),
                );
            },
        )
        .response
        .interact(Sense::click_and_drag())
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if !egui::DragAndDrop::has_any_payload(ui.ctx()) {
        info_resp = info_resp.on_hover_text(format!(
            "Open \"{}\" — or drag onto a folder to move",
            doc.title
        ));
    }
    if info_resp.drag_started() {
        info_resp.dnd_set_drag_payload(NotebookDragPayload { index: idx });
    }
    if info_resp.clicked() {
        interaction.open = true;
    }
    egui::Popup::context_menu(&info_resp)
        .width(230.0)
        .show(|ui| {
            draw_notebook_context_menu(
                ui,
                NotebookMenuCtx {
                    idx,
                    doc: &doc,
                    folder_tree,
                    can_transfer_to_cloud,
                },
                t,
                &mut interaction,
            );
        });

    child.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        if icon_button(ui, Icon::Settings, None, false, true, t)
            .on_hover_text("Notebook settings: rename, folder, tags, cover, or delete")
            .clicked()
        {
            interaction.settings = true;
        }
        if icon_button(ui, Icon::Star, None, doc.favorite, true, t)
            .on_hover_text(if doc.favorite {
                "Remove from Favorites"
            } else {
                "Mark as Favorite"
            })
            .clicked()
        {
            interaction.favorite = true;
        }
        let move_resp = icon_button(ui, Icon::MoveToFolder, None, false, true, t)
            .on_hover_text("Move notebook to a folder");
        egui::Popup::menu(&move_resp).width(230.0).show(|ui| {
            move_to_folder_menu_contents(
                ui,
                idx,
                &doc.folder_path,
                folder_tree,
                t,
                &mut interaction.deferred,
            );
        });
        if draw_card_open_pill(ui, hover_t, t).clicked() {
            interaction.open = true;
        }
        draw_notebook_sync_pill(ui, status, t);
    });

    update_rect_hover(ui, hover_id, rect, was_hovered);
    apply_notebook_card_interaction(app, idx, &doc, interaction);
    rect
}

fn draw_create_notebook_list_row(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    list_w: f32,
    t: &Tokens,
) -> Rect {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(list_w, 46.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let hover_t = crate::motion::hover_factor(ui.ctx(), resp.id.with("hov"), resp.hovered());
        let p = ui.painter();
        p.rect_filled(
            rect,
            CornerRadius::same(10),
            crate::motion::lerp_color(t.panel_bg, t.accent_soft, hover_t),
        );
        p.rect_stroke(
            rect,
            CornerRadius::same(10),
            Stroke::new(
                crate::motion::lerp_f32(1.0, 1.4, hover_t),
                crate::motion::lerp_color(t.border, t.accent, hover_t),
            ),
            StrokeKind::Inside,
        );
        let ir =
            Rect::from_center_size(pos2(rect.left() + 24.0, rect.center().y), Vec2::splat(16.0));
        paint_icon(p, ir, Icon::Plus, t.accent);
        let g = p.layout_no_wrap(
            "Create New Notebook...".to_string(),
            egui::FontId::proportional(13.0),
            t.accent,
        );
        p.galley(
            pos2(ir.right() + 10.0, rect.center().y - g.size().y * 0.5),
            g,
            t.accent,
        );
    }
    if resp
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        modal::open(app, ActiveModal::NewNotebook);
    }
    rect
}

fn draw_empty_state_card(
    ui: &mut egui::Ui,
    icon: Icon,
    title: &str,
    description: &str,
    t: &Tokens,
) {
    egui::Frame::NONE
        .fill(t.field_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(12))
        .inner_margin(Margin::same(24))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical_centered(|ui| {
                let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
                ui.painter().rect_filled(icon_rect, 10.0, t.accent_soft);
                paint_icon(ui.painter(), icon_rect.shrink(8.0), icon, t.accent);
                ui.add_space(10.0);
                ui.label(RichText::new(title).size(15.0).strong().color(t.text));
                ui.add_space(4.0);
                ui.add(
                    egui::Label::new(RichText::new(description).size(12.0).color(t.text_muted))
                        .wrap(),
                );
            });
        });
}

fn draw_empty_search_state(
    app: &PaperCraftApp,
    ui: &mut egui::Ui,
    scope: &LibraryScope,
    t: &Tokens,
) {
    let query = app.view.library_search.trim();
    let title = format!("No notebooks match \"{query}\"");
    let description = if *scope != LibraryScope::All {
        "Try another search term, clear the search, or check All Notebooks."
    } else {
        "Try another search term, clear the search, or start a fresh notebook."
    };
    draw_empty_state_card(ui, Icon::Search, &title, description, t);
}

fn draw_empty_scope_state(ui: &mut egui::Ui, scope: &LibraryScope, t: &Tokens) {
    let (title, description) = match scope {
        LibraryScope::Favorites => (
            "No favorite notebooks yet".to_string(),
            "Click the star on any notebook card, or drag a notebook onto Favorites in the sidebar."
                .to_string(),
        ),
        LibraryScope::Unfiled => (
            "Every notebook is filed in a folder".to_string(),
            "All your notebooks are organized into folders. Create a new unfiled notebook or browse All Notebooks."
                .to_string(),
        ),
        LibraryScope::Tag(tag) => (
            format!("No notebooks tagged #{tag}"),
            "Open a notebook's Settings to attach this tag and group related notes.".to_string(),
        ),
        LibraryScope::Folder(path) => (
            format!("\"{}\" is empty", folder_leaf(path)),
            "Create a notebook here, add a subfolder, or drag an existing notebook onto this folder in the sidebar."
                .to_string(),
        ),
        LibraryScope::All | LibraryScope::Trash => return,
    };
    draw_empty_state_card(ui, scope.icon(), &title, &description, t);
}

fn draw_empty_trash_state(ui: &mut egui::Ui, filtering: bool, t: &Tokens) {
    let title = if filtering {
        "No deleted notebooks match this search"
    } else {
        "Trash is empty"
    };
    let description = if filtering {
        "Try another title or clear the search to see every item in Trash."
    } else {
        "Notebooks you delete will stay here for 30 days, so you can restore them."
    };
    draw_empty_state_card(ui, Icon::Trash, title, description, t);
}

fn split_card_body_and_footer(ui: &egui::Ui, card_rect: Rect, t: &Tokens) -> (Rect, Rect) {
    let inner_rect = card_rect.shrink(f32::from(CARD_MARGIN));
    let actions_h = 28.0_f32;
    let footer_gap = 8.0_f32;
    let actions_rect = Rect::from_min_max(
        pos2(inner_rect.left(), inner_rect.bottom() - actions_h),
        inner_rect.right_bottom(),
    );
    let sep_y = actions_rect.top() - footer_gap - 0.5;
    if ui.is_rect_visible(card_rect) {
        ui.painter()
            .hline(inner_rect.x_range(), sep_y, Stroke::new(1.0, t.border));
    }
    let top_rect = Rect::from_min_max(
        inner_rect.left_top(),
        pos2(
            inner_rect.right(),
            (sep_y - footer_gap).max(inner_rect.top()),
        ),
    );
    (top_rect, actions_rect)
}

fn draw_trash_card(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    idx: usize,
    card_w: f32,
    t: &Tokens,
) -> Rect {
    let Some(entry) = app.session.trashed_notebooks.get(idx).cloned() else {
        return Rect::NOTHING;
    };
    let doc = Arc::clone(&entry.doc);
    let hover_id = ui.make_persistent_id(("papercraft_home_trash_hover", idx));
    let (was_hovered, hover_t) = rect_hover_factor(ui.ctx(), hover_id);
    let inner_w = (card_w - f32::from(CARD_MARGIN) * 2.0).max(120.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(card_w, CARD_HEIGHT), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(12), t.field_bg);
        painter.rect_stroke(
            rect,
            CornerRadius::same(12),
            Stroke::new(
                crate::motion::lerp_f32(1.0, 1.4, hover_t),
                crate::motion::lerp_color(t.border, t.text_muted.gamma_multiply(0.6), hover_t),
            ),
            StrokeKind::Inside,
        );
    }

    let (top_rect, actions_rect) = split_card_body_and_footer(ui, rect, t);
    {
        let mut body_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(top_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        body_ui.set_width(inner_w);
        body_ui.spacing_mut().item_spacing.y = 0.0;
        let (well_rect, _) = body_ui.allocate_exact_size(Vec2::new(inner_w, 116.0), Sense::hover());
        paint_notebook_card_showcase(
            &body_ui,
            well_rect,
            &doc,
            0.0,
            app.session.prefs.theme,
            app.session.prefs.auto_contrast_with_theme,
            t,
        );
        body_ui.add_space(8.0);
        draw_card_title_and_meta(&mut body_ui, inner_w, &doc, 0.0, t.text_muted, t);
        body_ui.add_space(4.0);
        body_ui.label(
            RichText::new(trash_expiration_label(entry.deleted_at_ms))
                .size(10.5)
                .color(t.text_muted),
        );
    }

    let mut restore_clicked = false;
    {
        let mut footer_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(actions_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        if icon_button(&mut footer_ui, Icon::Undo, Some("Restore"), true, true, t)
            .on_hover_text("Restore this notebook to your library")
            .clicked()
        {
            restore_clicked = true;
        }
    }

    if restore_clicked
        && app
            .session
            .execute("notebook.restore", &json!({ "index": idx }))
            .is_ok()
    {
        app.view.library_show_trash = false;
        crate::sync_view_to_active_notebook(app);
    }
    update_rect_hover(ui, hover_id, rect, was_hovered);
    rect
}

fn trash_expiration_label(deleted_at_ms: i64) -> String {
    const DAY_MS: i64 = 24 * 60 * 60 * 1000;
    const RETENTION_MS: i64 = 30 * DAY_MS;
    let now_ms = papercraft_sync_protocol::now_unix_ms();
    let elapsed_ms = now_ms.saturating_sub(deleted_at_ms).max(0);
    let days_left = RETENTION_MS
        .saturating_sub(elapsed_ms)
        .saturating_add(DAY_MS - 1)
        / DAY_MS;
    if days_left <= 0 {
        "Expires today".to_string()
    } else if days_left == 1 {
        "1 day left to restore".to_string()
    } else {
        format!("{days_left} days left to restore")
    }
}

fn draw_notebook_card(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    idx: usize,
    card_w: f32,
    folder_tree: &[FolderNode],
    t: &Tokens,
) -> (Rect, Rect) {
    let Some(doc) = app.session.notebooks.get(idx).cloned() else {
        return (Rect::NOTHING, Rect::NOTHING);
    };
    let status = app.session.notebook_sync_status(idx);
    let can_transfer_to_cloud = !app.session.is_viewing_cloud() && app.session.is_signed_in();
    let (status_label, _, _, _) = crate::account_modal::notebook_status_badge_style(status, t);
    let is_last_open = idx == app.session.active_notebook;
    let hover_id = ui.make_persistent_id(("papercraft_home_card_hover", idx));
    let (was_hovered, hover_t) = rect_hover_factor(ui.ctx(), hover_id);

    let border = Stroke::new(
        crate::motion::lerp_f32(1.0, 1.5, hover_t),
        crate::motion::lerp_color(t.border, t.accent, hover_t),
    );
    let mut interaction = CardInteraction::default();
    let inner_w = (card_w - f32::from(CARD_MARGIN) * 2.0).max(120.0);

    let (rect, _) = ui.allocate_exact_size(Vec2::new(card_w, CARD_HEIGHT), Sense::hover());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(12), t.field_bg);
        p.rect_stroke(rect, CornerRadius::same(12), border, StrokeKind::Inside);
    }

    let (top_rect, actions_rect) = split_card_body_and_footer(ui, rect, t);
    {
        let mut body_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(top_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        body_ui.set_width(inner_w);
        body_ui.spacing_mut().item_spacing.y = 0.0;

        let (well_rect, _) = body_ui.allocate_exact_size(Vec2::new(inner_w, 134.0), Sense::hover());
        paint_notebook_card_showcase(
            &body_ui,
            well_rect,
            &doc,
            hover_t,
            app.session.prefs.theme,
            app.session.prefs.auto_contrast_with_theme,
            t,
        );
        paint_notebook_sync_corner_badge(&body_ui, well_rect, status, t);

        body_ui.add_space(10.0);
        let folder_icon_color = folder_icon_color_for_path(folder_tree, &doc.folder_path, t);
        draw_card_title_and_meta(&mut body_ui, inner_w, &doc, hover_t, folder_icon_color, t);
    }
    let mut open_region = ui
        .interact(
            top_rect,
            ui.make_persistent_id(("papercraft_home_card_open", idx)),
            Sense::click_and_drag(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if !egui::DragAndDrop::has_any_payload(ui.ctx()) {
        open_region = open_region.on_hover_text(format!(
            "Open \"{}\" — or drag onto a folder to move\nSync status: {status_label}",
            doc.title
        ));
    }
    if open_region.drag_started() {
        open_region.dnd_set_drag_payload(NotebookDragPayload { index: idx });
    }
    if open_region.clicked() {
        interaction.open = true;
    }
    egui::Popup::context_menu(&open_region)
        .width(230.0)
        .show(|ui| {
            draw_notebook_context_menu(
                ui,
                NotebookMenuCtx {
                    idx,
                    doc: &doc,
                    folder_tree,
                    can_transfer_to_cloud,
                },
                t,
                &mut interaction,
            );
        });

    {
        let mut footer_ui = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(actions_rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        footer_ui.spacing_mut().item_spacing.x = 6.0;
        if draw_card_open_pill(&mut footer_ui, hover_t, t).clicked() {
            interaction.open = true;
        }
        if is_last_open {
            egui::Frame::NONE
                .fill(t.accent_soft)
                .corner_radius(CornerRadius::same(5))
                .inner_margin(Margin::symmetric(6, 3))
                .show(&mut footer_ui, |ui| {
                    ui.label(RichText::new("Recent").size(10.0).strong().color(t.accent));
                });
        }

        footer_ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            if icon_button(ui, Icon::Settings, None, false, true, t)
                .on_hover_text("Notebook settings: rename, folder, tags, cover, or delete")
                .clicked()
            {
                interaction.settings = true;
            }
            if icon_button(ui, Icon::Star, None, doc.favorite, true, t)
                .on_hover_text(if doc.favorite {
                    "Remove from Favorites"
                } else {
                    "Mark as Favorite"
                })
                .clicked()
            {
                interaction.favorite = true;
            }
            let move_resp = icon_button(ui, Icon::MoveToFolder, None, false, true, t)
                .on_hover_text("Move notebook to a folder");
            egui::Popup::menu(&move_resp).width(230.0).show(|ui| {
                move_to_folder_menu_contents(
                    ui,
                    idx,
                    &doc.folder_path,
                    folder_tree,
                    t,
                    &mut interaction.deferred,
                );
            });
        });
    }

    update_rect_hover(ui, hover_id, rect, was_hovered);

    apply_notebook_card_interaction(app, idx, &doc, interaction);

    (rect, actions_rect)
}

fn draw_card_title_and_meta(
    ui: &mut egui::Ui,
    inner_w: f32,
    doc: &Document,
    hover_t: f32,
    folder_icon_color: Color32,
    t: &Tokens,
) {
    let title_color = crate::motion::lerp_color(t.text, t.accent, hover_t);
    let page_count = doc.page_count();
    let total_strokes = doc.total_stroke_count();
    let paper_kind_label = doc
        .with_resolved_page(0, |first_page| {
            if first_page.pdf_backdrop.is_some() {
                "PDF"
            } else {
                first_page.paper_template.label()
            }
        })
        .unwrap_or_else(|| doc.paper_template.label());
    let page_noun = if page_count == 1 { "page" } else { "pages" };
    let stroke_info = match total_strokes {
        0 => "Empty".to_string(),
        1 => "1 stroke".to_string(),
        n => format!("{n} strokes"),
    };

    ui.allocate_ui_with_layout(
        Vec2::new(inner_w, 57.0),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_width(inner_w);
            ui.set_max_width(inner_w);
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.allocate_ui_with_layout(
                Vec2::new(inner_w, 20.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |row| {
                    row.set_max_width(inner_w);
                    row.add(
                        egui::Label::new(
                            RichText::new(&doc.title)
                                .size(14.0)
                                .strong()
                                .color(title_color),
                        )
                        .truncate(),
                    );
                },
            );
            ui.allocate_ui_with_layout(
                Vec2::new(inner_w, 17.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |row| {
                    row.set_max_width(inner_w);
                    row.spacing_mut().item_spacing.x = 4.0;
                    if doc.folder_path.is_empty() {
                        row.add(
                            egui::Label::new(
                                RichText::new(format!("{} cover", doc.cover.display_label()))
                                    .size(10.5)
                                    .color(t.text_muted),
                            )
                            .truncate(),
                        );
                    } else {
                        let (icon_rect, _) =
                            row.allocate_exact_size(Vec2::splat(11.0), Sense::hover());
                        paint_icon(row.painter(), icon_rect, Icon::Folder, folder_icon_color);
                        row.add(
                            egui::Label::new(
                                RichText::new(display_folder_path(&doc.folder_path))
                                    .size(10.5)
                                    .color(t.text_muted),
                            )
                            .truncate(),
                        );
                    }
                },
            );
            ui.allocate_ui_with_layout(
                Vec2::new(inner_w, 16.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |row| {
                    row.set_max_width(inner_w);
                    row.add(
                        egui::Label::new(
                            RichText::new(format!(
                                "{page_count} {page_noun} • {paper_kind_label} • {stroke_info}",
                            ))
                            .size(11.0)
                            .color(t.text_muted),
                        )
                        .truncate(),
                    );
                },
            );
        },
    );
}

fn draw_card_open_pill(ui: &mut egui::Ui, hover_t: f32, t: &Tokens) -> egui::Response {
    let fill = crate::motion::lerp_color(t.accent_soft, t.accent, hover_t);
    let text_color = crate::motion::lerp_color(t.accent, Color32::WHITE, hover_t);
    let btn = egui::Button::new(RichText::new("Open").size(11.5).strong().color(text_color))
        .fill(fill)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(7))
        .min_size(Vec2::new(50.0, 28.0));
    ui.add(btn).on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn draw_create_notebook_card(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    card_w: f32,
    t: &Tokens,
) -> Rect {
    let hover_id = ui.make_persistent_id("papercraft_home_create_card_hover");
    let (was_hovered, hover_t) = rect_hover_factor(ui.ctx(), hover_id);

    let fill = crate::motion::lerp_color(t.toolbar_bg, t.accent_soft.gamma_multiply(0.55), hover_t);
    let stroke = Stroke::new(
        crate::motion::lerp_f32(1.2, 1.5, hover_t),
        crate::motion::lerp_color(t.border, t.accent, hover_t),
    );

    let (rect, _) = ui.allocate_exact_size(Vec2::new(card_w, CARD_HEIGHT), Sense::hover());

    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        p.rect_filled(rect, CornerRadius::same(12), fill);
        p.rect_stroke(rect, CornerRadius::same(12), stroke, StrokeKind::Inside);
    }

    let inner_rect = rect.shrink(f32::from(CARD_MARGIN));
    let title_text = RichText::new("New Notebook")
        .size(14.5)
        .strong()
        .color(crate::motion::lerp_color(t.text, t.accent, hover_t));
    let subtitle_text = RichText::new("Pick a cover, paper ruling, and sheet tint")
        .size(11.5)
        .color(t.text_muted);
    let title_galley = egui::WidgetText::from(title_text.clone()).into_galley(
        ui,
        Some(egui::TextWrapMode::Extend),
        inner_rect.width(),
        egui::TextStyle::Body,
    );
    let sub_galley = egui::WidgetText::from(subtitle_text.clone()).into_galley(
        ui,
        Some(egui::TextWrapMode::Wrap),
        inner_rect.width(),
        egui::TextStyle::Body,
    );
    let badge_d = 44.0_f32;
    let stack_h = badge_d + 12.0 + title_galley.size().y + 6.0 + sub_galley.size().y;
    let top_pad = ((inner_rect.height() - stack_h) * 0.5).max(8.0);

    {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner_rect)
                .layout(egui::Layout::top_down(egui::Align::Center)),
        );
        child.spacing_mut().item_spacing.y = 0.0;
        child.add_space(top_pad);
        let (badge_rect, _) = child.allocate_exact_size(Vec2::splat(badge_d), Sense::hover());
        child.painter().circle_filled(
            badge_rect.center(),
            badge_d * 0.5,
            crate::motion::lerp_color(t.accent_soft, t.accent, hover_t),
        );
        paint_icon(
            child.painter(),
            badge_rect.shrink(11.0),
            Icon::Plus,
            crate::motion::lerp_color(t.accent, Color32::WHITE, hover_t),
        );

        child.add_space(12.0);
        child.label(title_text);
        child.add_space(6.0);
        child.add(egui::Label::new(subtitle_text).wrap());
    }

    let resp = ui
        .interact(
            rect,
            ui.make_persistent_id("papercraft_home_create_card_click"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Create a new multi-page notebook");

    update_rect_hover(ui, hover_id, rect, was_hovered);
    if resp.clicked() {
        modal::open(app, ActiveModal::NewNotebook);
    }
    rect
}

fn draw_home_pdf_error_alert(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    message: &str,
    t: &Tokens,
) {
    let mut dismiss = false;
    egui::Frame::NONE
        .fill(t.danger_bg)
        .stroke(Stroke::new(1.0, t.danger_border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 8.0;
                let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                paint_icon(ui.painter(), icon_rect, Icon::AlertCircle, t.danger_fg);
                ui.add(
                    egui::Label::new(RichText::new(message).size(12.0).color(t.danger_fg)).wrap(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if icon_button(ui, Icon::Close, None, false, true, t)
                        .on_hover_text("Dismiss PDF import error")
                        .clicked()
                    {
                        dismiss = true;
                    }
                });
            });
        });
    if dismiss {
        app.view.pdf_import_error = None;
    }
}

fn paint_notebook_card_showcase(
    ui: &egui::Ui,
    well_rect: Rect,
    doc: &Document,
    hover_t: f32,
    theme: papercraft_doc::UiTheme,
    auto_contrast: bool,
    t: &Tokens,
) {
    let p = ui.painter();
    let well_fill =
        crate::motion::lerp_color(t.toolbar_bg, t.accent_soft.gamma_multiply(0.45), hover_t);
    p.rect_filled(well_rect, CornerRadius::same(8), well_fill);

    let book_h = (well_rect.height() - 26.0).clamp(92.0, 108.0);
    let book_w = (book_h * 0.72).round();
    let sheet_h = (book_h * 0.96).round();
    let sheet_w = book_w;
    let pair_gap = (well_rect.width() * 0.065).clamp(12.0, 22.0).round();
    let total_w = book_w + pair_gap + sheet_w;
    let start_x = well_rect.center().x - total_w * 0.5;
    let book_y = well_rect.center().y - book_h * 0.5;
    let sheet_y = well_rect.center().y - sheet_h * 0.5;

    let cover_rect = Rect::from_min_size(pos2(start_x, book_y), Vec2::new(book_w, book_h));
    paint_hardcover_book_cover(p, cover_rect, doc.cover, Some(&doc.title), t.border);

    let sheet_rect = Rect::from_min_size(
        pos2(start_x + book_w + pair_gap, sheet_y),
        Vec2::new(sheet_w, sheet_h),
    );
    if doc.page_count() > 1 {
        let stack_rect = sheet_rect.translate(Vec2::new(3.0, 3.0));
        p.rect_filled(stack_rect, 4.0, t.field_bg);
        p.rect_stroke(
            stack_rect,
            4.0,
            Stroke::new(1.0, t.border),
            StrokeKind::Inside,
        );
    }
    p.rect_filled(
        sheet_rect.translate(Vec2::new(0.0, 2.0)),
        4.0,
        Color32::from_black_alpha(18),
    );
    doc.with_resolved_page(0, |first_page| {
        let has_backdrop =
            first_page.pdf_backdrop.is_some() || first_page.custom_image_backdrop.is_some();
        let is_pdf = first_page.pdf_backdrop.is_some();
        let eff_contrast = auto_contrast && !is_pdf;
        let eff_paper_color = if is_pdf {
            papercraft_doc::PaperColor::PureWhite
        } else {
            first_page.paper_color
        };
        let eff_tpl = if has_backdrop {
            papercraft_doc::PaperTemplate::Blank
        } else {
            first_page.paper_template
        };
        let paper_colors = eff_paper_color.resolved_colors(theme, eff_contrast);
        let border_color = crate::motion::lerp_color(t.border, t.accent, hover_t);
        paint_paper_preview_colors(p, sheet_rect, eff_tpl, paper_colors, border_color);
        if has_backdrop {
            crate::pdf_view::paint_page_pdf_backdrop_if_present(
                ui.ctx(),
                p,
                sheet_rect,
                doc,
                first_page,
                240,
            );
        }
        paint_thumbnail_strokes_cached(
            ui,
            sheet_rect,
            first_page,
            (doc.created_at_ms, doc.title.as_str()),
            theme,
            eff_contrast,
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Pos2;
    use papercraft_doc::{NotebookPreset, PenSettings, StrokeSample};

    fn run_home_frames(app: &mut PaperCraftApp, size: Vec2, frames: usize) -> HomeMetrics {
        let ctx = egui::Context::default();
        let mut metrics = HomeMetrics {
            header_rect: Rect::NOTHING,
            sidebar_rect: None,
            content_rect: Rect::NOTHING,
            folder_tile_rects: Vec::new(),
            card_rects: Vec::new(),
            create_card_rect: None,
            first_card_actions_rect: Rect::NOTHING,
        };
        for _ in 0..frames {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, size)),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                metrics = show_inner(app, ui);
            });
            out.drop_without_applying_deltas();
        }
        metrics
    }

    #[test]
    fn home_screen_renders_all_notebooks_without_overlap_at_min_and_wide_sizes() {
        let mut app = PaperCraftApp::default();
        for preset in [
            NotebookPreset::LectureCornell,
            NotebookPreset::EngineeringGrid,
            NotebookPreset::SketchBlank,
            NotebookPreset::MidnightSlate,
        ] {
            let _ = app
                .session
                .execute("notebook.create", &json!({ "preset": preset }));
        }
        app.session.commit_raw_stroke(
            PenSettings::fountain_pen(),
            vec![
                StrokeSample::from_xy_pressure(50.0, 50.0, 0.5),
                StrokeSample::from_xy_pressure(240.0, 120.0, 0.8),
            ],
        );

        for size in [Vec2::new(1280.0, 800.0), Vec2::new(840.0, 560.0)] {
            let m = run_home_frames(&mut app, size, 2);
            assert_eq!(m.card_rects.len(), 5);
            assert!(m.create_card_rect.is_some());
            for (i, a) in m.card_rects.iter().enumerate() {
                assert!(a.top() >= m.header_rect.bottom() - 0.5);
                assert!(a.width() >= 170.0 && a.height() >= 230.0);
                for b in m.card_rects.iter().skip(i + 1) {
                    assert!(
                        !a.intersects(*b),
                        "cards {a:?} and {b:?} overlapped at viewport {size:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn search_filter_matches_title_cover_and_paper_template() {
        let mut app = PaperCraftApp::default();
        let _ = app.session.execute(
            "notebook.create",
            &json!({ "preset": "EngineeringGrid", "title": "Physics Lab" }),
        );
        let _ = app.session.execute(
            "notebook.create",
            &json!({ "preset": "LectureCornell", "title": "World History" }),
        );

        assert_eq!(matching_notebook_indices(&app).len(), 3);

        app.view.library_search = "physics".to_string();
        assert_eq!(matching_notebook_indices(&app), vec![1]);

        app.view.library_search = "cornell".to_string();
        assert_eq!(matching_notebook_indices(&app), vec![2]);

        app.view.library_search = "nonexistent-book".to_string();
        assert!(matching_notebook_indices(&app).is_empty());

        let m = run_home_frames(&mut app, Vec2::new(1024.0, 720.0), 2);
        assert!(m.card_rects.is_empty());
        assert!(m.create_card_rect.is_none());
    }

    #[test]
    fn clicking_notebook_card_selects_and_opens_that_notebook_in_editor() {
        let mut app = PaperCraftApp::default();
        let _ = app.session.execute(
            "notebook.create",
            &json!({ "preset": "EngineeringGrid", "title": "Circuit Design" }),
        );
        let _ = app
            .session
            .execute("notebook.select", &json!({ "index": 0 }));
        assert_eq!(app.view.screen, crate::AppScreen::Home);
        assert_eq!(app.session.active_notebook, 0);

        let ctx = egui::Context::default();
        let target_slot = matching_notebook_indices(&app)
            .iter()
            .position(|&idx| idx == 1)
            .expect("second notebook should appear in home list");
        let mut target_card = Rect::NOTHING;
        for _ in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0))),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                let m = show_inner(&mut app, ui);
                target_card = m.card_rects[target_slot];
            });
            out.drop_without_applying_deltas();
        }

        // Click the upper showcase/title area of the "Circuit Design" notebook card.
        let click_pos = pos2(target_card.center().x, target_card.top() + 56.0);
        for (pressed, time) in [(true, 0.1), (false, 0.2)] {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0))),
                time: Some(time),
                events: vec![
                    egui::Event::PointerMoved(click_pos),
                    egui::Event::PointerButton {
                        pos: click_pos,
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

        assert_eq!(app.view.screen, crate::AppScreen::Editor);
        assert_eq!(app.session.active_notebook, 1);
        assert_eq!(app.session.doc.title, "Circuit Design");
    }

    #[test]
    fn toolbar_library_button_returns_from_editor_to_home() {
        let mut app = PaperCraftApp::default();
        crate::open_notebook_in_editor(&mut app, 0);
        assert_eq!(app.view.screen, crate::AppScreen::Editor);

        let ctx = egui::Context::default();
        for _ in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0))),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                crate::toolbar::show(&mut app, ui);
            });
            out.drop_without_applying_deltas();
        }

        // Click the left-most Library button in the editor toolbar.
        let lib_btn_pos = pos2(44.0, 22.0);
        for (pressed, time) in [(true, 0.1), (false, 0.2)] {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0))),
                time: Some(time),
                events: vec![
                    egui::Event::PointerMoved(lib_btn_pos),
                    egui::Event::PointerButton {
                        pos: lib_btn_pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                crate::toolbar::show(&mut app, ui);
            });
            out.drop_without_applying_deltas();
        }

        assert_eq!(app.view.screen, crate::AppScreen::Home);
    }

    #[test]
    fn header_height_matches_editor_toolbar_and_stays_stable() {
        let mut app = PaperCraftApp::default();
        let _ = app.session.execute(
            "notebook.create",
            &json!({ "preset": "EngineeringGrid", "title": "A" }),
        );
        let _ = app.session.execute(
            "notebook.create",
            &json!({
                "preset": "LectureCornell",
                "title": "Very Long Multi-Semester Differential Geometry Notebook Title"
            }),
        );

        let wide = Vec2::new(1920.0, 1080.0);
        let home_metrics = run_home_frames(&mut app, wide, 2);

        let run_toolbar = |app: &mut PaperCraftApp| -> Rect {
            let ctx = egui::Context::default();
            let mut tb_rect = Rect::NOTHING;
            for _ in 0..2 {
                let raw = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, wide)),
                    ..Default::default()
                };
                let out = ctx.run_ui(raw, |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    tb_rect = crate::toolbar::show_inner(app, ui);
                });
                out.drop_without_applying_deltas();
            }
            tb_rect
        };

        crate::open_notebook_in_editor(&mut app, 1);
        let short_tb = run_toolbar(&mut app);
        assert!(
            (home_metrics.header_rect.height() - short_tb.height()).abs() <= 1.0,
            "home header ({:.1}) and editor toolbar ({:.1}) heights must match",
            home_metrics.header_rect.height(),
            short_tb.height()
        );

        crate::open_notebook_in_editor(&mut app, 2);
        app.view.zoom = 1.55;
        let long_tb = run_toolbar(&mut app);
        assert!((short_tb.height() - long_tb.height()).abs() <= 0.5);
    }

    #[test]
    fn home_grid_cards_keep_constant_size_across_filters_and_viewports() {
        let mut app = PaperCraftApp::default();
        let titles = [
            "Short",
            "Calculus and Linear Algebra Lecture Notes",
            "Lab Journal",
            "Quarterly Architecture Plan",
        ];
        for (i, title) in titles.iter().enumerate() {
            let preset = if i % 2 == 0 {
                NotebookPreset::EngineeringGrid
            } else {
                NotebookPreset::LectureCornell
            };
            let _ = app.session.execute(
                "notebook.create",
                &json!({ "preset": preset, "title": title }),
            );
        }

        for size in [Vec2::new(1280.0, 800.0), Vec2::new(840.0, 560.0)] {
            let m = run_home_frames(&mut app, size, 2);
            let create = m.create_card_rect.expect("create card visible");
            let expected_w = create.width();
            assert!(
                expected_w >= MIN_CARD_WIDTH - 0.5,
                "expected create card width >= {MIN_CARD_WIDTH}, got {expected_w} (rect={create:?}) at {size:?}"
            );
            assert!(
                (create.height() - CARD_HEIGHT).abs() <= 1.0,
                "expected create card height {CARD_HEIGHT}, got {} (rect={create:?}) at {size:?}",
                create.height()
            );
            for card in &m.card_rects {
                assert!(
                    (card.width() - expected_w).abs() <= 1.0,
                    "expected uniform card width {expected_w}, got {} at {size:?}",
                    card.width()
                );
                assert!(
                    (card.height() - CARD_HEIGHT).abs() <= 1.0,
                    "expected uniform card height {CARD_HEIGHT}, got {} at {size:?}",
                    card.height()
                );
            }
            let grid_budget = (m.content_rect.width() - SCROLLBAR_RESERVE).max(0.0);
            let cols = ((grid_budget + CARD_GAP) / (expected_w + CARD_GAP))
                .round()
                .max(1.0);
            let filled_w = cols * expected_w + (cols - 1.0) * CARD_GAP;
            assert!(
                (filled_w - grid_budget).abs() <= 2.0,
                "expected cards to fill grid budget edge-to-edge at {size:?}: filled={filled_w}, grid_budget={grid_budget}"
            );
        }

        let wide = Vec2::new(1280.0, 800.0);
        let full = run_home_frames(&mut app, wide, 2);
        app.view.library_search = "Short".to_string();
        let filtered = run_home_frames(&mut app, wide, 2);
        assert_eq!(filtered.card_rects.len(), 1);
        assert!((filtered.card_rects[0].width() - full.card_rects[0].width()).abs() <= 0.5);
        assert!((filtered.card_rects[0].height() - full.card_rects[0].height()).abs() <= 0.5);
        assert!((filtered.card_rects[0].left() - full.card_rects[0].left()).abs() <= 0.5);
    }

    #[test]
    fn trash_view_only_offers_restore_and_has_no_delete_button() {
        let mut app = PaperCraftApp::default();
        app.session
            .execute(
                "notebook.create",
                &json!({ "preset": "SketchBlank", "title": "Sketchbook" }),
            )
            .unwrap();
        app.session
            .execute("notebook.delete", &json!({ "index": 1 }))
            .unwrap();
        assert_eq!(app.session.trashed_notebooks.len(), 1);

        app.view.library_show_trash = true;
        let ctx = egui::Context::default();
        let mut rendered = String::new();
        let mut card_rect = Rect::NOTHING;
        for _ in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0))),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                let m = show_inner(&mut app, ui);
                card_rect = m.card_rects[0];
            });
            rendered.clear();
            fn collect_text(shape: &egui::Shape, buf: &mut String) {
                match shape {
                    egui::Shape::Text(text) => {
                        buf.push_str(text.galley.text());
                        buf.push('\n');
                    }
                    egui::Shape::Vec(items) => {
                        for s in items {
                            collect_text(s, buf);
                        }
                    }
                    _ => {}
                }
            }
            for clipped in &out.shapes {
                collect_text(&clipped.shape, &mut rendered);
            }
            out.drop_without_applying_deltas();
        }

        assert!(
            rendered.contains("Restore"),
            "expected Restore button in Trash card, got:\n{rendered}"
        );
        assert!(
            !rendered.contains("Delete forever"),
            "Trash view must not expose permanent delete button, got:\n{rendered}"
        );
        assert!(
            !rendered.contains("remove it permanently"),
            "Trash subtitle must not mention manual permanent removal, got:\n{rendered}"
        );

        // Clicking Restore returns the notebook to the active library.
        let restore_pos = pos2(card_rect.left() + 48.0, card_rect.bottom() - 28.0);
        for (pressed, time) in [(true, 0.1), (false, 0.2)] {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0))),
                time: Some(time),
                events: vec![
                    egui::Event::PointerMoved(restore_pos),
                    egui::Event::PointerButton {
                        pos: restore_pos,
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
        assert!(app.session.trashed_notebooks.is_empty());
        assert!(!app.view.library_show_trash);
        assert!(
            app.session
                .notebooks
                .iter()
                .any(|nb| nb.title == "Sketchbook")
        );
    }

    #[test]
    fn folders_favorites_tags_sorting_and_list_view_work_in_home_library() {
        let mut app = PaperCraftApp::default();
        let _ = app.session.execute(
            "notebook.create",
            &json!({
                "title": "Zeta Notes",
                "folder_path": "School/Math",
                "favorite": true,
                "tags": ["calculus"],
            }),
        );
        let _ = app.session.execute(
            "notebook.create",
            &json!({
                "title": "Alpha Notes",
                "folder_path": "Work",
                "tags": ["planning"],
            }),
        );

        // Opening "School" renders its "Math" subfolder tile and direct notebooks; searching
        // inside "School" searches all nested subfolders too.
        crate::library_folders::set_scope(&mut app, LibraryScope::Folder("School".to_string()));
        assert!(matching_notebook_indices(&app).is_empty());
        let school_metrics = run_home_frames(&mut app, Vec2::new(1280.0, 800.0), 2);
        assert_eq!(school_metrics.folder_tile_rects.len(), 1);
        app.view.library_search = "zeta".to_string();
        assert_eq!(matching_notebook_indices(&app), vec![1]);
        app.view.library_search.clear();

        crate::library_folders::set_scope(
            &mut app,
            LibraryScope::Folder("School/Math".to_string()),
        );
        assert_eq!(matching_notebook_indices(&app), vec![1]);

        crate::library_folders::set_scope(&mut app, LibraryScope::Unfiled);
        assert_eq!(matching_notebook_indices(&app), vec![0]);

        crate::library_folders::set_scope(&mut app, LibraryScope::Favorites);
        assert_eq!(matching_notebook_indices(&app), vec![1]);

        crate::library_folders::set_scope(&mut app, LibraryScope::Tag("planning".to_string()));
        assert_eq!(matching_notebook_indices(&app), vec![2]);

        crate::library_folders::set_scope(&mut app, LibraryScope::All);
        let _ = app.session.execute(
            "library.setViewPrefs",
            &json!({
                "view_mode": "list",
                "sort_by": "name",
                "sort_ascending": true,
            }),
        );
        let sorted = matching_notebook_indices(&app);
        assert_eq!(app.session.notebooks[sorted[0]].title, "Alpha Notes");

        let m = run_home_frames(&mut app, Vec2::new(1100.0, 720.0), 2);
        assert_eq!(m.card_rects.len(), 3);
        assert!(m.create_card_rect.is_some());
    }

    #[test]
    fn home_grid_auto_layout_expands_columns_and_centers_symmetrically_on_wide_screens() {
        let mut app = PaperCraftApp::default();
        for i in 0..6 {
            let _ = app.session.execute(
                "notebook.create",
                &json!({ "preset": "EverydayLined", "title": format!("Book {i}") }),
            );
        }

        // At 1920px wide with the 236px Library sidebar: 6 columns fit in the 1684px content area.
        let full_hd = Vec2::new(1920.0, 1080.0);
        let m_hd = run_home_frames(&mut app, full_hd, 2);
        let sidebar_hd = m_hd
            .sidebar_rect
            .expect("sidebar must be visible on wide screen");
        assert_eq!(m_hd.card_rects.len(), 7);
        let row1_y = m_hd.card_rects[0].top();
        let row1_count = m_hd
            .card_rects
            .iter()
            .filter(|r| (r.top() - row1_y).abs() <= 1.0)
            .count();
        assert_eq!(
            row1_count, 6,
            "expected 6 cards in row 1 at 1920px beside sidebar, got {row1_count}"
        );
        let left_margin = m_hd.content_rect.left() - sidebar_hd.right();
        let right_margin = full_hd.x - m_hd.content_rect.right();
        assert!(
            (left_margin - right_margin).abs() <= 1.5,
            "expected symmetric centering beside sidebar at 1920px: left={left_margin}, right={right_margin}"
        );

        // At 1560px wide: 5 fluid columns fill the content budget cleanly without wasted side voids.
        let wide_1560 = Vec2::new(1560.0, 900.0);
        let m_1560 = run_home_frames(&mut app, wide_1560, 2);
        let sidebar_1560 = m_1560
            .sidebar_rect
            .expect("sidebar must be visible at 1560px");
        let left_1560 = m_1560.content_rect.left() - sidebar_1560.right();
        let right_1560 = wide_1560.x - m_1560.content_rect.right();
        assert!(
            left_1560 >= MIN_SIDE_PAD - 0.5,
            "expected left side_pad >= {MIN_SIDE_PAD}, got {left_1560}"
        );
        assert!(
            (left_1560 - right_1560).abs() <= 1.5,
            "expected symmetric margins at 1560px: left={left_1560}, right={right_1560}"
        );
        let row1_y_1560 = m_1560.card_rects[0].top();
        let row1_count_1560 = m_1560
            .card_rects
            .iter()
            .filter(|r| (r.top() - row1_y_1560).abs() <= 1.0)
            .count();
        assert_eq!(
            row1_count_1560, 5,
            "expected 5 columns in row 1 at 1560x900, got {row1_count_1560}"
        );
        let card_w_1560 = m_1560.card_rects[0].width();
        assert!(
            card_w_1560 >= MIN_CARD_WIDTH - 0.5,
            "expected expanded card width >= {MIN_CARD_WIDTH} at 1560x900, got {card_w_1560}"
        );
        let grid_budget_1560 = (m_1560.content_rect.width() - SCROLLBAR_RESERVE).max(0.0);
        let row1_span_1560 =
            row1_count_1560 as f32 * card_w_1560 + (row1_count_1560 - 1) as f32 * CARD_GAP;
        assert!(
            (row1_span_1560 - grid_budget_1560).abs() <= 2.0,
            "expected 5 cards + gaps to fill grid budget edge-to-edge: span={row1_span_1560}, grid_budget={grid_budget_1560}"
        );

        // Narrow viewport (480px) collapses sidebar into Browse menu and wraps cleanly.
        let narrow = Vec2::new(480.0, 640.0);
        let m_narrow = run_home_frames(&mut app, narrow, 2);
        assert!(m_narrow.sidebar_rect.is_none());
        assert!(m_narrow.content_rect.left() >= 0.0);
        assert!(m_narrow.content_rect.right() <= narrow.x + 0.5);
        assert!(m_narrow.content_rect.width() >= 400.0);
    }

    #[test]
    fn home_tabs_and_sort_combo_share_uniform_28px_height_and_baseline() {
        let mut app = PaperCraftApp::default();
        let ctx = egui::Context::default();
        let mut bar_row_rect = Rect::NOTHING;
        let mut combo_rect = Rect::NOTHING;

        for _ in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1280.0, 800.0))),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_visuals(ui.ctx(), papercraft_doc::UiTheme::Light);
                let t = Tokens::light();
                bar_row_rect = ui
                    .horizontal(|ui| {
                        draw_browse_button(&mut app, ui, &t);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.spacing_mut().item_spacing.x = 6.0;
                            draw_library_view_switch(&mut app, ui, &t);
                            draw_library_sort_dir_switch(&mut app, ui, &t);
                            let resp = crate::theme::studio_combo_box(
                                ui,
                                "test_uniform_combo",
                                "Modified Date",
                                128.0,
                                |_| {},
                            )
                            .response;
                            combo_rect = resp.rect;
                        });
                    })
                    .response
                    .rect;
            });
            out.drop_without_applying_deltas();
        }

        assert!(
            (bar_row_rect.height() - 28.0).abs() <= 1.0,
            "expected 28px uniform bar height, got {}",
            bar_row_rect.height()
        );
        assert!(
            (combo_rect.height() - 28.0).abs() <= 1.0,
            "expected 28px uniform combo height, got {}",
            combo_rect.height()
        );
        assert!(
            (combo_rect.center().y - bar_row_rect.center().y).abs() <= 0.5,
            "expected combo ({:?}) vertically centered in bar row ({:?})",
            combo_rect,
            bar_row_rect
        );

        let m = run_home_frames(&mut app, Vec2::new(1280.0, 800.0), 2);
        let card = m.card_rects[0];
        let bottom_pad = card.bottom() - m.first_card_actions_rect.bottom();
        assert!(
            (bottom_pad - f32::from(CARD_MARGIN)).abs() <= 0.5,
            "expected card bottom action row anchored at {CARD_MARGIN}px margin, got {bottom_pad}"
        );
    }

    #[test]
    fn dragging_notebook_card_onto_sidebar_folder_moves_notebook() {
        let mut app = PaperCraftApp::default();
        app.session
            .execute("library.folder.create", &json!({ "path": "Projects" }))
            .unwrap();
        assert_eq!(app.session.notebooks[0].folder_path, "");

        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let mut m = HomeMetrics {
            header_rect: Rect::NOTHING,
            sidebar_rect: None,
            content_rect: Rect::NOTHING,
            folder_tile_rects: Vec::new(),
            card_rects: Vec::new(),
            create_card_rect: None,
            first_card_actions_rect: Rect::NOTHING,
        };
        let mut target_folder_pos = None;
        for _ in 0..3 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                m = show_inner(&mut app, ui);
            });
            for clipped in &out.shapes {
                if let egui::Shape::Text(text) = &clipped.shape
                    && text.galley.text() == "Projects"
                {
                    target_folder_pos = Some(pos2(
                        m.sidebar_rect.expect("sidebar").center().x,
                        text.pos.y + 6.0,
                    ));
                }
            }
            out.drop_without_applying_deltas();
        }
        let drop_pos = target_folder_pos.expect("Projects folder row should render in sidebar");
        let start_pos = pos2(m.card_rects[0].center().x, m.card_rects[0].top() + 56.0);

        let steps = [
            (0.05, start_pos, Some(true)),
            (0.10, pos2(start_pos.x - 40.0, start_pos.y), None),
            (0.15, drop_pos, None),
            (0.20, drop_pos, Some(false)),
        ];
        for (time, pos, button) in steps {
            let mut events = vec![egui::Event::PointerMoved(pos)];
            if let Some(pressed) = button {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                time: Some(time),
                events,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                let _ = show_inner(&mut app, ui);
            });
            out.drop_without_applying_deltas();
        }

        assert_eq!(app.session.notebooks[0].folder_path, "Projects");
        assert_eq!(app.view.screen, crate::AppScreen::Home);
    }

    #[test]
    fn empty_states_stay_button_free_while_actions_sit_in_section_header() {
        fn collect_content_text_rects(
            shapes: &[egui::epaint::ClippedShape],
            m: &HomeMetrics,
        ) -> Vec<(String, Rect)> {
            fn visit(shape: &egui::Shape, m: &HomeMetrics, out: &mut Vec<(String, Rect)>) {
                match shape {
                    egui::Shape::Text(text) => {
                        let r = text.visual_bounding_rect();
                        if r.left() >= m.content_rect.left() - 4.0
                            && r.top() >= m.header_rect.bottom() - 1.0
                        {
                            out.push((text.galley.text().to_string(), r));
                        }
                    }
                    egui::Shape::Vec(items) => {
                        for item in items {
                            visit(item, m, out);
                        }
                    }
                    _ => {}
                }
            }
            let mut out = Vec::new();
            for clipped in shapes {
                visit(&clipped.shape, m, &mut out);
            }
            out
        }

        fn click_at(ctx: &egui::Context, app: &mut PaperCraftApp, viewport: Vec2, pos: Pos2) {
            let t0 = ctx.input(|i| i.time);
            for (pressed, time) in [(true, t0 + 0.05), (false, t0 + 0.10)] {
                let raw = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                    time: Some(time),
                    events: vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
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
        }

        let mut app = PaperCraftApp::default();
        app.session
            .execute("library.folder.create", &json!({ "path": "test/mk" }))
            .unwrap();
        crate::library_folders::set_scope(&mut app, LibraryScope::Folder("test/mk".to_string()));

        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let mut metrics = HomeMetrics {
            header_rect: Rect::NOTHING,
            sidebar_rect: None,
            content_rect: Rect::NOTHING,
            folder_tile_rects: Vec::new(),
            card_rects: Vec::new(),
            create_card_rect: None,
            first_card_actions_rect: Rect::NOTHING,
        };
        let mut content_texts = Vec::new();
        for _ in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                metrics = show_inner(&mut app, ui);
            });
            content_texts = collect_content_text_rects(&out.shapes, &metrics);
            out.drop_without_applying_deltas();
        }

        // Empty folder without subfolders renders only the '+' New Notebook card instead of an empty block.
        assert!(
            !content_texts
                .iter()
                .any(|(txt, _)| txt == "\"mk\" is empty"),
            "empty folder should not render the large empty-state block"
        );
        let create_rect = metrics
            .create_card_rect
            .expect("expected '+' New Notebook card in empty folder");
        assert!(
            content_texts.iter().any(|(txt, r)| txt
                == "Pick a cover, paper ruling, and sheet tint"
                && create_rect.contains(r.center())),
            "expected '+' create card subtitle inside create_card_rect"
        );

        let subfolder_rect = content_texts
            .iter()
            .find(|(txt, _)| txt == "Subfolder")
            .expect("expected Subfolder header button")
            .1;
        for label in ["New Notebook", "Subfolder", "Rename"] {
            let header_matches: Vec<Rect> = content_texts
                .iter()
                .filter(|(txt, r)| txt == label && !create_rect.contains(r.center()))
                .map(|(_, r)| *r)
                .collect();
            assert_eq!(
                header_matches.len(),
                1,
                "expected single '{label}' button in folder header, got {header_matches:?}"
            );
            let r = header_matches[0];
            assert!(
                r.bottom() < create_rect.top(),
                "'{label}' header button ({r:?}) must be above '+' card ({create_rect:?})"
            );
            assert!(
                (r.center().y - subfolder_rect.center().y).abs() <= 2.0,
                "'{label}' ({r:?}) was not aligned on header row with Subfolder ({subfolder_rect:?})"
            );
        }
        for forbidden in [
            "All Notebooks",
            "New Notebook Here",
            "New Subfolder",
            "Show All Notebooks",
            "Clear Search",
        ] {
            assert!(
                !content_texts.iter().any(|(txt, _)| txt == forbidden),
                "unexpected button '{forbidden}' rendered in content area"
            );
        }

        // Narrow viewport wraps header actions cleanly without overflowing content_rect.
        let narrow = Vec2::new(480.0, 640.0);
        for _ in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, narrow)),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                metrics = show_inner(&mut app, ui);
            });
            let narrow_texts = collect_content_text_rects(&out.shapes, &metrics);
            let mk_title_r = narrow_texts
                .iter()
                .find(|(txt, _)| txt == "mk")
                .expect("mk folder title")
                .1;
            let rename_r = narrow_texts
                .iter()
                .find(|(txt, _)| txt == "Rename")
                .expect("Rename button")
                .1;
            assert!(
                rename_r.top() > mk_title_r.bottom(),
                "expected narrow header buttons ({rename_r:?}) to wrap below title ({mk_title_r:?})"
            );
            assert!(rename_r.right() <= metrics.content_rect.right() + 2.0);
            out.drop_without_applying_deltas();
        }

        // Empty Search, Favorites, Unfiled, and Trash states also remain free of buttons.
        app.view.library_search = "nonexistent-query".to_string();
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
            ..Default::default()
        };
        let out = ctx.run_ui(raw, |ui| {
            crate::theme::apply_light_visuals(ui.ctx());
            metrics = show_inner(&mut app, ui);
        });
        let search_texts = collect_content_text_rects(&out.shapes, &metrics);
        let no_match_y = search_texts
            .iter()
            .find(|(txt, _)| txt.contains("No notebooks match"))
            .expect("empty search title")
            .1
            .top();
        assert!(
            !search_texts
                .iter()
                .any(|(_, r)| r.top() >= no_match_y + 24.0
                    && ["Clear Search", "Show All Notebooks", "New Notebook"]
                        .iter()
                        .any(|b| search_texts.iter().any(|(t, tr)| t == b && *tr == *r))),
            "empty search state should contain no buttons"
        );
        out.drop_without_applying_deltas();
        app.view.library_search.clear();

        // Settle one frame after clearing search so '+' card enters layout before clicking it.
        for _ in 0..2 {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                    ..Default::default()
                },
                |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    metrics = show_inner(&mut app, ui);
                },
            );
            out.drop_without_applying_deltas();
        }
        let create_rect = metrics
            .create_card_rect
            .expect("expected '+' card after clearing search");

        // Clicking '+' card in the empty folder opens NewNotebook targeted at that folder,
        // and clicking "Library" in the breadcrumb returns to All Notebooks.
        click_at(&ctx, &mut app, viewport, create_rect.center());
        assert_eq!(app.view.active_modal, Some(ActiveModal::NewNotebook));
        assert_eq!(app.view.new_notebook_folder, "test/mk");
        modal::close(&mut app);

        let library_crumb_pos = content_texts
            .iter()
            .find(|(txt, _)| txt == "Library")
            .expect("Library breadcrumb link")
            .1
            .center();
        click_at(&ctx, &mut app, viewport, library_crumb_pos);
        assert_eq!(current_scope(&app), LibraryScope::All);
    }

    #[test]
    fn new_notebook_header_button_appears_in_every_scope_except_trash() {
        fn header_new_notebook_pos(
            shapes: &[egui::epaint::ClippedShape],
            m: &HomeMetrics,
        ) -> Option<Pos2> {
            fn visit(shape: &egui::Shape, m: &HomeMetrics) -> Option<Pos2> {
                match shape {
                    egui::Shape::Text(text) if text.galley.text() == "New Notebook" => {
                        let r = text.visual_bounding_rect();
                        let in_card = m.create_card_rect.is_some_and(|c| c.contains(r.center()));
                        (!in_card && r.top() >= m.header_rect.bottom() - 1.0).then(|| r.center())
                    }
                    egui::Shape::Vec(items) => items.iter().find_map(|item| visit(item, m)),
                    _ => None,
                }
            }
            shapes.iter().find_map(|c| visit(&c.shape, m))
        }

        let mut app = PaperCraftApp::default();
        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let scopes = [
            LibraryScope::All,
            LibraryScope::Favorites,
            LibraryScope::Unfiled,
            LibraryScope::Tag("work".to_string()),
            LibraryScope::Trash,
        ];
        for scope in scopes {
            crate::library_folders::set_scope(&mut app, scope.clone());
            let mut pos = None;
            let mut time = ctx.input(|i| i.time);
            for _ in 0..3 {
                time += 0.05;
                let raw = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                    time: Some(time),
                    ..Default::default()
                };
                let mut metrics = None;
                let out = ctx.run_ui(raw, |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    metrics = Some(show_inner(&mut app, ui));
                });
                pos = metrics
                    .as_ref()
                    .and_then(|m| header_new_notebook_pos(&out.shapes, m));
                out.drop_without_applying_deltas();
            }

            if scope == LibraryScope::Trash {
                assert!(pos.is_none(), "Trash must not offer New Notebook");
                continue;
            }
            let pos = pos.unwrap_or_else(|| panic!("missing New Notebook button in {scope:?}"));
            for (pressed, dt) in [(true, 0.05), (false, 0.10)] {
                let raw = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                    time: Some(time + dt),
                    events: vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
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
            assert_eq!(app.view.active_modal, Some(ActiveModal::NewNotebook));
            assert_eq!(
                app.view.new_notebook_favorite,
                scope == LibraryScope::Favorites
            );
            let expected_tags: Vec<String> = match &scope {
                LibraryScope::Tag(tag) => vec![tag.clone()],
                _ => Vec::new(),
            };
            assert_eq!(app.view.new_notebook_tags, expected_tags);
            modal::close(&mut app);
            for _ in 0..8 {
                time += 0.1;
                let out = ctx.run_ui(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                        time: Some(time),
                        ..Default::default()
                    },
                    |ui| {
                        crate::theme::apply_light_visuals(ui.ctx());
                        let _ = show_inner(&mut app, ui);
                    },
                );
                out.drop_without_applying_deltas();
            }
        }
    }

    #[test]
    fn home_scope_and_view_transitions_fade_calmly_without_shifting_layout() {
        let mut app = PaperCraftApp::default();
        app.session
            .execute(
                "notebook.create",
                &json!({ "title": "Studio Notes", "folder_path": "Projects/Active" }),
            )
            .unwrap();
        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);

        let run_step = |ctx: &egui::Context,
                        app: &mut PaperCraftApp,
                        t: f64,
                        hover: Option<Pos2>|
         -> HomeMetrics {
            let events = hover
                .map(|p| vec![egui::Event::PointerMoved(p)])
                .unwrap_or_default();
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                time: Some(t),
                predicted_dt: 0.04,
                events,
                ..Default::default()
            };
            let mut m = HomeMetrics {
                header_rect: Rect::NOTHING,
                sidebar_rect: None,
                content_rect: Rect::NOTHING,
                folder_tile_rects: Vec::new(),
                card_rects: Vec::new(),
                create_card_rect: None,
                first_card_actions_rect: Rect::NOTHING,
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                m = show_inner(app, ui);
            });
            out.drop_without_applying_deltas();
            m
        };

        // Initial render starts completely settled.
        let mut t = 0.0;
        let m0 = run_step(&ctx, &mut app, t, None);
        assert_eq!(app.view.motion.home_scope.opacity(), 1.0);
        assert_eq!(app.view.motion.home_view.opacity(), 1.0);

        // Switching scope starts a calm reveal fade on home_scope without double-fading home_view.
        crate::library_folders::set_scope(&mut app, LibraryScope::Folder("Projects".to_string()));
        t += 0.04;
        let m_mid = run_step(&ctx, &mut app, t, None);
        assert!(app.view.motion.home_scope.opacity() < 1.0);
        assert_eq!(app.view.motion.home_view.opacity(), 1.0);
        assert_eq!(m_mid.content_rect, m0.content_rect);
        assert_eq!(m_mid.folder_tile_rects.len(), 1);

        // Hovering the subfolder tile settles cleanly alongside the scope reveal.
        let tile_center = m_mid.folder_tile_rects[0].center();
        for _ in 0..5 {
            t += 0.04;
            let _ = run_step(&ctx, &mut app, t, Some(tile_center));
        }
        assert_eq!(app.view.motion.home_scope.opacity(), 1.0);

        // Switching Grid -> List triggers home_view fade while keeping home_scope steady at 1.0.
        app.session
            .execute("library.setViewPrefs", &json!({ "view_mode": "list" }))
            .unwrap();
        t += 0.04;
        let _ = run_step(&ctx, &mut app, t, None);
        assert_eq!(app.view.motion.home_scope.opacity(), 1.0);
        assert!(app.view.motion.home_view.opacity() < 1.0);

        for _ in 0..5 {
            t += 0.04;
            let _ = run_step(&ctx, &mut app, t, None);
        }
        assert_eq!(app.view.motion.home_view.opacity(), 1.0);
    }

    #[test]
    fn home_header_renders_brand_logo_omits_new_notebook_and_toggles_settings_sidebar() {
        let mut app = PaperCraftApp::default();
        app.session
            .execute(
                "library.folder.create",
                &json!({ "path": "School/Math", "color_hex": "#0D9488" }),
            )
            .unwrap();
        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);

        let mut header_rect = Rect::NOTHING;
        let mut header_texts: Vec<(String, Rect)> = Vec::new();
        for _ in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                header_rect = show_inner(&mut app, ui).header_rect;
            });
            header_texts.clear();
            fn visit(shape: &egui::Shape, header: Rect, out: &mut Vec<(String, Rect)>) {
                match shape {
                    egui::Shape::Text(text) => {
                        let r = text.visual_bounding_rect();
                        if header.contains(r.center()) {
                            out.push((text.galley.text().to_string(), r));
                        }
                    }
                    egui::Shape::Vec(items) => {
                        for item in items {
                            visit(item, header, out);
                        }
                    }
                    _ => {}
                }
            }
            for clipped in &out.shapes {
                visit(&clipped.shape, header_rect, &mut header_texts);
            }
            out.drop_without_applying_deltas();
        }

        assert!(
            brand_logo_texture(&ctx).is_some(),
            "expected embedded PaperCraft brand logo texture to load"
        );
        assert!(
            !header_texts
                .iter()
                .any(|(label, _)| label == "New Notebook"),
            "home header must not contain 'New Notebook' button, got {header_texts:?}"
        );
        assert!(
            !header_texts.iter().any(|(label, _)| label == "Theme"),
            "home header should use standardized 'Settings' button instead of 'Theme' modal"
        );
        let settings_pos = header_texts
            .iter()
            .find(|(label, _)| label == "Settings")
            .expect("expected 'Settings' button in home header")
            .1
            .center();

        for (pressed, time) in [(true, 0.05), (false, 0.10)] {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                time: Some(time),
                events: vec![
                    egui::Event::PointerMoved(settings_pos),
                    egui::Event::PointerButton {
                        pos: settings_pos,
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
        assert!(app.view.inspector_open);
        assert_eq!(app.view.settings_tab, crate::SettingsTab::Appearance);
        assert_eq!(app.view.active_modal, None);

        // Settle sidebar open on Home and verify Appearance + App Scale render inside the sidebar.
        let mut body_texts = String::new();
        for f in 1..=8 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                time: Some(0.10 + f as f64 * 0.03),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                let _ = show_inner(&mut app, ui);
            });
            body_texts.clear();
            fn collect_all(shape: &egui::Shape, out: &mut String) {
                match shape {
                    egui::Shape::Text(text) => {
                        out.push_str(text.galley.text());
                        out.push('\n');
                    }
                    egui::Shape::Vec(items) => {
                        for item in items {
                            collect_all(item, out);
                        }
                    }
                    _ => {}
                }
            }
            for clipped in &out.shapes {
                collect_all(&clipped.shape, &mut body_texts);
            }
            out.drop_without_applying_deltas();
        }
        assert!(body_texts.contains("Appearance"));
        assert!(body_texts.contains("App Scale"));
        assert!(body_texts.contains("100%"));
    }

    #[test]
    fn tag_scope_header_exposes_color_button_and_list_view_renders_colored_tags() {
        let mut app = PaperCraftApp::default();
        app.session
            .execute(
                "notebook.create",
                &json!({ "title": "Lecture Notes", "tags": ["example"] }),
            )
            .unwrap();
        app.session
            .execute(
                "library.tag.setColor",
                &json!({ "tag": "example", "color_hex": "#DC2626" }),
            )
            .unwrap();
        app.session
            .execute("library.setViewPrefs", &json!({ "view_mode": "list" }))
            .unwrap();
        crate::library_folders::set_scope(&mut app, LibraryScope::Tag("example".to_string()));

        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let mut color_btn_pos = None;
        for _ in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                let _ = show_inner(&mut app, ui);
            });
            fn find_label(shape: &egui::Shape, target: &str, out: &mut Option<Pos2>) {
                match shape {
                    egui::Shape::Text(text) if text.galley.text() == target => {
                        *out = Some(text.visual_bounding_rect().center());
                    }
                    egui::Shape::Vec(items) => {
                        for item in items {
                            find_label(item, target, out);
                        }
                    }
                    _ => {}
                }
            }
            for clipped in &out.shapes {
                find_label(&clipped.shape, "Color", &mut color_btn_pos);
            }
            out.drop_without_applying_deltas();
        }

        let pos = color_btn_pos.expect("expected Color button in tag section header");
        for (pressed, time) in [(true, 0.05), (false, 0.10)] {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                time: Some(time),
                events: vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
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
        assert_eq!(app.view.active_modal, Some(ActiveModal::TagColor));
        assert_eq!(app.view.tag_dialog_target, "example");
        assert_eq!(
            app.view.tag_dialog_color,
            papercraft_color::Color::from_hex("#DC2626")
        );
    }

    #[test]
    fn home_header_shows_active_workspace_indicator_and_notebook_sync_badges() {
        fn collect_all(shape: &egui::Shape, out: &mut String) {
            match shape {
                egui::Shape::Text(text) => {
                    out.push_str(text.galley.text());
                    out.push('\n');
                }
                egui::Shape::Vec(items) => {
                    for item in items {
                        collect_all(item, out);
                    }
                }
                _ => {}
            }
        }

        fn render_home_text(ctx: &egui::Context, app: &mut PaperCraftApp) -> String {
            let viewport = Vec2::new(1280.0, 800.0);
            let mut text = String::new();
            for step in 0..3 {
                let raw = egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, viewport)),
                    time: Some(f64::from(step) * 0.05),
                    predicted_dt: 0.05,
                    ..Default::default()
                };
                let out = ctx.run_ui(raw, |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    let _ = show_inner(app, ui);
                });
                text.clear();
                for clipped in &out.shapes {
                    collect_all(&clipped.shape, &mut text);
                }
                out.drop_without_applying_deltas();
            }
            text
        }

        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();

        let local_text = render_home_text(&ctx, &mut app);
        assert!(
            local_text.contains("Local · Personal"),
            "missing Local · Personal pill in:\n{local_text}"
        );
        assert!(
            local_text.contains("Local only"),
            "missing Local only badge in:\n{local_text}"
        );

        app.session.switch_to_authenticated_library(
            papercraft_sync::ActiveAccountSession {
                server_url: papercraft_sync_protocol::DEFAULT_SERVER_URL.to_string(),
                user: papercraft_sync_protocol::UserProfile {
                    user_id: "u-home".to_string(),
                    email: Some("alex@example.com".to_string()),
                    username: "alex".to_string(),
                    display_name: "Alex".to_string(),
                    created_at_ms: 1000,
                    must_change_password: false,
                },
                access_token: "a".to_string(),
                refresh_token: "r".to_string(),
                access_expires_at_ms: 99_999,
                refresh_expires_at_ms: 99_999,
                device_name: "WinPC".to_string(),
                last_auth_ms: 1000,
            },
            papercraft_engine::AuthWorkspaceMode::SwitchOnly,
        );
        app.session
            .execute("notebook.create", &json!({ "title": "Cloud Notes" }))
            .unwrap();

        let cloud_text = render_home_text(&ctx, &mut app);
        assert!(
            cloud_text.contains("alex@example.com"),
            "missing email workspace badge in:\n{cloud_text}"
        );
        assert!(
            cloud_text.contains("Waiting to sync"),
            "missing Waiting to sync badge in:\n{cloud_text}"
        );
    }
}
