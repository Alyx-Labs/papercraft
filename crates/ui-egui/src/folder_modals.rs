//! New Folder, Rename Folder, Folder Color, and Delete Folder dialogs for the Notebooks Library.
//!
//! All four use the shared centered modal architecture in [`crate::modal`]:
//! focused inputs, a visual location list, and a preset + custom color picker.

use egui::{Color32, CornerRadius, Margin, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2};
use papercraft_color::Color;
use papercraft_doc::{FOLDER_COLOR_PRESETS, UserStudioConfig};
use serde_json::json;

use crate::PaperCraftApp;
use crate::color_picker::{paint_hue_bar_picker, paint_sv_square_picker};
use crate::icons::{Icon, compact_icon_button, paint_icon};
use crate::library_folders::{
    LibraryScope, current_scope, display_folder_path, folder_depth, folder_leaf, folder_parent,
    folder_tree, folder_visual_colors, known_library_folders, move_notebook_to_folder, path_within,
    plural, set_scope, validate_folder_name,
};
use crate::modal::{
    self, ActiveModal, danger_action_button, primary_action_button, secondary_action_button,
};
use crate::theme::{Tokens, to_color32};

/// Interactive draft state for picking a custom RGB folder color.
#[derive(Clone, Debug)]
pub struct FolderColorDraftState {
    pub custom_open: bool,
    /// `[hue (0..=1), saturation (0..=1), value (0..=1)]`.
    pub hsv: [f32; 3],
    pub hex_draft: String,
}

impl Default for FolderColorDraftState {
    fn default() -> Self {
        let initial = Color::from_hex("#2563EB").unwrap_or(Color::BLACK);
        Self {
            custom_open: false,
            hsv: initial.to_hsv(),
            hex_draft: initial.to_hex(),
        }
    }
}

impl FolderColorDraftState {
    /// Synchronizes draft state from `color`, opening custom controls when a non-preset color is set.
    pub fn sync_from_color(&mut self, color: Option<Color>) {
        match color {
            Some(c) => {
                self.set_color(c);
                self.custom_open = !is_preset_folder_color(c);
            }
            None => {
                let fallback = Color::from_hex("#2563EB").unwrap_or(Color::BLACK);
                self.set_color(fallback);
                self.custom_open = false;
            }
        }
    }

    pub fn set_color(&mut self, color: Color) {
        let opaque = color.sanitized().with_alpha(1.0);
        self.hsv = opaque.to_hsv();
        self.hex_draft = opaque.to_hex();
    }

    pub fn current_color(&self) -> Color {
        Color::from_hsv(self.hsv[0], self.hsv[1], self.hsv[2], 1.0)
    }
}

fn same_rgb(a: Color, b: Color) -> bool {
    a.to_rgba8()[..3] == b.to_rgba8()[..3]
}

fn is_preset_folder_color(color: Color) -> bool {
    FOLDER_COLOR_PRESETS
        .iter()
        .filter_map(|&(_, hex)| Color::from_hex(hex))
        .any(|preset| same_rgb(color, preset))
}

/// Opens the New Folder dialog inside `parent` (`""` = top level). When
/// `move_notebook` is set, that notebook moves into the folder once it exists.
pub(crate) fn open_new_folder(app: &mut PaperCraftApp, parent: &str, move_notebook: Option<usize>) {
    app.view.folder_dialog_target.clear();
    app.view.folder_dialog_name.clear();
    app.view.folder_dialog_parent = parent.to_string();
    app.view.folder_dialog_move_notebook = move_notebook;
    app.view.folder_dialog_focus_pending = true;
    app.view.folder_dialog_color = None;
    app.view.folder_color_draft.sync_from_color(None);
    modal::open(app, ActiveModal::NewFolder);
}

/// Opens the Rename Folder dialog for `path`.
pub(crate) fn open_rename_folder(app: &mut PaperCraftApp, path: &str) {
    let existing_color = app.session.prefs.folder_color(path);
    app.view.folder_dialog_target = path.to_string();
    app.view.folder_dialog_name = folder_leaf(path).to_string();
    app.view.folder_dialog_parent = folder_parent(path).to_string();
    app.view.folder_dialog_move_notebook = None;
    app.view.folder_dialog_focus_pending = true;
    app.view.folder_dialog_color = existing_color;
    app.view.folder_color_draft.sync_from_color(existing_color);
    modal::open(app, ActiveModal::RenameFolder);
}

/// Opens the Folder Color dialog for `path`.
pub(crate) fn open_folder_color(app: &mut PaperCraftApp, path: &str) {
    let existing_color = app.session.prefs.folder_color(path);
    app.view.folder_dialog_target = path.to_string();
    app.view.folder_dialog_move_notebook = None;
    app.view.folder_dialog_focus_pending = false;
    app.view.folder_dialog_color = existing_color;
    app.view.folder_color_draft.sync_from_color(existing_color);
    modal::open(app, ActiveModal::FolderColor);
}

/// Opens the Tag Color dialog for `tag`.
pub(crate) fn open_tag_color(app: &mut PaperCraftApp, tag: &str) {
    let clean = papercraft_doc::sanitize_notebook_tag(tag).unwrap_or_else(|| tag.to_string());
    let existing_color = app.session.prefs.tag_color(&clean);
    app.view.tag_dialog_target = clean;
    app.view.tag_dialog_color = existing_color;
    app.view.tag_color_draft.sync_from_color(existing_color);
    modal::open(app, ActiveModal::TagColor);
}

/// Opens the Delete Folder confirmation for `path`.
pub(crate) fn open_delete_folder(app: &mut PaperCraftApp, path: &str) {
    app.view.folder_dialog_target = path.to_string();
    app.view.folder_dialog_move_notebook = None;
    modal::open(app, ActiveModal::DeleteFolder);
}

fn section_title(ui: &mut egui::Ui, title: &str, subtitle: &str, t: &Tokens) {
    ui.label(RichText::new(title).size(13.0).strong().color(t.text));
    ui.add(egui::Label::new(RichText::new(subtitle).size(11.0).color(t.text_muted)).wrap());
    ui.add_space(4.0);
}

/// Name input that grabs keyboard focus when the dialog opens. Returns `true` on Enter.
fn draw_name_field(ui: &mut egui::Ui, name: &mut String, focus_pending: &mut bool) -> bool {
    let id = ui.make_persistent_id("papercraft_folder_dialog_name");
    let resp = ui.add_sized(
        [ui.available_width(), 32.0],
        egui::TextEdit::singleline(name)
            .id(id)
            .hint_text("Folder name, e.g. Biology")
            .char_limit(crate::library_folders::MAX_FOLDER_NAME_CHARS)
            .margin(Margin::symmetric(8, 2))
            .align(egui::Align2::LEFT_CENTER),
    );
    if *focus_pending {
        resp.request_focus();
        *focus_pending = false;
    }
    resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter))
}

fn location_row(
    ui: &mut egui::Ui,
    icon: Icon,
    icon_color: Option<Color32>,
    label: &str,
    depth: usize,
    selected: bool,
    t: &Tokens,
) -> egui::Response {
    let width = ui.available_width().max(120.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 28.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        if selected {
            p.rect_filled(rect, CornerRadius::same(6), t.accent_soft);
        } else if resp.hovered() {
            p.rect_filled(rect, CornerRadius::same(6), t.border.gamma_multiply(0.45));
        }
        let fg = if selected { t.accent } else { t.text };
        let icon_fg = icon_color.unwrap_or(fg);
        let x = rect.left() + 8.0 + depth as f32 * 16.0;
        let icon_rect = Rect::from_center_size(pos2(x + 7.5, rect.center().y), Vec2::splat(15.0));
        paint_icon(p, icon_rect, icon, icon_fg);
        let galley = p.layout_no_wrap(label.to_string(), egui::FontId::proportional(12.5), fg);
        p.galley(
            pos2(x + 22.0, rect.center().y - galley.size().y * 0.5),
            galley,
            fg,
        );
        if selected {
            let check = Rect::from_center_size(
                pos2(rect.right() - 16.0, rect.center().y),
                Vec2::splat(14.0),
            );
            paint_icon(p, check, Icon::Check, t.accent);
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Visual list of every place a folder can live. Folders inside `exclude`
/// (the folder being moved) are hidden so a folder can't move into itself.
fn draw_location_list(
    ui: &mut egui::Ui,
    folders: &[String],
    prefs: &UserStudioConfig,
    parent: &mut String,
    exclude: Option<&str>,
    t: &Tokens,
) {
    egui::Frame::NONE
        .fill(t.panel_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(4))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().scroll.floating = false;
            egui::ScrollArea::vertical()
                .id_salt(("papercraft_folder_location_list", exclude.is_some()))
                .max_height(200.0)
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    if location_row(
                        ui,
                        Icon::Library,
                        None,
                        "Library (top level)",
                        0,
                        parent.is_empty(),
                        t,
                    )
                    .clicked()
                    {
                        parent.clear();
                    }
                    for folder in folders {
                        if exclude.is_some_and(|moving| path_within(folder, moving)) {
                            continue;
                        }
                        let selected = parent.eq_ignore_ascii_case(folder);
                        let icon = if selected {
                            Icon::FolderOpen
                        } else {
                            Icon::Folder
                        };
                        let icon_color = prefs
                            .folder_color(folder)
                            .map(|c| folder_visual_colors(Some(c), t).icon);
                        if location_row(
                            ui,
                            icon,
                            icon_color,
                            folder_leaf(folder),
                            folder_depth(folder),
                            selected,
                            t,
                        )
                        .on_hover_text(display_folder_path(folder))
                        .clicked()
                        {
                            *parent = folder.clone();
                        }
                    }
                });
        });
}

fn draw_folder_color_section(
    ui: &mut egui::Ui,
    selected: &mut Option<Color>,
    draft: &mut FolderColorDraftState,
    t: &Tokens,
) {
    draw_color_section(
        ui,
        "folder",
        "folder_color_studio_collapse",
        selected,
        draft,
        t,
    );
}

fn draw_color_section(
    ui: &mut egui::Ui,
    target_noun: &str,
    collapse_salt: &str,
    selected: &mut Option<Color>,
    draft: &mut FolderColorDraftState,
    t: &Tokens,
) {
    section_title(
        ui,
        "Color",
        "Pick a preset color or craft any custom shade.",
        t,
    );
    let is_custom = selected.is_some_and(|c| !is_preset_folder_color(c));
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        if compact_icon_button(
            ui,
            Icon::Reset,
            Some("Default"),
            selected.is_none(),
            true,
            t,
        )
        .on_hover_text(format!("Use the default theme {target_noun} color"))
        .clicked()
        {
            *selected = None;
            draft.custom_open = false;
        }
        let rim = if t.is_dark {
            Color32::from_white_alpha(48)
        } else {
            Color32::from_black_alpha(38)
        };
        for &(name, hex) in FOLDER_COLOR_PRESETS {
            let Some(swatch) = Color::from_hex(hex) else {
                continue;
            };
            let active = selected.is_some_and(|c| same_rgb(c, swatch));
            let (dot_rect, dot_resp) = ui.allocate_exact_size(Vec2::splat(22.0), Sense::click());
            if ui.is_rect_visible(dot_rect) {
                let p = ui.painter();
                if active {
                    p.circle_stroke(dot_rect.center(), 9.8, Stroke::new(1.8, t.accent));
                }
                p.circle_filled(dot_rect.center(), 7.6, to_color32(swatch));
                p.circle_stroke(dot_rect.center(), 7.6, Stroke::new(0.9, rim));
            }
            if dot_resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(format!("{name} ({hex})"))
                .clicked()
            {
                *selected = Some(swatch);
                draft.set_color(swatch);
                draft.custom_open = false;
            }
        }
        if compact_icon_button(
            ui,
            Icon::Palette,
            Some("Custom"),
            is_custom || draft.custom_open,
            true,
            t,
        )
        .on_hover_text("Pick any custom color using HSV sliders or a #RRGGBB hex code")
        .clicked()
        {
            draft.custom_open = !draft.custom_open;
            if draft.custom_open && selected.is_none() {
                *selected = Some(draft.current_color());
            }
        }
    });

    let custom_id = ui.make_persistent_id(collapse_salt);
    crate::motion::animated_collapse(ui, custom_id, draft.custom_open, 178.0, |ui| {
        ui.add_space(8.0);
        draw_inline_color_studio(ui, target_noun, selected, draft, t);
    });
}

fn draw_inline_color_studio(
    ui: &mut egui::Ui,
    target_noun: &str,
    selected: &mut Option<Color>,
    draft: &mut FolderColorDraftState,
    t: &Tokens,
) {
    egui::Frame::NONE
        .fill(t.panel_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::same(10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 8.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    let sv_w = (ui.available_width() - 26.0).max(120.0);
                    let mut changed =
                        paint_sv_square_picker(ui, Vec2::new(sv_w, 72.0), &mut draft.hsv, t);
                    changed |=
                        paint_hue_bar_picker(ui, Vec2::new(16.0, 72.0), true, &mut draft.hsv[0], t);
                    if changed {
                        let c = draft.current_color();
                        draft.hex_draft = c.to_hex();
                        *selected = Some(c);
                    }
                });
                draw_color_hex_row(ui, target_noun, selected, draft, t);
            });
        });
}

fn draw_color_hex_row(
    ui: &mut egui::Ui,
    target_noun: &str,
    selected: &mut Option<Color>,
    draft: &mut FolderColorDraftState,
    t: &Tokens,
) {
    let current = draft.current_color();
    let [r, g, b, _] = current.to_rgba8();
    ui.horizontal(|ui| {
        ui.set_height(28.0);
        ui.spacing_mut().item_spacing.x = 8.0;
        let (swatch_rect, _) = ui.allocate_exact_size(Vec2::new(26.0, 24.0), Sense::hover());
        ui.painter()
            .rect_filled(swatch_rect, 5.0, to_color32(current));
        ui.painter().rect_stroke(
            swatch_rect,
            5.0,
            Stroke::new(1.0, t.border),
            StrokeKind::Inside,
        );
        crate::theme::studio_row_label(
            ui,
            RichText::new("Hex").size(11.5).color(t.text_muted),
            28.0,
        );
        let hex_resp = ui.add_sized(
            [78.0, 26.0],
            egui::TextEdit::singleline(&mut draft.hex_draft)
                .desired_width(74.0)
                .char_limit(7)
                .font(egui::TextStyle::Monospace)
                .margin(Margin::symmetric(6, 2))
                .align(egui::Align2::LEFT_CENTER),
        );
        if hex_resp.changed()
            && let Some(parsed) = Color::from_hex(draft.hex_draft.trim())
        {
            let opaque = parsed.with_alpha(1.0);
            draft.hsv = opaque.to_hsv();
            *selected = Some(opaque);
        }
        if hex_resp.lost_focus() {
            draft.hex_draft = draft.current_color().to_hex();
        }
        crate::theme::studio_row_label(
            ui,
            RichText::new(format!("{r}, {g}, {b}"))
                .size(10.5)
                .monospace()
                .color(t.text_muted),
            28.0,
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if compact_icon_button(ui, Icon::Check, Some("Done"), false, true, t)
                .on_hover_text(format!(
                    "Collapse custom color controls while keeping this {target_noun} color"
                ))
                .clicked()
            {
                draft.custom_open = false;
            }
        });
    });
}

fn draw_validation(
    ui: &mut egui::Ui,
    name: &str,
    result: &Result<String, String>,
    verb: &str,
    t: &Tokens,
) {
    match result {
        Err(message) if !name.trim().is_empty() => {
            ui.label(RichText::new(message).size(11.0).color(t.danger_fg));
        }
        Ok(path) => {
            ui.label(
                RichText::new(format!("{verb}: Library / {}", display_folder_path(path)))
                    .size(11.0)
                    .color(t.text_muted),
            );
        }
        Err(_) => {
            ui.label(RichText::new(" ").size(11.0));
        }
    }
}

// ---------------------------------------------------------------------------
// New Folder
// ---------------------------------------------------------------------------

fn new_folder_validation(app: &PaperCraftApp) -> Result<String, String> {
    validate_folder_name(
        &app.view.folder_dialog_name,
        &app.view.folder_dialog_parent,
        &known_library_folders(app),
        None,
    )
}

/// Renders the New Folder dialog body. Returns `true` when the modal should close.
pub(crate) fn render_new_folder_body(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    let folders = known_library_folders(app);
    section_title(ui, "Name", "What should this folder be called?", t);
    let enter = draw_name_field(
        ui,
        &mut app.view.folder_dialog_name,
        &mut app.view.folder_dialog_focus_pending,
    );
    let result = new_folder_validation(app);
    draw_validation(
        ui,
        &app.view.folder_dialog_name,
        &result,
        "Will be created at",
        t,
    );

    ui.add_space(10.0);
    section_title(
        ui,
        "Location",
        "Keep it at the top of your Library, or place it inside another folder.",
        t,
    );
    draw_location_list(
        ui,
        &folders,
        &app.session.prefs,
        &mut app.view.folder_dialog_parent,
        None,
        t,
    );

    ui.add_space(10.0);
    draw_folder_color_section(
        ui,
        &mut app.view.folder_dialog_color,
        &mut app.view.folder_color_draft,
        t,
    );

    if let Some(title) = app
        .view
        .folder_dialog_move_notebook
        .and_then(|idx| app.session.notebooks.get(idx))
        .map(|doc| doc.title.clone())
    {
        ui.add_space(8.0);
        ui.label(
            RichText::new(format!("\"{title}\" will be moved into the new folder."))
                .size(11.5)
                .color(t.accent),
        );
    }
    enter && result.is_ok() && submit_new_folder(app)
}

/// Renders the New Folder dialog footer. Returns `true` when the modal should close.
pub(crate) fn render_new_folder_footer(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    let valid = new_folder_validation(app).is_ok();
    let mut close = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if primary_action_button(ui, "Create Folder", valid, t).clicked()
                && submit_new_folder(app)
            {
                close = true;
            }
            if secondary_action_button(ui, "Cancel", true, t).clicked() {
                close = true;
            }
        });
    });
    close
}

fn submit_new_folder(app: &mut PaperCraftApp) -> bool {
    let Ok(path) = new_folder_validation(app) else {
        return false;
    };
    let mut payload = json!({ "path": path });
    if let Some(color) = app.view.folder_dialog_color {
        payload["color"] = json!(color);
    }
    if app
        .session
        .execute("library.folder.create", &payload)
        .is_err()
    {
        return false;
    }
    if let Some(index) = app.view.folder_dialog_move_notebook.take() {
        let _ = move_notebook_to_folder(app, index, &path);
    }
    set_scope(app, LibraryScope::Folder(path));
    app.view.folder_dialog_name.clear();
    true
}

// ---------------------------------------------------------------------------
// Rename Folder
// ---------------------------------------------------------------------------

fn rename_validation(app: &PaperCraftApp) -> Result<String, String> {
    let target = &app.view.folder_dialog_target;
    if target.is_empty() {
        return Err("This folder no longer exists.".to_string());
    }
    if path_within(&app.view.folder_dialog_parent, target) {
        return Err("A folder can't be moved inside itself.".to_string());
    }
    validate_folder_name(
        &app.view.folder_dialog_name,
        &app.view.folder_dialog_parent,
        &known_library_folders(app),
        Some(target),
    )
}

/// Renders the Rename Folder dialog body. Returns `true` when the modal should close.
pub(crate) fn render_rename_folder_body(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    let folders = known_library_folders(app);
    let target = app.view.folder_dialog_target.clone();
    let summary = folder_tree(app)
        .into_iter()
        .find(|node| node.path.eq_ignore_ascii_case(&target))
        .map(|node| node.summary())
        .unwrap_or_default();

    section_title(
        ui,
        "Name",
        &format!("Currently \"{}\". {summary}", folder_leaf(&target)),
        t,
    );
    let enter = draw_name_field(
        ui,
        &mut app.view.folder_dialog_name,
        &mut app.view.folder_dialog_focus_pending,
    );
    let result = rename_validation(app);
    draw_validation(ui, &app.view.folder_dialog_name, &result, "New location", t);

    ui.add_space(10.0);
    section_title(
        ui,
        "Location",
        "Move the folder, with everything inside it, to another place.",
        t,
    );
    draw_location_list(
        ui,
        &folders,
        &app.session.prefs,
        &mut app.view.folder_dialog_parent,
        Some(&target),
        t,
    );

    ui.add_space(10.0);
    draw_folder_color_section(
        ui,
        &mut app.view.folder_dialog_color,
        &mut app.view.folder_color_draft,
        t,
    );
    enter && result.is_ok() && submit_rename_folder(app)
}

/// Renders the Rename Folder dialog footer. Returns `true` when the modal should close.
pub(crate) fn render_rename_folder_footer(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    let valid = rename_validation(app).is_ok();
    let mut close = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if primary_action_button(ui, "Save", valid, t).clicked() && submit_rename_folder(app) {
                close = true;
            }
            if secondary_action_button(ui, "Cancel", true, t).clicked() {
                close = true;
            }
        });
    });
    close
}

fn remap_path(path: &str, old_root: &str, new_root: &str) -> Option<String> {
    if !path_within(path, old_root) {
        return None;
    }
    let suffix = path.get(old_root.len()..).unwrap_or("");
    Some(format!("{new_root}{suffix}"))
}

fn submit_rename_folder(app: &mut PaperCraftApp) -> bool {
    let Ok(new_path) = rename_validation(app) else {
        return false;
    };
    let old_path = app.view.folder_dialog_target.clone();
    let desired_color = app.view.folder_dialog_color;
    let existing_color = app.session.prefs.folder_color(&old_path);
    let color_changed = desired_color != existing_color;
    if new_path == old_path {
        if !color_changed {
            return true;
        }
        return apply_folder_color_command(app, &old_path, desired_color);
    }
    let mut payload = json!({ "old_path": old_path, "new_path": new_path });
    if color_changed {
        match desired_color {
            Some(c) => payload["color"] = json!(c),
            None => payload["clear_color"] = json!(true),
        }
    }
    if app
        .session
        .execute("library.folder.rename", &payload)
        .is_err()
    {
        return false;
    }
    if let LibraryScope::Folder(open) = current_scope(app)
        && let Some(moved) = remap_path(&open, &old_path, &new_path)
    {
        set_scope(app, LibraryScope::Folder(moved));
    }
    for collapsed in &mut app.view.library_collapsed_folders {
        if let Some(moved) = remap_path(collapsed, &old_path, &new_path) {
            *collapsed = moved;
        }
    }
    true
}

fn apply_folder_color_command(app: &mut PaperCraftApp, path: &str, color: Option<Color>) -> bool {
    let payload = match color {
        Some(c) => json!({ "path": path, "color": c }),
        None => json!({ "path": path, "clear": true }),
    };
    app.session
        .execute("library.folder.setColor", &payload)
        .is_ok()
}

fn apply_tag_color_command(app: &mut PaperCraftApp, tag: &str, color: Option<Color>) -> bool {
    let payload = match color {
        Some(c) => json!({ "tag": tag, "color": c }),
        None => json!({ "tag": tag, "clear": true }),
    };
    app.session
        .execute("library.tag.setColor", &payload)
        .is_ok()
}

fn draw_color_preview_card(
    ui: &mut egui::Ui,
    icon: Icon,
    title: &str,
    subtitle: &str,
    color: Option<Color>,
    show_color_pill: bool,
    t: &Tokens,
) {
    let vis = folder_visual_colors(color, t);
    egui::Frame::NONE
        .fill(t.panel_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(CornerRadius::same(10))
        .inner_margin(Margin::same(12))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (badge, _) = ui.allocate_exact_size(Vec2::splat(36.0), Sense::hover());
                ui.painter()
                    .rect_filled(badge, CornerRadius::same(9), vis.soft_fill);
                paint_icon(ui.painter(), badge.shrink(9.0), icon, vis.icon);
                ui.vertical(|ui| {
                    ui.label(RichText::new(title).size(14.0).strong().color(t.text));
                    ui.label(RichText::new(subtitle).size(11.0).color(t.text_muted));
                });
                if show_color_pill {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let label = color.map_or_else(|| "Default".to_string(), Color::to_hex);
                        let pill_fg = if color.is_some() {
                            vis.selected_text
                        } else {
                            t.text_muted
                        };
                        let pill_fill = if color.is_some() {
                            vis.soft_fill
                        } else {
                            t.field_bg
                        };
                        egui::Frame::NONE
                            .fill(pill_fill)
                            .stroke(Stroke::new(1.0, t.border))
                            .corner_radius(CornerRadius::same(8))
                            .inner_margin(Margin::symmetric(8, 3))
                            .show(ui, |ui| {
                                ui.label(RichText::new(label).size(10.5).strong().color(pill_fg));
                            });
                    });
                }
            });
        });
}

fn draw_folder_preview_card(
    ui: &mut egui::Ui,
    path: &str,
    summary: &str,
    color: Option<Color>,
    show_color_pill: bool,
    t: &Tokens,
) {
    draw_color_preview_card(
        ui,
        Icon::Folder,
        folder_leaf(path),
        &format!("Library / {}  •  {summary}", display_folder_path(path)),
        color,
        show_color_pill,
        t,
    );
}

// ---------------------------------------------------------------------------
// Folder Color & Tag Color
// ---------------------------------------------------------------------------

/// Renders the Folder Color dialog body. Returns `true` when the modal should close.
pub(crate) fn render_folder_color_body(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    let target = app.view.folder_dialog_target.clone();
    let Some(node) = folder_tree(app)
        .into_iter()
        .find(|node| node.path.eq_ignore_ascii_case(&target))
    else {
        ui.label(
            RichText::new("This folder no longer exists.")
                .size(12.0)
                .color(t.text_muted),
        );
        return false;
    };
    draw_folder_preview_card(
        ui,
        &node.path,
        &node.summary(),
        app.view.folder_dialog_color,
        true,
        t,
    );
    ui.add_space(12.0);
    draw_folder_color_section(
        ui,
        &mut app.view.folder_dialog_color,
        &mut app.view.folder_color_draft,
        t,
    );
    false
}

fn render_color_footer(
    ui: &mut egui::Ui,
    enabled: bool,
    t: &Tokens,
    save: impl FnOnce() -> bool,
) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if primary_action_button(ui, "Save Color", enabled, t).clicked() && save() {
                close = true;
            }
            if secondary_action_button(ui, "Cancel", true, t).clicked() {
                close = true;
            }
        });
    });
    close
}

/// Renders the Folder Color dialog footer. Returns `true` when the modal should close.
pub(crate) fn render_folder_color_footer(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    let exists = !app.view.folder_dialog_target.is_empty();
    render_color_footer(ui, exists, t, || submit_folder_color(app))
}

fn submit_folder_color(app: &mut PaperCraftApp) -> bool {
    let target = app.view.folder_dialog_target.clone();
    if target.is_empty() {
        return false;
    }
    apply_folder_color_command(app, &target, app.view.folder_dialog_color)
}

/// Renders the Tag Color dialog body. Returns `true` when the modal should close.
pub(crate) fn render_tag_color_body(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    let target = app.view.tag_dialog_target.clone();
    if target.is_empty() {
        ui.label(
            RichText::new("No tag selected.")
                .size(12.0)
                .color(t.text_muted),
        );
        return false;
    }
    let notebook_count = app
        .session
        .notebooks
        .iter()
        .filter(|doc| doc.tags.iter().any(|tag| tag.eq_ignore_ascii_case(&target)))
        .count();
    draw_color_preview_card(
        ui,
        Icon::Tag,
        &format!("#{target}"),
        &format!(
            "Tags  •  {}",
            plural(notebook_count, "notebook", "notebooks")
        ),
        app.view.tag_dialog_color,
        true,
        t,
    );
    ui.add_space(12.0);
    draw_color_section(
        ui,
        "tag",
        "tag_color_studio_collapse",
        &mut app.view.tag_dialog_color,
        &mut app.view.tag_color_draft,
        t,
    );
    false
}

/// Renders the Tag Color dialog footer. Returns `true` when the modal should close.
pub(crate) fn render_tag_color_footer(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    let exists = !app.view.tag_dialog_target.is_empty();
    render_color_footer(ui, exists, t, || submit_tag_color(app))
}

fn submit_tag_color(app: &mut PaperCraftApp) -> bool {
    let target = app.view.tag_dialog_target.clone();
    if target.is_empty() {
        return false;
    }
    apply_tag_color_command(app, &target, app.view.tag_dialog_color)
}

// ---------------------------------------------------------------------------
// Delete Folder
// ---------------------------------------------------------------------------

/// Renders the Delete Folder confirmation body.
pub(crate) fn render_delete_folder_body(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    let target = app.view.folder_dialog_target.clone();
    let Some(node) = folder_tree(app)
        .into_iter()
        .find(|node| node.path.eq_ignore_ascii_case(&target))
    else {
        ui.label(
            RichText::new("This folder no longer exists.")
                .size(12.0)
                .color(t.text_muted),
        );
        return false;
    };
    let parent = folder_parent(&node.path);
    let destination = if parent.is_empty() {
        "Unfiled".to_string()
    } else {
        display_folder_path(parent)
    };

    draw_folder_preview_card(ui, &node.path, &node.summary(), node.color, false, t);
    ui.add_space(10.0);

    let notebooks_line = if node.total_notebooks == 0 {
        "There are no notebooks inside, so nothing else changes.".to_string()
    } else {
        format!(
            "{} inside (including subfolders) will move to \"{destination}\". No notebook is deleted.",
            plural(node.total_notebooks, "notebook", "notebooks")
        )
    };
    bullet(ui, Icon::Notebook, &notebooks_line, t);
    if node.subfolders > 0 {
        let nested = folder_tree(app)
            .iter()
            .filter(|other| {
                path_within(&other.path, &node.path) && !other.path.eq_ignore_ascii_case(&node.path)
            })
            .count();
        bullet(
            ui,
            Icon::Folder,
            &format!(
                "{} inside it will be removed as well.",
                plural(nested, "subfolder", "subfolders")
            ),
            t,
        );
    }
    false
}

fn bullet(ui: &mut egui::Ui, icon: Icon, text: &str, t: &Tokens) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        let (rect, _) = ui.allocate_exact_size(Vec2::splat(15.0), Sense::hover());
        paint_icon(ui.painter(), rect, icon, t.text_muted);
        ui.add(egui::Label::new(RichText::new(text).size(12.0).color(t.text)).wrap());
    });
}

/// Renders the Delete Folder confirmation footer.
pub(crate) fn render_delete_folder_footer(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
) -> bool {
    let exists = !app.view.folder_dialog_target.is_empty();
    let mut close = false;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if danger_action_button(ui, "Delete Folder", exists, t).clicked()
                && submit_delete_folder(app)
            {
                close = true;
            }
            if secondary_action_button(ui, "Cancel", true, t).clicked() {
                close = true;
            }
        });
    });
    close
}

fn submit_delete_folder(app: &mut PaperCraftApp) -> bool {
    let target = app.view.folder_dialog_target.clone();
    if target.is_empty() {
        return false;
    }
    let parent = folder_parent(&target).to_string();
    if app
        .session
        .execute(
            "library.folder.delete",
            &json!({ "path": target, "move_notebooks_to": parent }),
        )
        .is_err()
    {
        return false;
    }
    if let LibraryScope::Folder(open) = current_scope(app)
        && path_within(&open, &target)
    {
        let next = if parent.is_empty() {
            LibraryScope::All
        } else {
            LibraryScope::Folder(parent)
        };
        set_scope(app, next);
    }
    app.view
        .library_collapsed_folders
        .retain(|collapsed| !path_within(collapsed, &target));
    app.view.folder_dialog_target.clear();
    true
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn new_folder_dialog_creates_nested_folder_moves_notebook_and_opens_it() {
        let mut app = PaperCraftApp::default();
        app.session
            .execute("library.folder.create", &json!({ "path": "School" }))
            .unwrap();
        open_new_folder(&mut app, "School", Some(0));
        assert_eq!(app.view.active_modal, Some(ActiveModal::NewFolder));
        assert!(new_folder_validation(&app).is_err());

        app.view.folder_dialog_name = "school".to_string();
        app.view.folder_dialog_parent.clear();
        assert!(
            new_folder_validation(&app).is_err(),
            "duplicate at top level"
        );

        app.view.folder_dialog_parent = "School".to_string();
        app.view.folder_dialog_name = " Math ".to_string();
        assert!(submit_new_folder(&mut app));
        assert!(
            app.session
                .prefs
                .library_folders
                .contains(&"School/Math".to_string())
        );
        assert_eq!(app.session.notebooks[0].folder_path, "School/Math");
        assert_eq!(
            current_scope(&app),
            LibraryScope::Folder("School/Math".to_string())
        );
    }

    #[test]
    fn rename_dialog_moves_folder_and_keeps_open_view_in_sync() {
        let mut app = PaperCraftApp::default();
        app.session
            .execute(
                "notebook.create",
                &json!({ "title": "Calc", "folder_path": "School/Math" }),
            )
            .unwrap();
        set_scope(&mut app, LibraryScope::Folder("School/Math".to_string()));
        open_rename_folder(&mut app, "School");
        assert_eq!(app.view.folder_dialog_name, "School");
        assert_eq!(app.view.folder_dialog_parent, "");

        // A folder can never move into its own subtree.
        app.view.folder_dialog_parent = "School/Math".to_string();
        assert!(rename_validation(&app).is_err());

        app.view.folder_dialog_parent.clear();
        app.view.folder_dialog_name = "College".to_string();
        assert!(submit_rename_folder(&mut app));
        assert_eq!(app.session.notebooks[1].folder_path, "College/Math");
        assert_eq!(
            current_scope(&app),
            LibraryScope::Folder("College/Math".to_string())
        );
    }

    #[test]
    fn delete_dialog_moves_notebooks_to_parent_and_leaves_deleted_view() {
        let mut app = PaperCraftApp::default();
        app.session
            .execute(
                "notebook.create",
                &json!({ "title": "Calc", "folder_path": "School/Math/Calc" }),
            )
            .unwrap();
        set_scope(
            &mut app,
            LibraryScope::Folder("School/Math/Calc".to_string()),
        );
        open_delete_folder(&mut app, "School/Math");
        assert_eq!(app.view.active_modal, Some(ActiveModal::DeleteFolder));
        assert!(submit_delete_folder(&mut app));
        assert_eq!(app.session.notebooks[1].folder_path, "School");
        assert_eq!(
            current_scope(&app),
            LibraryScope::Folder("School".to_string())
        );
        assert!(
            !app.session
                .prefs
                .library_folders
                .iter()
                .any(|f| f.starts_with("School/Math"))
        );
    }

    #[test]
    fn folder_dialogs_render_without_panicking() {
        let mut app = PaperCraftApp::default();
        app.session
            .execute(
                "notebook.create",
                &json!({ "title": "Calc", "folder_path": "School/Math", "tags": ["exam"] }),
            )
            .unwrap();
        let t = Tokens::light();
        for open in [
            |app: &mut PaperCraftApp| open_new_folder(app, "School", None),
            |app: &mut PaperCraftApp| open_rename_folder(app, "School/Math"),
            |app: &mut PaperCraftApp| open_delete_folder(app, "School"),
            |app: &mut PaperCraftApp| {
                open_folder_color(app, "School/Math");
                app.view.folder_color_draft.custom_open = true;
            },
            |app: &mut PaperCraftApp| {
                open_tag_color(app, "exam");
                app.view.tag_color_draft.custom_open = true;
            },
        ] {
            open(&mut app);
            let ctx = egui::Context::default();
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
                    match app.view.active_modal {
                        Some(ActiveModal::NewFolder) => {
                            let _ = render_new_folder_body(&mut app, ui, &t);
                            let _ = render_new_folder_footer(&mut app, ui, &t);
                        }
                        Some(ActiveModal::RenameFolder) => {
                            let _ = render_rename_folder_body(&mut app, ui, &t);
                            let _ = render_rename_folder_footer(&mut app, ui, &t);
                        }
                        Some(ActiveModal::DeleteFolder) => {
                            let _ = render_delete_folder_body(&mut app, ui, &t);
                            let _ = render_delete_folder_footer(&mut app, ui, &t);
                        }
                        Some(ActiveModal::FolderColor) => {
                            let _ = render_folder_color_body(&mut app, ui, &t);
                            let _ = render_folder_color_footer(&mut app, ui, &t);
                        }
                        Some(ActiveModal::TagColor) => {
                            let _ = render_tag_color_body(&mut app, ui, &t);
                            let _ = render_tag_color_footer(&mut app, ui, &t);
                        }
                        _ => {}
                    }
                });
                out.drop_without_applying_deltas();
            }
        }
    }

    #[test]
    fn folder_color_workflows_persist_across_create_rename_and_color_dialog() {
        let mut app = PaperCraftApp::default();
        let teal = Color::from_hex("#0D9488").unwrap();
        let rose = Color::from_hex("#DB2777").unwrap();

        open_new_folder(&mut app, "", None);
        app.view.folder_dialog_name = "Biology".to_string();
        app.view.folder_dialog_color = Some(teal);
        assert!(submit_new_folder(&mut app));
        assert_eq!(app.session.prefs.folder_color("Biology"), Some(teal));

        // Rename dialog updates color even without changing name or parent.
        open_rename_folder(&mut app, "Biology");
        assert_eq!(app.view.folder_dialog_color, Some(teal));
        app.view.folder_dialog_color = Some(rose);
        assert!(submit_rename_folder(&mut app));
        assert_eq!(app.session.prefs.folder_color("Biology"), Some(rose));

        // Folder Color dialog sets and clears color cleanly.
        open_folder_color(&mut app, "Biology");
        assert_eq!(app.view.active_modal, Some(ActiveModal::FolderColor));
        assert_eq!(app.view.folder_dialog_color, Some(rose));
        app.view.folder_dialog_color = None;
        assert!(submit_folder_color(&mut app));
        assert_eq!(app.session.prefs.folder_color("Biology"), None);
    }

    #[test]
    fn tag_color_dialog_sets_updates_and_clears_color_cleanly() {
        let mut app = PaperCraftApp::default();
        let blue = Color::from_hex("#2563EB").unwrap();
        let amber = Color::from_hex("#D97706").unwrap();

        open_tag_color(&mut app, "#example");
        assert_eq!(app.view.active_modal, Some(ActiveModal::TagColor));
        assert_eq!(app.view.tag_dialog_target, "example");
        assert_eq!(app.view.tag_dialog_color, None);
        app.view.tag_dialog_color = Some(blue);
        assert!(submit_tag_color(&mut app));
        assert_eq!(app.session.prefs.tag_color("example"), Some(blue));

        open_tag_color(&mut app, "Example");
        assert_eq!(app.view.tag_dialog_color, Some(blue));
        app.view.tag_dialog_color = Some(amber);
        assert!(submit_tag_color(&mut app));
        assert_eq!(app.session.prefs.tag_color("EXAMPLE"), Some(amber));

        open_tag_color(&mut app, "example");
        app.view.tag_dialog_color = None;
        assert!(submit_tag_color(&mut app));
        assert_eq!(app.session.prefs.tag_color("example"), None);
    }
}
