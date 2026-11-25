//! Library organization for the Notebooks home screen: the folder model, the
//! left navigation sidebar (Library views, folder tree, tags), subfolder tiles,
//! folder action menus, notebook drag-and-drop, and the notebook folder picker.
//!
//! Interaction model:
//! - Exactly one *place* is open at a time ([`LibraryScope`]): All Notebooks,
//!   Favorites, Unfiled, Trash, a folder, or a tag.
//! - Opening a folder shows its subfolders as tiles plus the notebooks stored
//!   directly inside it, like a file browser.
//! - Notebooks move by dragging them onto any folder (sidebar row or tile), or
//!   through the "Move to Folder" menu on every notebook.
//! - Folder actions (new subfolder, rename/move, delete) live in the `...` menu
//!   on each folder row, its right-click menu, and the open folder's header.

use egui::{
    Color32, CornerRadius, FontId, Margin, Rect, RichText, Sense, Stroke, StrokeKind, Vec2, pos2,
};
use papercraft_color::{Color, contrast_ratio, relative_luminance};
use papercraft_doc::{
    FOLDER_COLOR_PRESETS, MAX_FOLDER_DEPTH, MAX_FOLDER_PATH_CHARS, MAX_LIBRARY_FOLDERS,
    UserStudioConfig, sanitize_library_folders,
};
use serde_json::json;

use crate::PaperCraftApp;
use crate::icons::{Icon, compact_icon_button, paint_icon};
use crate::modal::{self, ActiveModal};
use crate::theme::{Tokens, to_color32};

/// Fixed width of the Library navigation sidebar.
pub(crate) const SIDEBAR_WIDTH: f32 = 236.0;
/// Below this window width the sidebar collapses into a "Browse" menu button.
pub(crate) const SIDEBAR_MIN_VIEWPORT: f32 = 760.0;
/// Maximum characters allowed in a single folder name.
pub(crate) const MAX_FOLDER_NAME_CHARS: usize = 48;

const ROW_HEIGHT: f32 = 30.0;
const INDENT: f32 = 14.0;
const TILE_HEIGHT: f32 = 64.0;
const SIDEBAR_FOOTER_HEIGHT: f32 = 28.0;

// ---------------------------------------------------------------------------
// Folder paths
// ---------------------------------------------------------------------------

/// Parent path of `path` (`""` for top-level folders).
pub(crate) fn folder_parent(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

/// Last segment (display name) of `path`.
pub(crate) fn folder_leaf(path: &str) -> &str {
    path.rsplit_once('/').map_or(path, |(_, leaf)| leaf)
}

/// Number of segments in `path` (`0` for the top level).
pub(crate) fn folder_depth(path: &str) -> usize {
    if path.is_empty() {
        0
    } else {
        path.split('/').count()
    }
}

/// Joins a parent path and a single folder name.
pub(crate) fn join_folder(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_string()
    } else {
        format!("{parent}/{name}")
    }
}

/// Human-friendly path such as `School / Math`.
pub(crate) fn display_folder_path(path: &str) -> String {
    path.replace('/', " / ")
}

/// Returns `true` when `candidate` is `ancestor` itself or nested anywhere inside it.
pub(crate) fn path_within(candidate: &str, ancestor: &str) -> bool {
    if candidate.is_empty() || ancestor.is_empty() {
        return false;
    }
    if candidate.eq_ignore_ascii_case(ancestor) {
        return true;
    }
    let head_matches = candidate
        .get(..ancestor.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(ancestor));
    head_matches && candidate.as_bytes().get(ancestor.len()) == Some(&b'/')
}

/// Every folder known to the library (explicit folders plus folders referenced by
/// notebooks), parents included, sorted as a depth-first tree.
pub(crate) fn known_library_folders(app: &PaperCraftApp) -> Vec<String> {
    let mut raw = app.session.prefs.library_folders.clone();
    raw.extend(
        app.session
            .notebooks
            .iter()
            .filter(|doc| !doc.folder_path.is_empty())
            .map(|doc| doc.folder_path.clone()),
    );
    let mut folders = sanitize_library_folders(&raw);
    folders.sort_by_cached_key(|path| {
        path.split('/')
            .map(str::to_lowercase)
            .collect::<Vec<String>>()
    });
    folders
}

/// Checks a folder name typed by the user and returns the full path it would create.
///
/// `ignore` is the folder being renamed, so it never collides with itself.
pub(crate) fn validate_folder_name(
    name: &str,
    parent: &str,
    existing: &[String],
    ignore: Option<&str>,
) -> Result<String, String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Type a name for the folder.".to_string());
    }
    if trimmed.contains('/') || trimmed.contains('\\') {
        return Err("Folder names can't contain / or \\ characters.".to_string());
    }
    if trimmed == "." || trimmed == ".." || trimmed.chars().any(char::is_control) {
        return Err("Choose a different folder name.".to_string());
    }
    if trimmed.chars().count() > MAX_FOLDER_NAME_CHARS {
        return Err(format!(
            "Keep folder names to {MAX_FOLDER_NAME_CHARS} characters or fewer."
        ));
    }
    if folder_depth(parent) + 1 > MAX_FOLDER_DEPTH {
        return Err(format!(
            "Folders can be nested up to {MAX_FOLDER_DEPTH} levels deep."
        ));
    }
    let path = join_folder(parent, trimmed);
    if path.chars().count() > MAX_FOLDER_PATH_CHARS {
        return Err(
            "This location is nested too deeply. Pick a shorter name or place.".to_string(),
        );
    }
    let is_ignored = |folder: &str| ignore.is_some_and(|skip| skip.eq_ignore_ascii_case(folder));
    if existing
        .iter()
        .any(|folder| folder.eq_ignore_ascii_case(&path) && !is_ignored(folder))
    {
        return Err(format!("A folder named \"{trimmed}\" already exists here."));
    }
    if ignore.is_none() && existing.len() >= MAX_LIBRARY_FOLDERS {
        return Err(format!(
            "You have reached the limit of {MAX_LIBRARY_FOLDERS} folders."
        ));
    }
    Ok(path)
}

// ---------------------------------------------------------------------------
// Folder color styling (theme-safe contrast for custom folder tints)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
pub(crate) struct FolderVisualColors {
    pub(crate) icon: Color32,
    pub(crate) soft_fill: Color32,
    pub(crate) selected_text: Color32,
}

fn color32_to_doc(c: Color32) -> Color {
    Color::from_rgba8(c.r(), c.g(), c.b(), c.a())
}

fn ensure_contrast(color: Color, bg: Color, min_ratio: f32) -> Color {
    let c = color.sanitized().with_alpha(1.0);
    if contrast_ratio(c, bg) >= min_ratio {
        return c;
    }
    let [h, s, v] = c.to_hsv();
    let bg_is_dark = relative_luminance(bg) < 0.25;
    let eval = |t: f32| -> Color {
        if bg_is_dark {
            let next_v = v + (1.0 - v) * (t * 2.0).min(1.0);
            let desat = ((t - 0.5) * 2.0).clamp(0.0, 1.0);
            Color::from_hsv(h, s * (1.0 - desat), next_v, 1.0)
        } else {
            Color::from_hsv(h, s, v * (1.0 - t), 1.0)
        }
    };
    let mut lo = 0.0_f32;
    let mut hi = 1.0_f32;
    let mut best = eval(1.0);
    for _ in 0..14 {
        let mid = 0.5 * (lo + hi);
        let cand = eval(mid);
        if contrast_ratio(cand, bg) >= min_ratio {
            best = cand;
            hi = mid;
        } else {
            lo = mid;
        }
    }
    best
}

pub(crate) fn folder_visual_colors(custom: Option<Color>, t: &Tokens) -> FolderVisualColors {
    let Some(raw) = custom else {
        return FolderVisualColors {
            icon: t.accent,
            soft_fill: t.accent_soft,
            selected_text: t.accent,
        };
    };
    let surface = color32_to_doc(t.toolbar_bg);
    let icon_doc = ensure_contrast(raw, surface, 3.1);
    let icon = to_color32(icon_doc);
    let soft_alpha = if t.is_dark { 0.22 } else { 0.14 };
    let soft_fill = crate::motion::lerp_color(t.field_bg, icon, soft_alpha);
    let soft_doc = color32_to_doc(soft_fill);
    let text_doc = ensure_contrast(raw, soft_doc, 4.5);
    FolderVisualColors {
        icon,
        soft_fill,
        selected_text: to_color32(text_doc),
    }
}

// ---------------------------------------------------------------------------
// Folder tree model
// ---------------------------------------------------------------------------

/// One folder in the library tree with its notebook counts.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FolderNode {
    pub(crate) path: String,
    /// `0` for top-level folders.
    pub(crate) depth: usize,
    /// Number of direct subfolders.
    pub(crate) subfolders: usize,
    /// Notebooks stored directly in this folder.
    pub(crate) direct_notebooks: usize,
    /// Notebooks stored in this folder or any subfolder.
    pub(crate) total_notebooks: usize,
    /// Optional custom display color assigned by the user.
    pub(crate) color: Option<Color>,
}

impl FolderNode {
    pub(crate) fn name(&self) -> &str {
        folder_leaf(&self.path)
    }

    /// Short summary such as `3 notebooks · 1 folder`.
    pub(crate) fn summary(&self) -> String {
        let mut parts = Vec::new();
        if self.total_notebooks > 0 {
            parts.push(plural(self.total_notebooks, "notebook", "notebooks"));
        }
        if self.subfolders > 0 {
            parts.push(plural(self.subfolders, "folder", "folders"));
        }
        if parts.is_empty() {
            "Empty folder".to_string()
        } else {
            parts.join(" · ")
        }
    }
}

pub(crate) fn plural(count: usize, one: &str, many: &str) -> String {
    if count == 1 {
        format!("1 {one}")
    } else {
        format!("{count} {many}")
    }
}

/// Builds the full folder tree in depth-first display order.
pub(crate) fn folder_tree(app: &PaperCraftApp) -> Vec<FolderNode> {
    let folders = known_library_folders(app);
    folders
        .iter()
        .map(|path| FolderNode {
            path: path.clone(),
            depth: folder_depth(path).saturating_sub(1),
            subfolders: folders
                .iter()
                .filter(|other| folder_parent(other).eq_ignore_ascii_case(path))
                .count(),
            direct_notebooks: app
                .session
                .notebooks
                .iter()
                .filter(|doc| doc.folder_path.eq_ignore_ascii_case(path))
                .count(),
            total_notebooks: app
                .session
                .notebooks
                .iter()
                .filter(|doc| path_within(&doc.folder_path, path))
                .count(),
            color: app.session.prefs.folder_color(path),
        })
        .collect()
}

/// Direct subfolders of `parent` (`""` = top level).
pub(crate) fn subfolders_of(tree: &[FolderNode], parent: &str) -> Vec<FolderNode> {
    tree.iter()
        .filter(|node| folder_parent(&node.path).eq_ignore_ascii_case(parent))
        .cloned()
        .collect()
}

fn is_collapsed(app: &PaperCraftApp, path: &str) -> bool {
    app.view
        .library_collapsed_folders
        .iter()
        .any(|collapsed| collapsed.eq_ignore_ascii_case(path))
}

fn hidden_by_collapsed_ancestor(app: &PaperCraftApp, path: &str) -> bool {
    let mut current = folder_parent(path);
    while !current.is_empty() {
        if is_collapsed(app, current) {
            return true;
        }
        current = folder_parent(current);
    }
    false
}

// ---------------------------------------------------------------------------
// Library scope (the place currently open in the library)
// ---------------------------------------------------------------------------

/// The single place currently open in the Library.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LibraryScope {
    All,
    Favorites,
    Unfiled,
    Trash,
    Folder(String),
    Tag(String),
}

impl LibraryScope {
    pub(crate) fn motion_key(&self) -> String {
        match self {
            Self::All => "all".to_string(),
            Self::Favorites => "favorites".to_string(),
            Self::Unfiled => "unfiled".to_string(),
            Self::Trash => "trash".to_string(),
            Self::Folder(path) => format!("folder:{}", path.to_lowercase()),
            Self::Tag(tag) => format!("tag:{}", tag.to_lowercase()),
        }
    }

    pub(crate) fn title(&self) -> String {
        match self {
            Self::All => "All Notebooks".to_string(),
            Self::Favorites => "Favorites".to_string(),
            Self::Unfiled => "Unfiled".to_string(),
            Self::Trash => "Trash".to_string(),
            Self::Folder(path) => folder_leaf(path).to_string(),
            Self::Tag(tag) => format!("#{tag}"),
        }
    }

    pub(crate) fn icon(&self) -> Icon {
        match self {
            Self::All => Icon::Library,
            Self::Favorites => Icon::Star,
            Self::Unfiled => Icon::Inbox,
            Self::Trash => Icon::Trash,
            Self::Folder(_) => Icon::FolderOpen,
            Self::Tag(_) => Icon::Tag,
        }
    }

    pub(crate) fn description(&self) -> &'static str {
        match self {
            Self::All => {
                "Every notebook in your library. Drag a notebook onto a folder in the sidebar to file it."
            }
            Self::Favorites => {
                "Notebooks you starred. Click the star on any notebook to pin it here."
            }
            Self::Unfiled => {
                "Notebooks that are not in a folder yet. Drag one onto a folder to organize it."
            }
            Self::Trash => {
                "Deleted notebooks stay in Trash for 30 days and are removed automatically when they expire. Restore a notebook any time before then."
            }
            Self::Folder(_) => {
                "Open a subfolder or notebook below. Drag notebooks onto any folder to move them."
            }
            Self::Tag(_) => "Notebooks labeled with this tag, from every folder.",
        }
    }
}

/// Reads the place currently open in the Library from view state.
pub(crate) fn current_scope(app: &PaperCraftApp) -> LibraryScope {
    if app.view.library_show_trash {
        return LibraryScope::Trash;
    }
    match app.view.library_folder_filter.as_deref() {
        Some("") => return LibraryScope::Unfiled,
        Some(path) => return LibraryScope::Folder(path.to_string()),
        None => {}
    }
    if app.view.library_favorites_only {
        return LibraryScope::Favorites;
    }
    if let Some(tag) = &app.view.library_tag_filter {
        return LibraryScope::Tag(tag.clone());
    }
    LibraryScope::All
}

/// Opens `scope`, replacing whichever place was open before.
pub(crate) fn set_scope(app: &mut PaperCraftApp, scope: LibraryScope) {
    app.view.library_show_trash = false;
    app.view.library_favorites_only = false;
    app.view.library_folder_filter = None;
    app.view.library_tag_filter = None;
    match scope {
        LibraryScope::All => {}
        LibraryScope::Favorites => app.view.library_favorites_only = true,
        LibraryScope::Unfiled => app.view.library_folder_filter = Some(String::new()),
        LibraryScope::Trash => app.view.library_show_trash = true,
        LibraryScope::Folder(path) => {
            // Reveal the opened folder in the sidebar tree.
            app.view.library_collapsed_folders.retain(|collapsed| {
                !path_within(&path, collapsed) || collapsed.eq_ignore_ascii_case(&path)
            });
            app.view.library_folder_filter = Some(path);
        }
        LibraryScope::Tag(tag) => app.view.library_tag_filter = Some(tag),
    }
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

/// Payload carried while a notebook is dragged across the Library.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NotebookDragPayload {
    pub(crate) index: usize,
}

/// A deferred Library interaction, applied once drawing has finished.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum LibraryAction {
    Navigate(LibraryScope),
    ToggleCollapse(String),
    NewFolder {
        parent: String,
        move_notebook: Option<usize>,
    },
    RenameFolder(String),
    OpenFolderColor(String),
    SetFolderColor {
        path: String,
        color: Option<Color>,
    },
    DeleteFolder(String),
    OpenTagColor(String),
    SetTagColor {
        tag: String,
        color: Option<Color>,
    },
    MoveNotebook {
        index: usize,
        folder: String,
    },
    FavoriteNotebook(usize),
    TrashNotebook(usize),
}

/// Applies a deferred Library interaction through engine commands and dialogs.
pub(crate) fn apply_library_action(app: &mut PaperCraftApp, action: LibraryAction) {
    match action {
        LibraryAction::Navigate(scope) => set_scope(app, scope),
        LibraryAction::ToggleCollapse(path) => {
            let before = app.view.library_collapsed_folders.len();
            app.view
                .library_collapsed_folders
                .retain(|collapsed| !collapsed.eq_ignore_ascii_case(&path));
            if app.view.library_collapsed_folders.len() == before {
                app.view.library_collapsed_folders.push(path);
            }
        }
        LibraryAction::NewFolder {
            parent,
            move_notebook,
        } => crate::folder_modals::open_new_folder(app, &parent, move_notebook),
        LibraryAction::RenameFolder(path) => crate::folder_modals::open_rename_folder(app, &path),
        LibraryAction::OpenFolderColor(path) => {
            crate::folder_modals::open_folder_color(app, &path);
        }
        LibraryAction::SetFolderColor { path, color } => {
            let payload = match color {
                Some(c) => json!({ "path": path, "color": c }),
                None => json!({ "path": path, "clear": true }),
            };
            let _ = app.session.execute("library.folder.setColor", &payload);
        }
        LibraryAction::DeleteFolder(path) => crate::folder_modals::open_delete_folder(app, &path),
        LibraryAction::OpenTagColor(tag) => {
            crate::folder_modals::open_tag_color(app, &tag);
        }
        LibraryAction::SetTagColor { tag, color } => {
            let payload = match color {
                Some(c) => json!({ "tag": tag, "color": c }),
                None => json!({ "tag": tag, "clear": true }),
            };
            let _ = app.session.execute("library.tag.setColor", &payload);
        }
        LibraryAction::MoveNotebook { index, folder } => {
            let _ = move_notebook_to_folder(app, index, &folder);
        }
        LibraryAction::FavoriteNotebook(index) => {
            let _ = app.session.execute(
                "notebook.setMetadata",
                &json!({ "index": index, "favorite": true }),
            );
        }
        LibraryAction::TrashNotebook(index) => {
            if let Some(doc) = app.session.notebooks.get(index).cloned() {
                app.view.pending_delete_notebook = Some(doc);
                modal::open(app, ActiveModal::ConfirmDeleteNotebook);
            }
        }
    }
}

/// Moves notebook `index` into `folder` (`""` = Unfiled). Returns `true` if it moved.
pub(crate) fn move_notebook_to_folder(app: &mut PaperCraftApp, index: usize, folder: &str) -> bool {
    let Some(doc) = app.session.notebooks.get(index) else {
        return false;
    };
    if doc.folder_path == folder {
        return false;
    }
    app.session
        .execute(
            "notebook.setMetadata",
            &json!({ "index": index, "folder_path": folder }),
        )
        .is_ok()
}

// ---------------------------------------------------------------------------
// Shared drawing helpers
// ---------------------------------------------------------------------------

fn truncated_galley(
    ui: &egui::Ui,
    text: &str,
    size: f32,
    color: Color32,
    max_width: f32,
) -> std::sync::Arc<egui::Galley> {
    let mut job = egui::text::LayoutJob::single_section(
        text.to_string(),
        egui::TextFormat {
            font_id: FontId::proportional(size),
            color,
            ..Default::default()
        },
    );
    job.wrap.max_width = max_width.max(8.0);
    job.wrap.max_rows = 1;
    job.wrap.break_anywhere = true;
    job.wrap.overflow_character = Some('…');
    ui.painter().layout_job(job)
}

/// Full-width menu entry with a leading icon, used by folder and notebook menus.
pub(crate) fn menu_item(
    ui: &mut egui::Ui,
    icon: Icon,
    label: &str,
    enabled: bool,
    danger: bool,
    t: &Tokens,
) -> egui::Response {
    menu_item_tinted(ui, icon, None, label, enabled, danger, t)
}

fn menu_item_tinted(
    ui: &mut egui::Ui,
    icon: Icon,
    icon_color: Option<Color32>,
    label: &str,
    enabled: bool,
    danger: bool,
    t: &Tokens,
) -> egui::Response {
    let width = ui.available_width().clamp(180.0, 320.0);
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 28.0), sense);
    if ui.is_rect_visible(rect) {
        let hovered = enabled && resp.hovered();
        let fg = if !enabled {
            t.text_muted.gamma_multiply(0.55)
        } else if danger {
            t.danger_fg
        } else {
            t.text
        };
        let icon_fg = match (icon_color, enabled) {
            (Some(c), true) => c,
            (Some(c), false) => c.gamma_multiply(0.55),
            (None, _) => fg,
        };
        if hovered {
            let fill = if danger {
                t.danger_bg
            } else {
                t.border.gamma_multiply(0.45)
            };
            ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
        }
        let icon_rect =
            Rect::from_center_size(pos2(rect.left() + 15.0, rect.center().y), Vec2::splat(14.0));
        paint_icon(ui.painter(), icon_rect, icon, icon_fg);
        let galley = truncated_galley(ui, label, 12.5, fg, rect.width() - 38.0);
        ui.painter().galley(
            pos2(rect.left() + 30.0, rect.center().y - galley.size().y * 0.5),
            galley,
            fg,
        );
    }
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

fn menu_separator(ui: &mut egui::Ui, t: &Tokens) {
    ui.add_space(3.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 1.0), Sense::hover());
    ui.painter()
        .hline(rect.x_range(), rect.center().y, Stroke::new(1.0, t.border));
    ui.add_space(3.0);
}

fn same_rgb(a: Color, b: Color) -> bool {
    a.to_rgba8()[..3] == b.to_rgba8()[..3]
}

#[derive(Clone, Copy)]
enum QuickColorTarget<'a> {
    Folder(&'a str),
    Tag(&'a str),
}

impl QuickColorTarget<'_> {
    fn default_tooltip(self) -> &'static str {
        match self {
            Self::Folder(_) => "Default folder color",
            Self::Tag(_) => "Default tag color",
        }
    }

    fn action(self, color: Option<Color>) -> LibraryAction {
        match self {
            Self::Folder(path) => LibraryAction::SetFolderColor {
                path: path.to_string(),
                color,
            },
            Self::Tag(tag) => LibraryAction::SetTagColor {
                tag: tag.to_string(),
                color,
            },
        }
    }
}

fn draw_menu_quick_colors(
    ui: &mut egui::Ui,
    target: QuickColorTarget<'_>,
    current_color: Option<Color>,
    t: &Tokens,
    actions: &mut Vec<LibraryAction>,
) {
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.label(
            RichText::new("Quick Color")
                .size(10.5)
                .strong()
                .color(t.text_muted),
        );
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        ui.spacing_mut().item_spacing.x = 2.5;
        let (def_rect, def_resp) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
        if ui.is_rect_visible(def_rect) {
            let p = ui.painter();
            if current_color.is_none() {
                p.circle_stroke(def_rect.center(), 9.0, Stroke::new(1.8, t.accent));
            }
            p.circle_filled(def_rect.center(), 6.8, t.panel_bg);
            p.circle_stroke(def_rect.center(), 6.8, Stroke::new(1.0, t.border));
            let offset = Vec2::splat(4.0);
            p.line_segment(
                [def_rect.center() - offset, def_rect.center() + offset],
                Stroke::new(1.1, t.text_muted),
            );
        }
        if def_resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(target.default_tooltip())
            .clicked()
        {
            actions.push(target.action(None));
            ui.close();
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
            let active = current_color.is_some_and(|c| same_rgb(c, swatch));
            let (dot_rect, dot_resp) = ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
            if ui.is_rect_visible(dot_rect) {
                let p = ui.painter();
                if active {
                    p.circle_stroke(dot_rect.center(), 9.0, Stroke::new(1.8, t.accent));
                }
                p.circle_filled(dot_rect.center(), 6.8, to_color32(swatch));
                p.circle_stroke(dot_rect.center(), 6.8, Stroke::new(0.9, rim));
            }
            if dot_resp
                .on_hover_cursor(egui::CursorIcon::PointingHand)
                .on_hover_text(format!("{name} ({hex})"))
                .clicked()
            {
                actions.push(target.action(Some(swatch)));
                ui.close();
            }
        }
    });
    ui.add_space(2.0);
}

/// Contents of a folder's action menu (`...` button and right-click).
pub(crate) fn folder_menu_contents(
    ui: &mut egui::Ui,
    node: &FolderNode,
    can_add_folder: bool,
    t: &Tokens,
    actions: &mut Vec<LibraryAction>,
) {
    if menu_item(ui, Icon::FolderOpen, "Open", true, false, t).clicked() {
        actions.push(LibraryAction::Navigate(LibraryScope::Folder(
            node.path.clone(),
        )));
        ui.close();
    }
    let nested_ok = can_add_folder && node.depth + 1 < MAX_FOLDER_DEPTH;
    if menu_item(
        ui,
        Icon::FolderPlus,
        "New Subfolder...",
        nested_ok,
        false,
        t,
    )
    .clicked()
    {
        actions.push(LibraryAction::NewFolder {
            parent: node.path.clone(),
            move_notebook: None,
        });
        ui.close();
    }
    if menu_item(ui, Icon::FolderPen, "Rename or Move...", true, false, t).clicked() {
        actions.push(LibraryAction::RenameFolder(node.path.clone()));
        ui.close();
    }
    if menu_item_tinted(
        ui,
        Icon::Palette,
        node.color.map(|c| folder_visual_colors(Some(c), t).icon),
        "Folder Color...",
        true,
        false,
        t,
    )
    .clicked()
    {
        actions.push(LibraryAction::OpenFolderColor(node.path.clone()));
        ui.close();
    }
    menu_separator(ui, t);
    draw_menu_quick_colors(
        ui,
        QuickColorTarget::Folder(&node.path),
        node.color,
        t,
        actions,
    );
    menu_separator(ui, t);
    if menu_item(ui, Icon::Trash, "Delete Folder...", true, true, t).clicked() {
        actions.push(LibraryAction::DeleteFolder(node.path.clone()));
        ui.close();
    }
}

/// Contents of a tag's action menu (`...` button and right-click).
pub(crate) fn tag_menu_contents(
    ui: &mut egui::Ui,
    tag: &str,
    color: Option<Color>,
    t: &Tokens,
    actions: &mut Vec<LibraryAction>,
) {
    if menu_item(ui, Icon::Tag, "Open", true, false, t).clicked() {
        actions.push(LibraryAction::Navigate(LibraryScope::Tag(tag.to_string())));
        ui.close();
    }
    if menu_item_tinted(
        ui,
        Icon::Palette,
        color.map(|c| folder_visual_colors(Some(c), t).icon),
        "Tag Color...",
        true,
        false,
        t,
    )
    .clicked()
    {
        actions.push(LibraryAction::OpenTagColor(tag.to_string()));
        ui.close();
    }
    menu_separator(ui, t);
    draw_menu_quick_colors(ui, QuickColorTarget::Tag(tag), color, t, actions);
}

/// Contents of a notebook's "Move to Folder" menu.
pub(crate) fn move_to_folder_menu_contents(
    ui: &mut egui::Ui,
    index: usize,
    current_folder: &str,
    tree: &[FolderNode],
    t: &Tokens,
    actions: &mut Vec<LibraryAction>,
) {
    ui.label(
        RichText::new("Move to")
            .size(10.5)
            .strong()
            .color(t.text_muted),
    );
    ui.add_space(2.0);
    egui::ScrollArea::vertical()
        .id_salt(("papercraft_move_to_folder_scroll", index))
        .max_height(280.0)
        .auto_shrink([false, true])
        .show(ui, |ui| {
            let unfiled_here = current_folder.is_empty();
            if menu_item(ui, Icon::Inbox, "Unfiled", !unfiled_here, false, t)
                .on_hover_text("Take the notebook out of every folder")
                .clicked()
            {
                actions.push(LibraryAction::MoveNotebook {
                    index,
                    folder: String::new(),
                });
            }
            for node in tree {
                let here = node.path.eq_ignore_ascii_case(current_folder);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 0.0;
                    ui.add_space(node.depth as f32 * INDENT);
                    let icon = if here { Icon::FolderOpen } else { Icon::Folder };
                    let label = if here {
                        format!("{} (current)", node.name())
                    } else {
                        node.name().to_string()
                    };
                    let icon_color = node.color.map(|c| folder_visual_colors(Some(c), t).icon);
                    if menu_item_tinted(ui, icon, icon_color, &label, !here, false, t).clicked() {
                        actions.push(LibraryAction::MoveNotebook {
                            index,
                            folder: node.path.clone(),
                        });
                    }
                });
            }
        });
    menu_separator(ui, t);
    if menu_item(
        ui,
        Icon::FolderPlus,
        "New Folder...",
        tree.len() < MAX_LIBRARY_FOLDERS,
        false,
        t,
    )
    .on_hover_text("Create a folder and move this notebook into it")
    .clicked()
    {
        actions.push(LibraryAction::NewFolder {
            parent: String::new(),
            move_notebook: Some(index),
        });
    }
}

/// Floating label that follows the pointer while a notebook is dragged.
pub(crate) fn paint_drag_ghost(ctx: &egui::Context, title: &str, t: &Tokens) {
    let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) else {
        return;
    };
    ctx.set_cursor_icon(egui::CursorIcon::Grabbing);
    egui::Area::new(egui::Id::unique("papercraft_notebook_drag_ghost"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pos + Vec2::new(16.0, 12.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(t.field_bg)
                .stroke(Stroke::new(1.5, t.accent))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::symmetric(10, 6))
                .shadow(ctx.style_of(ctx.theme()).visuals.popup_shadow)
                .show(ui, |ui| {
                    ui.set_max_width(240.0);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        let (icon_rect, _) =
                            ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                        paint_icon(ui.painter(), icon_rect, Icon::Notebook, t.accent);
                        ui.add(
                            egui::Label::new(
                                RichText::new(title).size(12.5).strong().color(t.text),
                            )
                            .truncate(),
                        );
                    });
                    ui.label(
                        RichText::new("Drop on a folder to move it")
                            .size(10.5)
                            .color(t.text_muted),
                    );
                });
        });
}

// ---------------------------------------------------------------------------
// Sidebar navigation
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expander {
    None,
    Leaf,
    Collapsed,
    Expanded,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DropKind {
    None,
    Accept,
}

struct NavRowSpec<'a> {
    id: egui::Id,
    icon: Icon,
    label: &'a str,
    count: Option<usize>,
    depth: usize,
    selected: bool,
    expander: Expander,
    menu_tooltip: Option<&'a str>,
    drop: DropKind,
    muted: bool,
    item_color: Option<Color>,
}

struct NavRowResponse {
    row: egui::Response,
    chevron_clicked: bool,
    more: Option<egui::Response>,
    dropped: Option<usize>,
}

struct MoreSlot {
    rect: Rect,
    radius: u8,
    id: egui::Id,
    visible: bool,
    hovered: bool,
}

fn paint_more_slot(ui: &egui::Ui, slot: MoreSlot, t: &Tokens) -> f32 {
    let vis_t = crate::motion::hover_factor(ui.ctx(), slot.id.with("vis"), slot.visible);
    let hov_t = crate::motion::hover_factor(ui.ctx(), slot.id.with("hov"), slot.hovered);
    if vis_t > 0.02 {
        let p = ui.painter();
        if hov_t > 0.02 {
            p.rect_filled(
                slot.rect,
                CornerRadius::same(slot.radius),
                t.border.gamma_multiply(0.7 * vis_t * hov_t),
            );
        }
        paint_icon(
            p,
            Rect::from_center_size(slot.rect.center(), Vec2::splat(15.0)),
            Icon::More,
            t.text.gamma_multiply(vis_t),
        );
    }
    vis_t
}

fn nav_row(ui: &mut egui::Ui, spec: &NavRowSpec<'_>, t: &Tokens) -> NavRowResponse {
    let width = ui.available_width().max(120.0);
    let (rect, row) = ui.allocate_exact_size(Vec2::new(width, ROW_HEIGHT), Sense::click());
    let accepts_drop = spec.drop == DropKind::Accept;
    let drag_over = accepts_drop && row.dnd_hover_payload::<NotebookDragPayload>().is_some();
    let dropped = if accepts_drop {
        row.dnd_release_payload::<NotebookDragPayload>()
            .map(|payload| payload.index)
    } else {
        None
    };
    let pointer_inside = row.contains_pointer();

    let mut x = rect.left() + 6.0 + spec.depth as f32 * INDENT;
    let chevron_rect = (spec.expander != Expander::None).then(|| {
        let r = Rect::from_center_size(pos2(x + 7.0, rect.center().y), Vec2::splat(14.0));
        x += 16.0;
        r
    });
    let icon_rect = Rect::from_center_size(pos2(x + 8.0, rect.center().y), Vec2::splat(15.0));
    x += 22.0;
    let slot_rect = Rect::from_min_max(
        pos2(rect.right() - 28.0, rect.top() + 3.0),
        pos2(rect.right() - 3.0, rect.bottom() - 3.0),
    );

    let chevron_clicked = match (chevron_rect, spec.expander) {
        (Some(r), Expander::Collapsed | Expander::Expanded) => ui
            .interact(r.expand(4.0), spec.id.with("chevron"), Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked(),
        _ => false,
    };

    let more_id = spec.id.with("more");
    let menu_open =
        spec.menu_tooltip.is_some() && egui::Popup::is_id_open(ui.ctx(), more_id.with("popup"));
    let show_more = spec.menu_tooltip.is_some() && (pointer_inside || menu_open);
    let more = show_more.then(|| {
        let resp = ui
            .interact(slot_rect, more_id, Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        match spec.menu_tooltip {
            Some(tip) => resp.on_hover_text(tip),
            None => resp,
        }
    });

    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let hovered = pointer_inside || menu_open;
        let hover_t = crate::motion::hover_factor(
            ui.ctx(),
            spec.id.with("hov"),
            hovered && !spec.selected && !drag_over,
        );
        let select_t =
            crate::motion::hover_factor(ui.ctx(), spec.id.with("sel"), spec.selected || drag_over);
        let drag_t = crate::motion::hover_factor(ui.ctx(), spec.id.with("dnd"), drag_over);
        let vis = folder_visual_colors(spec.item_color, t);
        let rest_fill =
            crate::motion::lerp_color(Color32::TRANSPARENT, t.border.gamma_multiply(0.45), hover_t);
        let fill = crate::motion::lerp_color(rest_fill, vis.soft_fill, select_t);
        p.rect_filled(rect, CornerRadius::same(7), fill);
        if drag_t > 0.02 {
            p.rect_stroke(
                rect,
                CornerRadius::same(7),
                Stroke::new(1.5, vis.icon.gamma_multiply(drag_t)),
                StrokeKind::Inside,
            );
        }
        let rest_fg = if spec.muted { t.text_muted } else { t.text };
        let fg = crate::motion::lerp_color(rest_fg, vis.selected_text, select_t);
        let icon_fg = if spec.item_color.is_some() {
            crate::motion::lerp_color(vis.icon, vis.selected_text, select_t * 0.5)
        } else {
            fg
        };
        if let Some(r) = chevron_rect {
            let chevron_icon = match spec.expander {
                Expander::Expanded => Some(Icon::ChevronDown),
                Expander::Collapsed => Some(Icon::NextPage),
                Expander::None | Expander::Leaf => None,
            };
            if let Some(icon) = chevron_icon {
                paint_icon(p, r.shrink(1.0), icon, t.text_muted);
            }
        }
        paint_icon(p, icon_rect, spec.icon, icon_fg);

        let label_right = slot_rect.left() - 4.0;
        let galley = truncated_galley(ui, spec.label, 12.5, fg, label_right - x);
        p.galley(pos2(x, rect.center().y - galley.size().y * 0.5), galley, fg);

        let more_hovered = more.as_ref().is_some_and(egui::Response::hovered) || menu_open;
        let more_vis_t = paint_more_slot(
            ui,
            MoreSlot {
                rect: slot_rect,
                radius: 5,
                id: more_id,
                visible: show_more,
                hovered: more_hovered,
            },
            t,
        );
        if let Some(count) = spec.count.filter(|c| *c > 0)
            && more_vis_t < 0.98
        {
            let count_color = t.text_muted.gamma_multiply(1.0 - more_vis_t);
            let galley = ui.painter().layout_no_wrap(
                count.to_string(),
                FontId::proportional(11.0),
                count_color,
            );
            p.galley(
                pos2(
                    slot_rect.right() - 4.0 - galley.size().x,
                    rect.center().y - galley.size().y * 0.5,
                ),
                galley,
                count_color,
            );
        }
    }

    NavRowResponse {
        row: row.on_hover_cursor(egui::CursorIcon::PointingHand),
        chevron_clicked,
        more,
        dropped,
    }
}

fn section_header(ui: &mut egui::Ui, title: &str, add_tooltip: Option<&str>, t: &Tokens) -> bool {
    let width = ui.available_width().max(120.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::hover());
    let galley =
        ui.painter()
            .layout_no_wrap(title.to_string(), FontId::proportional(10.5), t.text_muted);
    ui.painter().galley(
        pos2(rect.left() + 8.0, rect.center().y - galley.size().y * 0.5),
        galley,
        t.text_muted,
    );
    let Some(tooltip) = add_tooltip else {
        return false;
    };
    let btn_rect = Rect::from_center_size(
        pos2(rect.right() - 14.0, rect.center().y),
        Vec2::splat(22.0),
    );
    let resp = ui
        .interact(
            btn_rect,
            ui.scope_id().with(("section_add", title)),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tooltip);
    let hover_t = crate::motion::hover_factor(ui.ctx(), resp.id.with("hov"), resp.hovered());
    if hover_t > 0.02 {
        ui.painter().rect_filled(
            btn_rect,
            CornerRadius::same(5),
            t.border.gamma_multiply(0.7 * hover_t),
        );
    }
    paint_icon(
        ui.painter(),
        btn_rect.shrink(4.0),
        Icon::Plus,
        crate::motion::lerp_color(t.text_muted, t.text, hover_t),
    );
    resp.clicked()
}

struct TagEntry {
    name: String,
    count: usize,
    color: Option<Color>,
}

/// Everything the navigation needs, captured up front so drawing never
/// holds a borrow of the app while actions are collected.
struct NavSnapshot {
    scope: LibraryScope,
    all: usize,
    favorites: usize,
    unfiled: usize,
    trash: usize,
    visible_folders: Vec<(FolderNode, bool)>,
    folder_count: usize,
    tags: Vec<TagEntry>,
}

impl NavSnapshot {
    fn capture(app: &PaperCraftApp) -> Self {
        let tree = folder_tree(app);
        let folder_count = tree.len();
        let visible_folders = tree
            .into_iter()
            .filter(|node| !hidden_by_collapsed_ancestor(app, &node.path))
            .map(|node| {
                let collapsed = is_collapsed(app, &node.path);
                (node, collapsed)
            })
            .collect();
        let notebooks = &app.session.notebooks;
        let tags = library_tags_with_counts(app)
            .into_iter()
            .map(|(name, count)| TagEntry {
                color: app.session.prefs.tag_color(&name),
                name,
                count,
            })
            .collect();
        Self {
            scope: current_scope(app),
            all: notebooks.len(),
            favorites: notebooks.iter().filter(|doc| doc.favorite).count(),
            unfiled: notebooks
                .iter()
                .filter(|doc| doc.folder_path.is_empty())
                .count(),
            trash: app.session.trashed_notebooks.len(),
            visible_folders,
            folder_count,
            tags,
        }
    }
}

/// All tags used by notebooks with how many notebooks carry each, sorted by name.
pub(crate) fn library_tags_with_counts(app: &PaperCraftApp) -> Vec<(String, usize)> {
    let mut tags: Vec<(String, usize)> = Vec::new();
    for doc in &app.session.notebooks {
        for tag in &doc.tags {
            if let Some(entry) = tags
                .iter_mut()
                .find(|(known, _)| known.eq_ignore_ascii_case(tag))
            {
                entry.1 += 1;
            } else {
                tags.push((tag.clone(), 1));
            }
        }
    }
    tags.sort_by_key(|(tag, _)| tag.to_lowercase());
    tags
}

/// Where the navigation is being drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NavMode {
    /// Docked sidebar on wide windows: full folder actions and drag-and-drop.
    Sidebar,
    /// "Browse" dropdown on narrow windows: navigation only.
    Popup,
}

/// Draws the docked Library sidebar. Returns its outer rectangle.
pub(crate) fn show_sidebar_panel(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) -> Rect {
    let frame = egui::Frame::NONE
        .fill(t.toolbar_bg)
        .stroke(Stroke::new(1.0, t.border))
        .inner_margin(Margin::symmetric(10, 12));
    crate::fixed_side_panel(
        ui,
        egui::Panel::left("papercraft_home_library_sidebar").drag_to_open(false),
        SIDEBAR_WIDTH,
        frame,
    )
    .show(ui, |ui| {
        draw_library_nav(app, ui, t, NavMode::Sidebar);
    })
    .response
    .rect
}

/// Draws the Library navigation (views, folder tree, tags) and applies the
/// resulting actions.
pub(crate) fn draw_library_nav(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
    mode: NavMode,
) {
    #[cfg(not(target_arch = "wasm32"))]
    let notice = {
        let status = app.updater.status().clone();
        sidebar_update_notice_spec(&status)
    };
    #[cfg(target_arch = "wasm32")]
    let notice: Option<SidebarUpdateNoticeSpec> = None;

    draw_library_nav_with_notice(app, ui, t, mode, notice.as_ref());
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SidebarUpdateNoticeKind {
    Available,
    Downloading,
    ReadyToInstall,
}

#[derive(Clone, Debug)]
struct SidebarUpdateNoticeSpec {
    kind: SidebarUpdateNoticeKind,
    title: String,
    subtitle: String,
    badge: String,
    tooltip: String,
    progress_frac: Option<f32>,
    indeterminate: bool,
}

#[cfg(not(target_arch = "wasm32"))]
fn sidebar_update_notice_spec(
    status: &papercraft_updater::UpdateStatus,
) -> Option<SidebarUpdateNoticeSpec> {
    use papercraft_updater::{UpdateStatus, format_bytes};

    match status {
        UpdateStatus::Available(u) => Some(SidebarUpdateNoticeSpec {
            kind: SidebarUpdateNoticeKind::Available,
            title: "Update available".to_string(),
            subtitle: format!("v{} · Click to review & install", u.version),
            badge: format!("v{}", u.version),
            tooltip: format!(
                "PaperCraft v{} is available (current: v{}). Click to review release notes and install, or check Settings -> System anytime.",
                u.version, u.current_version
            ),
            progress_frac: None,
            indeterminate: false,
        }),
        UpdateStatus::Downloading {
            update,
            downloaded_bytes,
            total_bytes,
        } => {
            let is_verifying = *total_bytes > 0 && *downloaded_bytes >= *total_bytes;
            let title = if is_verifying {
                "Verifying update..."
            } else {
                "Downloading update..."
            };
            let (badge, progress_frac, indeterminate, subtitle) = if *total_bytes > 0 {
                let frac = (*downloaded_bytes as f32 / *total_bytes as f32).clamp(0.0, 1.0);
                let pct = (frac * 100.0).round() as u32;
                (
                    format!("{pct}%"),
                    Some(frac),
                    false,
                    format!(
                        "{} / {}",
                        format_bytes(*downloaded_bytes),
                        format_bytes(*total_bytes)
                    ),
                )
            } else {
                (
                    format_bytes(*downloaded_bytes),
                    None,
                    true,
                    format!("v{} · Click for details", update.version),
                )
            };
            Some(SidebarUpdateNoticeSpec {
                kind: SidebarUpdateNoticeKind::Downloading,
                title: title.to_string(),
                subtitle,
                badge,
                tooltip: format!(
                    "Downloading PaperCraft v{}. Click to view progress or cancel.",
                    update.version
                ),
                progress_frac,
                indeterminate,
            })
        }
        UpdateStatus::ReadyToInstall { update, .. } => Some(SidebarUpdateNoticeSpec {
            kind: SidebarUpdateNoticeKind::ReadyToInstall,
            title: "Update ready".to_string(),
            subtitle: format!("Click to restart & install v{}", update.version),
            badge: format!("v{}", update.version),
            tooltip: format!(
                "PaperCraft v{} is verified and ready. Click to save open work and restart.",
                update.version
            ),
            progress_frac: None,
            indeterminate: false,
        }),
        _ => None,
    }
}

fn draw_library_nav_with_notice(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    t: &Tokens,
    mode: NavMode,
    notice: Option<&SidebarUpdateNoticeSpec>,
) {
    let snapshot = NavSnapshot::capture(app);
    let mut actions = Vec::new();
    let show_footer = mode == NavMode::Sidebar;
    let sidebar_notice = if show_footer { notice } else { None };
    let item_gap = ui.spacing().item_spacing.y;
    let notice_h_id = ui.make_persistent_id("papercraft_sidebar_update_notice_h");
    let estimated_notice_h = sidebar_notice.map_or(0.0, |spec| {
        let fallback = if spec.kind == SidebarUpdateNoticeKind::Downloading {
            66.0
        } else {
            54.0
        };
        ui.ctx()
            .data(|d| d.get_temp::<f32>(notice_h_id))
            .unwrap_or(fallback)
    });
    let scroll = egui::ScrollArea::vertical()
        .id_salt(("papercraft_library_nav_scroll", mode == NavMode::Popup));
    let scroll = match mode {
        NavMode::Sidebar => {
            let notice_reserve = if sidebar_notice.is_some() {
                estimated_notice_h + item_gap
            } else {
                0.0
            };
            let max_h = (ui.available_height() - SIDEBAR_FOOTER_HEIGHT - item_gap - notice_reserve)
                .max(60.0);
            scroll.auto_shrink([false, false]).max_height(max_h)
        }
        NavMode::Popup => scroll.auto_shrink([false, true]).max_height(440.0),
    };
    scroll.show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        draw_library_views(ui, &snapshot, mode, t, &mut actions);
        ui.add_space(14.0);
        draw_folder_section(ui, &snapshot, mode, t, &mut actions);
        if !snapshot.tags.is_empty() {
            ui.add_space(14.0);
            draw_tag_section(ui, &snapshot, mode, t, &mut actions);
        }
        ui.add_space(8.0);
    });
    for action in actions {
        apply_library_action(app, action);
    }
    if show_footer {
        if let Some(spec) = sidebar_notice {
            let modal_open = app.view.active_modal == Some(ActiveModal::AppUpdate);
            let (clicked, rect) = draw_sidebar_update_notice(ui, spec, modal_open, t);
            ui.ctx()
                .data_mut(|d| d.insert_temp(notice_h_id, rect.height()));
            if clicked {
                modal::open(app, ActiveModal::AppUpdate);
            }
        }
        if draw_sidebar_footer(ui, t) {
            crate::open_external_url(app, ui.ctx(), crate::GITHUB_REPO_URL);
        }
    }
}

fn draw_sidebar_update_notice(
    ui: &mut egui::Ui,
    spec: &SidebarUpdateNoticeSpec,
    active: bool,
    t: &Tokens,
) -> (bool, Rect) {
    let notice_id = ui.make_persistent_id("papercraft_sidebar_update_notice");
    let was_hovered = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(notice_id.with("prev_hov")))
        .unwrap_or(false);
    let hover_t =
        crate::motion::hover_factor(ui.ctx(), notice_id.with("hov"), was_hovered || active);

    let (base_fill, base_border, icon, accent_fg, title_fg) = match spec.kind {
        SidebarUpdateNoticeKind::Available | SidebarUpdateNoticeKind::Downloading => {
            (t.accent_soft, t.accent, Icon::MoveDown, t.accent, t.text)
        }
        SidebarUpdateNoticeKind::ReadyToInstall => (
            t.success_bg,
            t.success_border,
            Icon::CheckCircle,
            t.success_fg,
            t.success_fg,
        ),
    };
    let fill = crate::motion::lerp_color(base_fill, t.panel_bg, hover_t * 0.22);
    let border = crate::motion::lerp_color(base_border, accent_fg, hover_t.max(0.2));

    let frame_resp = egui::Frame::NONE
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(CornerRadius::same(8))
        .inner_margin(Margin::symmetric(9, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 5.0;
                let (ir, _) = ui.allocate_exact_size(Vec2::splat(13.5), Sense::hover());
                paint_icon(ui.painter(), ir, icon, accent_fg);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(&spec.badge)
                            .size(10.5)
                            .monospace()
                            .strong()
                            .color(accent_fg),
                    );
                    ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                        ui.add(
                            egui::Label::new(
                                RichText::new(&spec.title)
                                    .size(11.5)
                                    .strong()
                                    .color(title_fg),
                            )
                            .truncate(),
                        );
                    });
                });
            });
            if spec.progress_frac.is_some() || spec.indeterminate {
                ui.add_space(1.0);
                draw_sidebar_update_progress(ui, spec.progress_frac, spec.indeterminate, t);
                ui.add_space(1.0);
            }
            ui.add(
                egui::Label::new(RichText::new(&spec.subtitle).size(10.5).color(t.text_muted))
                    .truncate(),
            );
        });

    let rect = frame_resp.response.rect;
    let resp = ui
        .interact(rect, notice_id, Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(&spec.tooltip);
    ui.ctx()
        .data_mut(|d| d.insert_temp(notice_id.with("prev_hov"), resp.hovered()));
    (resp.clicked(), rect)
}

fn draw_sidebar_update_progress(
    ui: &mut egui::Ui,
    progress_frac: Option<f32>,
    indeterminate: bool,
    t: &Tokens,
) {
    let track_h = 4.0_f32;
    let radius = 2_u8;
    let (bar_rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), track_h), Sense::hover());
    if !ui.is_rect_visible(bar_rect) {
        return;
    }
    let p = ui.painter();
    p.rect_filled(bar_rect, CornerRadius::same(radius), t.panel_bg);
    if let Some(frac) = progress_frac {
        let fill_w = bar_rect.width() * frac.clamp(0.0, 1.0);
        if fill_w > 0.5 {
            p.rect_filled(
                Rect::from_min_size(bar_rect.min, Vec2::new(fill_w, track_h)),
                CornerRadius::same(radius),
                t.accent,
            );
        }
    } else if indeterminate {
        let time_s = ui.ctx().input(|i| i.time);
        let seg_w = (bar_rect.width() * 0.30).max(18.0).min(bar_rect.width());
        let travel = bar_rect.width() + seg_w;
        let phase = ((time_s * 0.85) % 1.0) as f32;
        let left = bar_rect.left() - seg_w + travel * phase;
        let seg_rect = Rect::from_min_size(pos2(left, bar_rect.top()), Vec2::new(seg_w, track_h));
        p.with_clip_rect(bar_rect)
            .rect_filled(seg_rect, CornerRadius::same(radius), t.accent);
        ui.ctx().request_repaint();
    }
}

fn draw_sidebar_footer(ui: &mut egui::Ui, t: &Tokens) -> bool {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), SIDEBAR_FOOTER_HEIGHT),
        Sense::hover(),
    );
    let row_rect = Rect::from_min_max(pos2(rect.left(), rect.top() + 6.0), rect.right_bottom());
    let version_text = format!("v{}", env!("CARGO_PKG_VERSION"));
    let version_color = t.text_muted.gamma_multiply(0.75);
    let version_galley =
        ui.painter()
            .layout_no_wrap(version_text, FontId::proportional(10.5), version_color);
    let version_pos = pos2(
        row_rect.right() - version_galley.size().x,
        row_rect.center().y - version_galley.size().y * 0.5,
    );

    let icon_dim = 12.5_f32;
    let icon_gap = 5.0_f32;
    let label_galley = ui.painter().layout_no_wrap(
        "GitHub".to_string(),
        FontId::proportional(11.0),
        t.text_muted,
    );
    let max_link_right = (version_pos.x - 8.0).max(row_rect.left() + icon_dim);
    let link_w = (icon_dim + icon_gap + label_galley.size().x)
        .min((max_link_right - row_rect.left()).max(icon_dim));
    let link_rect = Rect::from_min_size(row_rect.left_top(), Vec2::new(link_w, row_rect.height()));
    let link_resp = ui
        .interact(
            link_rect,
            ui.make_persistent_id("papercraft_home_sidebar_github_link"),
            Sense::click(),
        )
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!(
            "Open GitHub repository\n{}",
            crate::GITHUB_REPO_URL
        ));

    if ui.is_rect_visible(rect) {
        let hover_t =
            crate::motion::hover_factor(ui.ctx(), link_resp.id.with("hov"), link_resp.hovered());
        let fg = crate::motion::lerp_color(t.text_muted.gamma_multiply(0.82), t.text, hover_t);
        let p = ui.painter();
        p.hline(
            rect.x_range(),
            rect.top() + 0.5,
            Stroke::new(1.0, t.border.gamma_multiply(0.5)),
        );
        p.galley(version_pos, version_galley, version_color);

        let icon_rect = Rect::from_center_size(
            pos2(link_rect.left() + icon_dim * 0.5, row_rect.center().y),
            Vec2::splat(icon_dim),
        );
        paint_icon(p, icon_rect, Icon::GitHub, fg);
        let text_x = icon_rect.right() + icon_gap;
        let text_y = row_rect.center().y - label_galley.size().y * 0.5;
        let label_w = label_galley.size().x;
        let label_h = label_galley.size().y;
        if text_x + label_w <= link_rect.right() + 0.5 {
            p.galley_with_override_text_color(pos2(text_x, text_y), label_galley, fg);
            if hover_t > 0.05 {
                p.hline(
                    text_x..=(text_x + label_w),
                    text_y + label_h - 1.0,
                    Stroke::new(1.0, fg.gamma_multiply(hover_t * 0.85)),
                );
            }
        }
    }

    link_resp.clicked()
}

fn draw_library_views(
    ui: &mut egui::Ui,
    snap: &NavSnapshot,
    mode: NavMode,
    t: &Tokens,
    actions: &mut Vec<LibraryAction>,
) {
    let drop = if mode == NavMode::Sidebar {
        DropKind::Accept
    } else {
        DropKind::None
    };
    let _ = section_header(ui, "LIBRARY", None, t);
    let entries = [
        (
            LibraryScope::All,
            Icon::Library,
            "All Notebooks",
            snap.all,
            "Every notebook, from every folder",
            DropKind::None,
        ),
        (
            LibraryScope::Favorites,
            Icon::Star,
            "Favorites",
            snap.favorites,
            "Starred notebooks. Drop a notebook here to star it.",
            drop,
        ),
        (
            LibraryScope::Unfiled,
            Icon::Inbox,
            "Unfiled",
            snap.unfiled,
            "Notebooks that are not in any folder. Drop a notebook here to take it out of its folder.",
            drop,
        ),
        (
            LibraryScope::Trash,
            Icon::Trash,
            "Trash",
            snap.trash,
            "Deleted notebooks, kept for 30 days. Drop a notebook here to delete it.",
            drop,
        ),
    ];
    for (scope, icon, label, count, tooltip, drop_kind) in entries {
        let spec = NavRowSpec {
            id: ui.scope_id().with(("nav_view", label)),
            icon,
            label,
            count: Some(count),
            depth: 0,
            selected: snap.scope == scope,
            expander: Expander::None,
            menu_tooltip: None,
            drop: drop_kind,
            muted: false,
            item_color: None,
        };
        let resp = nav_row(ui, &spec, t);
        if let Some(index) = resp.dropped {
            actions.push(match scope {
                LibraryScope::Favorites => LibraryAction::FavoriteNotebook(index),
                LibraryScope::Trash => LibraryAction::TrashNotebook(index),
                _ => LibraryAction::MoveNotebook {
                    index,
                    folder: String::new(),
                },
            });
        }
        if resp.row.on_hover_text(tooltip).clicked() {
            actions.push(LibraryAction::Navigate(scope));
        }
    }
}

fn draw_folder_section(
    ui: &mut egui::Ui,
    snap: &NavSnapshot,
    mode: NavMode,
    t: &Tokens,
    actions: &mut Vec<LibraryAction>,
) {
    let can_add = snap.folder_count < MAX_LIBRARY_FOLDERS;
    let add_tip = can_add.then_some("New folder");
    if section_header(ui, "FOLDERS", add_tip, t) {
        actions.push(LibraryAction::NewFolder {
            parent: String::new(),
            move_notebook: None,
        });
    }
    if snap.visible_folders.is_empty() {
        ui.add(
            egui::Label::new(
                RichText::new(
                    "Group notebooks into folders to keep classes, projects, and journals apart.",
                )
                .size(11.0)
                .color(t.text_muted),
            )
            .wrap(),
        );
        ui.add_space(4.0);
    }
    for (node, collapsed) in &snap.visible_folders {
        draw_folder_row(ui, snap, node, *collapsed, mode, t, actions);
    }
    if can_add {
        let drop = if mode == NavMode::Sidebar {
            DropKind::Accept
        } else {
            DropKind::None
        };
        let spec = NavRowSpec {
            id: ui.scope_id().with("nav_new_folder"),
            icon: Icon::FolderPlus,
            label: "New Folder",
            count: None,
            depth: 0,
            selected: false,
            expander: Expander::None,
            menu_tooltip: None,
            drop,
            muted: true,
            item_color: None,
        };
        let resp = nav_row(ui, &spec, t);
        if let Some(index) = resp.dropped {
            actions.push(LibraryAction::NewFolder {
                parent: String::new(),
                move_notebook: Some(index),
            });
        }
        if resp
            .row
            .on_hover_text("Create a folder. Drop a notebook here to put it in a new folder.")
            .clicked()
        {
            actions.push(LibraryAction::NewFolder {
                parent: String::new(),
                move_notebook: None,
            });
        }
    }
}

fn draw_folder_row(
    ui: &mut egui::Ui,
    snap: &NavSnapshot,
    node: &FolderNode,
    collapsed: bool,
    mode: NavMode,
    t: &Tokens,
    actions: &mut Vec<LibraryAction>,
) {
    let selected =
        matches!(&snap.scope, LibraryScope::Folder(open) if open.eq_ignore_ascii_case(&node.path));
    let expander = match (node.subfolders > 0, collapsed) {
        (false, _) => Expander::Leaf,
        (true, true) => Expander::Collapsed,
        (true, false) => Expander::Expanded,
    };
    let sidebar = mode == NavMode::Sidebar;
    let spec = NavRowSpec {
        id: ui.scope_id().with(("nav_folder", node.path.to_lowercase())),
        icon: if selected {
            Icon::FolderOpen
        } else {
            Icon::Folder
        },
        label: node.name(),
        count: Some(node.total_notebooks),
        depth: node.depth,
        selected,
        expander,
        menu_tooltip: sidebar.then_some("Folder actions"),
        drop: if sidebar {
            DropKind::Accept
        } else {
            DropKind::None
        },
        muted: false,
        item_color: node.color,
    };
    let resp = nav_row(ui, &spec, t);
    let can_add = snap.folder_count < MAX_LIBRARY_FOLDERS;
    if resp.chevron_clicked {
        actions.push(LibraryAction::ToggleCollapse(node.path.clone()));
    }
    if let Some(index) = resp.dropped {
        actions.push(LibraryAction::MoveNotebook {
            index,
            folder: node.path.clone(),
        });
    }
    if let Some(more) = &resp.more {
        egui::Popup::menu(more)
            .width(226.0)
            .show(|ui| folder_menu_contents(ui, node, can_add, t, actions));
    }
    if sidebar {
        egui::Popup::context_menu(&resp.row)
            .width(226.0)
            .show(|ui| folder_menu_contents(ui, node, can_add, t, actions));
    }
    let more_clicked = resp.more.as_ref().is_some_and(egui::Response::clicked);
    if !resp.chevron_clicked
        && !more_clicked
        && resp
            .row
            .on_hover_text(format!(
                "{}\n{}",
                display_folder_path(&node.path),
                node.summary()
            ))
            .clicked()
    {
        actions.push(LibraryAction::Navigate(LibraryScope::Folder(
            node.path.clone(),
        )));
    }
}

fn draw_tag_section(
    ui: &mut egui::Ui,
    snap: &NavSnapshot,
    mode: NavMode,
    t: &Tokens,
    actions: &mut Vec<LibraryAction>,
) {
    let _ = section_header(ui, "TAGS", None, t);
    for entry in &snap.tags {
        draw_tag_row(ui, snap, entry, mode, t, actions);
    }
}

fn draw_tag_row(
    ui: &mut egui::Ui,
    snap: &NavSnapshot,
    entry: &TagEntry,
    mode: NavMode,
    t: &Tokens,
    actions: &mut Vec<LibraryAction>,
) {
    let selected =
        matches!(&snap.scope, LibraryScope::Tag(open) if open.eq_ignore_ascii_case(&entry.name));
    let sidebar = mode == NavMode::Sidebar;
    let spec = NavRowSpec {
        id: ui.scope_id().with(("nav_tag", entry.name.to_lowercase())),
        icon: Icon::Tag,
        label: &entry.name,
        count: Some(entry.count),
        depth: 0,
        selected,
        expander: Expander::None,
        menu_tooltip: sidebar.then_some("Tag actions"),
        drop: DropKind::None,
        muted: false,
        item_color: entry.color,
    };
    let resp = nav_row(ui, &spec, t);
    if let Some(more) = &resp.more {
        egui::Popup::menu(more)
            .width(226.0)
            .show(|ui| tag_menu_contents(ui, &entry.name, entry.color, t, actions));
    }
    if sidebar {
        egui::Popup::context_menu(&resp.row)
            .width(226.0)
            .show(|ui| tag_menu_contents(ui, &entry.name, entry.color, t, actions));
    }
    let more_clicked = resp.more.as_ref().is_some_and(egui::Response::clicked);
    if !more_clicked
        && resp
            .row
            .on_hover_text(format!("Show notebooks tagged #{}", entry.name))
            .clicked()
    {
        actions.push(LibraryAction::Navigate(LibraryScope::Tag(
            entry.name.clone(),
        )));
    }
}

/// Narrow-window replacement for the sidebar: a button showing the open place
/// that drops down the full Library navigation.
pub(crate) fn draw_browse_button(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) {
    let scope = current_scope(app);
    let label = format!("{}  ▾", scope.title());
    let resp = compact_icon_button(ui, scope.icon(), Some(&label), true, true, t)
        .on_hover_text("Browse folders, favorites, tags, and Trash");
    egui::Popup::menu(&resp).width(250.0).show(|ui| {
        draw_library_nav(app, ui, t, NavMode::Popup);
    });
}

// ---------------------------------------------------------------------------
// Breadcrumb & subfolder tiles
// ---------------------------------------------------------------------------

/// Clickable `Library > School > Math` trail above an open folder's title.
pub(crate) fn draw_breadcrumb(
    ui: &mut egui::Ui,
    path: &str,
    t: &Tokens,
    actions: &mut Vec<LibraryAction>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if crumb(ui, "Library", "root", t).clicked() {
            actions.push(LibraryAction::Navigate(LibraryScope::All));
        }
        let mut prefix = String::new();
        let segments: Vec<&str> = path.split('/').collect();
        let last = segments.len().saturating_sub(1);
        for (i, segment) in segments.iter().enumerate() {
            let (sep_rect, _) = ui.allocate_exact_size(Vec2::splat(12.0), Sense::hover());
            paint_icon(ui.painter(), sep_rect, Icon::NextPage, t.text_muted);
            prefix = join_folder(&prefix, segment);
            if i == last {
                ui.label(RichText::new(*segment).size(11.5).color(t.text));
            } else if crumb(ui, segment, &prefix, t).clicked() {
                actions.push(LibraryAction::Navigate(LibraryScope::Folder(
                    prefix.clone(),
                )));
            }
        }
    });
}

fn crumb(ui: &mut egui::Ui, text: &str, id_key: &str, t: &Tokens) -> egui::Response {
    let hov_id = ui.make_persistent_id(("papercraft_crumb_hov", id_key));
    let prev_hovered = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(hov_id).unwrap_or(false));
    let hover_t = crate::motion::hover_factor(ui.ctx(), hov_id.with("t"), prev_hovered);
    let fg = crate::motion::lerp_color(t.text_muted, t.text, hover_t);
    let resp = ui
        .add(egui::Label::new(RichText::new(text).size(11.5).color(fg)).sense(Sense::click()))
        .on_hover_cursor(egui::CursorIcon::PointingHand);
    if resp.hovered() != prev_hovered {
        ui.ctx().data_mut(|d| d.insert_temp(hov_id, resp.hovered()));
    }
    if hover_t > 0.02 {
        let y = resp.rect.bottom() - 1.0;
        ui.painter().hline(
            resp.rect.x_range(),
            y,
            Stroke::new(1.0, t.text_muted.gamma_multiply(hover_t)),
        );
    }
    resp
}

/// Lays out an open folder's subfolders as tiles in the notebook grid columns.
pub(crate) fn draw_subfolder_tiles(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    nodes: &[FolderNode],
    cols: usize,
    tile_w: f32,
    gap: f32,
    t: &Tokens,
) -> Vec<Rect> {
    let can_add = known_library_folders(app).len() < MAX_LIBRARY_FOLDERS;
    let mut actions = Vec::new();
    let mut rects = Vec::new();
    for (row_idx, row) in nodes.chunks(cols.max(1)).enumerate() {
        if row_idx > 0 {
            ui.add_space(gap * 0.75);
        }
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for node in row {
                rects.push(draw_folder_tile(ui, node, tile_w, can_add, t, &mut actions));
            }
        });
    }
    for action in actions {
        apply_library_action(app, action);
    }
    rects
}

fn draw_folder_tile(
    ui: &mut egui::Ui,
    node: &FolderNode,
    tile_w: f32,
    can_add: bool,
    t: &Tokens,
    actions: &mut Vec<LibraryAction>,
) -> Rect {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(tile_w, TILE_HEIGHT), Sense::click());
    let id = ui
        .scope_id()
        .with(("folder_tile", node.path.to_lowercase()));
    let drag_over = resp.dnd_hover_payload::<NotebookDragPayload>().is_some();
    if let Some(payload) = resp.dnd_release_payload::<NotebookDragPayload>() {
        actions.push(LibraryAction::MoveNotebook {
            index: payload.index,
            folder: node.path.clone(),
        });
    }
    let hovered = resp.contains_pointer();
    let more_rect = Rect::from_center_size(
        pos2(rect.right() - 22.0, rect.center().y),
        Vec2::splat(26.0),
    );
    let more_id = id.with("more");
    let menu_open = egui::Popup::is_id_open(ui.ctx(), more_id.with("popup"));
    let more = (hovered || menu_open).then(|| {
        ui.interact(more_rect, more_id, Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text("Folder actions")
    });

    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let hover_t = crate::motion::hover_factor(ui.ctx(), id.with("hov"), hovered || menu_open);
        let drag_t = crate::motion::hover_factor(ui.ctx(), id.with("dnd"), drag_over);
        let active_t = hover_t.max(drag_t);
        let vis = folder_visual_colors(node.color, t);
        let fill = crate::motion::lerp_color(t.field_bg, vis.soft_fill, drag_t);
        let stroke = Stroke::new(
            crate::motion::lerp_f32(1.0, 1.5, active_t),
            crate::motion::lerp_color(t.border, vis.icon, active_t),
        );
        let title_color = crate::motion::lerp_color(t.text, vis.selected_text, active_t);
        p.rect_filled(rect, CornerRadius::same(12), fill);
        p.rect_stroke(rect, CornerRadius::same(12), stroke, StrokeKind::Inside);
        let badge = Rect::from_min_size(
            pos2(rect.left() + 12.0, rect.center().y - 19.0),
            Vec2::splat(38.0),
        );
        p.rect_filled(badge, CornerRadius::same(9), vis.soft_fill);
        paint_icon(p, badge.shrink(9.5), Icon::Folder, vis.icon);
        let text_left = badge.right() + 12.0;
        let text_w = more_rect.left() - 6.0 - text_left;
        let title = truncated_galley(ui, node.name(), 13.5, title_color, text_w);
        let sub = truncated_galley(ui, &node.summary(), 11.0, t.text_muted, text_w);
        let block_h = title.size().y + 2.0 + sub.size().y;
        let top = rect.center().y - block_h * 0.5;
        p.galley(pos2(text_left, top), title.clone(), title_color);
        p.galley(
            pos2(text_left, top + title.size().y + 2.0),
            sub,
            t.text_muted,
        );
        let more_hovered = more.as_ref().is_some_and(egui::Response::hovered) || menu_open;
        let _ = paint_more_slot(
            ui,
            MoreSlot {
                rect: more_rect,
                radius: 6,
                id: more_id,
                visible: hovered || menu_open,
                hovered: more_hovered,
            },
            t,
        );
    }

    if let Some(more_resp) = &more {
        egui::Popup::menu(more_resp)
            .width(226.0)
            .show(|ui| folder_menu_contents(ui, node, can_add, t, actions));
    }
    egui::Popup::context_menu(&resp)
        .width(226.0)
        .show(|ui| folder_menu_contents(ui, node, can_add, t, actions));
    let more_clicked = more.as_ref().is_some_and(egui::Response::clicked);
    if !more_clicked
        && resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(format!("Open {}", display_folder_path(&node.path)))
            .clicked()
    {
        actions.push(LibraryAction::Navigate(LibraryScope::Folder(
            node.path.clone(),
        )));
    }
    rect
}

// ---------------------------------------------------------------------------
// Notebook folder picker (New Notebook & Notebook Settings dialogs)
// ---------------------------------------------------------------------------

/// Mutable drafts edited by [`draw_folder_picker`].
pub(crate) struct FolderPickerDrafts<'a> {
    pub(crate) folder: &'a mut String,
    pub(crate) new_open: &'a mut bool,
    pub(crate) new_name: &'a mut String,
}

/// Applies any valid pending inline "New Folder" text if the user submits the
/// enclosing notebook dialog directly without pressing "Add" first.
pub(crate) fn commit_pending_picker_folder(
    known_folders: &[String],
    drafts: FolderPickerDrafts<'_>,
) {
    let FolderPickerDrafts {
        folder,
        new_open,
        new_name,
    } = drafts;
    if *new_open && let Ok(path) = validate_folder_name(new_name, folder, known_folders, None) {
        *folder = path;
    }
    *new_open = false;
    new_name.clear();
}

/// Folder dropdown with an inline "New Folder" field for notebook dialogs.
pub(crate) fn draw_folder_picker(
    ui: &mut egui::Ui,
    id_salt: &str,
    known_folders: &[String],
    prefs: &UserStudioConfig,
    drafts: FolderPickerDrafts<'_>,
    t: &Tokens,
) {
    let FolderPickerDrafts {
        folder,
        new_open,
        new_name,
    } = drafts;
    let mut options: Vec<String> = known_folders.to_vec();
    if !folder.is_empty() && !options.iter().any(|f| f.eq_ignore_ascii_case(folder)) {
        options.push(folder.clone());
        options.sort_by_cached_key(|path| {
            path.split('/')
                .map(str::to_lowercase)
                .collect::<Vec<String>>()
        });
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let display = if folder.is_empty() {
            "Unfiled (no folder)".to_string()
        } else {
            display_folder_path(folder)
        };
        crate::theme::studio_combo_box(
            ui,
            id_salt,
            RichText::new(display).size(12.0).color(t.text),
            260.0,
            |ui| {
                if picker_option_row(
                    ui,
                    Icon::Inbox,
                    None,
                    "Unfiled (no folder)",
                    0,
                    folder.is_empty(),
                    t,
                )
                .clicked()
                {
                    folder.clear();
                }
                for option in &options {
                    let depth = folder_depth(option).saturating_sub(1);
                    let active = folder.eq_ignore_ascii_case(option);
                    let icon = if active {
                        Icon::FolderOpen
                    } else {
                        Icon::Folder
                    };
                    let icon_color = prefs
                        .folder_color(option)
                        .map(|c| folder_visual_colors(Some(c), t).icon);
                    if picker_option_row(
                        ui,
                        icon,
                        icon_color,
                        folder_leaf(option),
                        depth,
                        active,
                        t,
                    )
                    .on_hover_text(display_folder_path(option))
                    .clicked()
                    {
                        *folder = option.clone();
                    }
                }
            },
        );
        if !*new_open
            && compact_icon_button(ui, Icon::FolderPlus, Some("New Folder"), false, true, t)
                .on_hover_text("Create a new folder for this notebook")
                .clicked()
        {
            *new_open = true;
            new_name.clear();
        }
    });
    let new_folder_id = ui.make_persistent_id((id_salt, "inline_new_folder"));
    crate::motion::animated_collapse(ui, new_folder_id, *new_open, 92.0, |ui| {
        draw_inline_new_folder(ui, &options, folder, new_open, new_name, t);
    });
}

fn picker_option_row(
    ui: &mut egui::Ui,
    icon: Icon,
    icon_color: Option<Color32>,
    label: &str,
    depth: usize,
    selected: bool,
    t: &Tokens,
) -> egui::Response {
    let width = ui.available_width().max(220.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::click());
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        if selected {
            p.rect_filled(rect, CornerRadius::same(5), t.accent_soft);
        } else if resp.hovered() {
            p.rect_filled(rect, CornerRadius::same(5), t.border.gamma_multiply(0.45));
        }
        let fg = if selected { t.accent } else { t.text };
        let icon_fg = icon_color.unwrap_or(fg);
        let x = rect.left() + 8.0 + depth as f32 * INDENT;
        let icon_rect = Rect::from_center_size(pos2(x + 7.0, rect.center().y), Vec2::splat(14.0));
        paint_icon(p, icon_rect, icon, icon_fg);
        let max_w = rect.right() - 24.0 - (x + 20.0);
        let galley = truncated_galley(ui, label, 12.0, fg, max_w);
        p.galley(
            pos2(x + 20.0, rect.center().y - galley.size().y * 0.5),
            galley,
            fg,
        );
        if selected {
            let check = Rect::from_center_size(
                pos2(rect.right() - 12.0, rect.center().y),
                Vec2::splat(13.0),
            );
            paint_icon(p, check, Icon::Check, t.accent);
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn draw_inline_new_folder(
    ui: &mut egui::Ui,
    options: &[String],
    folder: &mut String,
    new_open: &mut bool,
    new_name: &mut String,
    t: &Tokens,
) {
    let validation = validate_folder_name(new_name, folder, options, None);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        let hint = if folder.is_empty() {
            "New folder name".to_string()
        } else {
            format!("New folder inside {}", folder_leaf(folder))
        };
        let edit = ui.add_sized(
            [220.0, 28.0],
            egui::TextEdit::singleline(new_name)
                .hint_text(hint)
                .char_limit(MAX_FOLDER_NAME_CHARS)
                .margin(egui::Margin::symmetric(8, 2))
                .align(egui::Align2::LEFT_CENTER),
        );
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let add = compact_icon_button(ui, Icon::Check, Some("Add"), true, validation.is_ok(), t)
            .on_hover_text("Use this new folder for the notebook");
        if (add.clicked() || enter)
            && let Ok(path) = &validation
        {
            *folder = path.clone();
            *new_open = false;
            new_name.clear();
        }
        if compact_icon_button(ui, Icon::Close, Some("Cancel"), false, true, t).clicked() {
            *new_open = false;
            new_name.clear();
        }
    });
    if let Err(message) = &validation
        && !new_name.trim().is_empty()
    {
        ui.label(RichText::new(message).size(11.0).color(t.danger_fg));
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn folder_path_helpers_handle_nesting_case_and_hostile_input() {
        assert_eq!(folder_parent("School/Math/Calc"), "School/Math");
        assert_eq!(folder_parent("School"), "");
        assert_eq!(folder_leaf("School/Math"), "Math");
        assert_eq!(folder_depth(""), 0);
        assert_eq!(folder_depth("A/B/C"), 3);
        assert_eq!(join_folder("", "Work"), "Work");
        assert_eq!(join_folder("Work", "Q1"), "Work/Q1");
        assert!(path_within("school/math", "School"));
        assert!(path_within("School", "school"));
        assert!(!path_within("Schoolwork", "School"));
        assert!(!path_within("", "School"));
        assert!(!path_within("School", ""));
        // Multi-byte names never panic on slicing.
        assert!(!path_within("Ünï/x", "Ün"));
        assert!(path_within("Ünï/x", "Ünï"));
    }

    #[test]
    fn folder_name_validation_explains_every_rejection() {
        let existing = vec!["School".to_string(), "School/Math".to_string()];
        assert_eq!(
            validate_folder_name(" Physics ", "School", &existing, None),
            Ok("School/Physics".to_string())
        );
        assert!(validate_folder_name("   ", "", &existing, None).is_err());
        assert!(validate_folder_name("a/b", "", &existing, None).is_err());
        assert!(validate_folder_name("..", "", &existing, None).is_err());
        assert!(validate_folder_name(&"x".repeat(49), "", &existing, None).is_err());
        let dup = validate_folder_name("math", "School", &existing, None);
        assert!(dup.is_err_and(|msg| msg.contains("already exists")));
        // Renaming a folder to a different case of its own name is allowed.
        assert_eq!(
            validate_folder_name("MATH", "School", &existing, Some("School/Math")),
            Ok("School/MATH".to_string())
        );
        let deep = (0..MAX_FOLDER_DEPTH)
            .map(|i| format!("L{i}"))
            .collect::<Vec<_>>()
            .join("/");
        assert!(validate_folder_name("Too deep", &deep, &existing, None).is_err());
    }

    #[test]
    fn scope_round_trips_and_opening_a_folder_reveals_it_in_the_tree() {
        let mut app = PaperCraftApp::default();
        for scope in [
            LibraryScope::All,
            LibraryScope::Favorites,
            LibraryScope::Unfiled,
            LibraryScope::Trash,
            LibraryScope::Folder("School/Math".to_string()),
            LibraryScope::Tag("exam".to_string()),
        ] {
            set_scope(&mut app, scope.clone());
            assert_eq!(current_scope(&app), scope);
        }
        app.view.library_collapsed_folders = vec!["school".to_string(), "Work".to_string()];
        set_scope(&mut app, LibraryScope::Folder("School/Math".to_string()));
        assert_eq!(app.view.library_collapsed_folders, vec!["Work".to_string()]);
    }

    #[test]
    fn folder_tree_orders_depth_first_and_counts_nested_notebooks() {
        let mut app = PaperCraftApp::default();
        for (title, folder) in [
            ("Calc", "School/Math"),
            ("Bio", "School"),
            ("Plan", "School Work"),
        ] {
            app.session
                .execute(
                    "notebook.create",
                    &json!({ "title": title, "folder_path": folder }),
                )
                .unwrap();
        }
        let tree = folder_tree(&app);
        let paths: Vec<&str> = tree.iter().map(|n| n.path.as_str()).collect();
        assert_eq!(paths, vec!["School", "School/Math", "School Work"]);
        assert_eq!(tree[0].total_notebooks, 2);
        assert_eq!(tree[0].direct_notebooks, 1);
        assert_eq!(tree[0].subfolders, 1);
        assert_eq!(tree[1].depth, 1);
        assert_eq!(subfolders_of(&tree, "School").len(), 1);
        assert_eq!(subfolders_of(&tree, "").len(), 2);
    }

    #[test]
    fn library_actions_move_favorite_and_collapse_through_commands() {
        let mut app = PaperCraftApp::default();
        app.session
            .execute("library.folder.create", &json!({ "path": "Work" }))
            .unwrap();
        apply_library_action(
            &mut app,
            LibraryAction::MoveNotebook {
                index: 0,
                folder: "Work".to_string(),
            },
        );
        assert_eq!(app.session.notebooks[0].folder_path, "Work");
        apply_library_action(
            &mut app,
            LibraryAction::MoveNotebook {
                index: 0,
                folder: String::new(),
            },
        );
        assert_eq!(app.session.notebooks[0].folder_path, "");
        apply_library_action(&mut app, LibraryAction::FavoriteNotebook(0));
        assert!(app.session.notebooks[0].favorite);
        // Out-of-range indices are ignored.
        apply_library_action(
            &mut app,
            LibraryAction::MoveNotebook {
                index: 99,
                folder: "Work".to_string(),
            },
        );
        apply_library_action(&mut app, LibraryAction::ToggleCollapse("Work".to_string()));
        assert!(is_collapsed(&app, "work"));
        apply_library_action(&mut app, LibraryAction::ToggleCollapse("WORK".to_string()));
        assert!(!is_collapsed(&app, "Work"));
        apply_library_action(&mut app, LibraryAction::TrashNotebook(0));
        assert_eq!(
            app.view.active_modal,
            Some(ActiveModal::ConfirmDeleteNotebook)
        );
        assert!(app.view.pending_delete_notebook.is_some());
    }

    #[test]
    fn tag_actions_and_right_click_popup_change_color_and_open_modal() {
        let mut app = PaperCraftApp::default();
        app.session
            .execute(
                "notebook.create",
                &json!({ "title": "Study Notes", "tags": ["example", "test"] }),
            )
            .unwrap();
        let teal = Color::from_hex("#0D9488").unwrap();
        apply_library_action(
            &mut app,
            LibraryAction::SetTagColor {
                tag: "example".to_string(),
                color: Some(teal),
            },
        );
        assert_eq!(app.session.prefs.tag_color("example"), Some(teal));

        apply_library_action(
            &mut app,
            LibraryAction::SetTagColor {
                tag: "example".to_string(),
                color: None,
            },
        );
        assert_eq!(app.session.prefs.tag_color("example"), None);

        // Right-clicking a tag row in the sidebar opens the tag popup with Quick Color and Tag Color...
        let ctx = egui::Context::default();
        let viewport = Vec2::new(320.0, 600.0);
        let t = Tokens::light();
        let mut tag_pos = None;
        for _ in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, viewport)),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                draw_library_nav(&mut app, ui, &t, NavMode::Sidebar);
            });
            for clipped in &out.shapes {
                if let egui::Shape::Text(text) = &clipped.shape
                    && text.galley.text() == "example"
                {
                    tag_pos = Some(text.visual_bounding_rect().center());
                }
            }
            out.drop_without_applying_deltas();
        }
        let pos = tag_pos.expect("expected 'example' tag row in sidebar");

        // Secondary click (right-click) opens the context menu popup.
        let mut popup_texts = Vec::new();
        let mut tag_color_item_pos = None;
        for (pressed, time) in [(true, 0.05), (false, 0.10), (false, 0.15)] {
            let mut events = vec![egui::Event::PointerMoved(pos)];
            if time < 0.12 {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, viewport)),
                time: Some(time),
                events,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                draw_library_nav(&mut app, ui, &t, NavMode::Sidebar);
            });
            popup_texts.clear();
            fn visit_text(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
                match shape {
                    egui::Shape::Text(txt) => {
                        out.push((txt.galley.text().to_string(), txt.visual_bounding_rect()));
                    }
                    egui::Shape::Vec(items) => {
                        for item in items {
                            visit_text(item, out);
                        }
                    }
                    _ => {}
                }
            }
            for clipped in &out.shapes {
                visit_text(&clipped.shape, &mut popup_texts);
            }
            if let Some((_, r)) = popup_texts.iter().find(|(txt, _)| txt == "Tag Color...") {
                tag_color_item_pos = Some(r.center());
            }
            out.drop_without_applying_deltas();
        }
        assert!(popup_texts.iter().any(|(txt, _)| txt == "Open"));
        assert!(popup_texts.iter().any(|(txt, _)| txt == "Tag Color..."));
        assert!(popup_texts.iter().any(|(txt, _)| txt == "Quick Color"));

        // Clicking "Tag Color..." opens ActiveModal::TagColor for "example" and closes the popup.
        let click_pos = tag_color_item_pos.expect("Tag Color... item rect");
        for (pressed, time) in [(true, 0.20), (false, 0.25)] {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, viewport)),
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
                draw_library_nav(&mut app, ui, &t, NavMode::Sidebar);
            });
            out.drop_without_applying_deltas();
        }
        assert_eq!(app.view.active_modal, Some(ActiveModal::TagColor));
        assert_eq!(app.view.tag_dialog_target, "example");
        assert!(!egui::Popup::is_any_open(&ctx));
    }

    #[test]
    fn home_sidebar_footer_shows_github_link_and_version_only_in_sidebar_mode() {
        let opened = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let opened_hook = std::sync::Arc::clone(&opened);
        let hooks = crate::PlatformHooks {
            open_url: Some(Box::new(move |url| {
                opened_hook.lock().unwrap().push(url.to_string());
            })),
            ..crate::PlatformHooks::default()
        };
        let mut app = PaperCraftApp::new(papercraft_engine::Session::default(), hooks);
        let ctx = egui::Context::default();
        let viewport = Vec2::new(1280.0, 800.0);
        let expected_version = format!("v{}", env!("CARGO_PKG_VERSION"));
        assert_eq!(expected_version, "v0.2.3");

        let mut sidebar_rect = Rect::NOTHING;
        let mut github_rect = None;
        let mut version_rect = None;
        for _ in 0..2 {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, viewport)),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                sidebar_rect = crate::home::show_inner(&mut app, ui)
                    .sidebar_rect
                    .expect("Home sidebar visible at 1280px");
            });
            fn find_texts(
                shape: &egui::Shape,
                github_rect: &mut Option<Rect>,
                version_rect: &mut Option<Rect>,
                expected_version: &str,
            ) {
                match shape {
                    egui::Shape::Text(text) => {
                        let s = text.galley.text();
                        if s == "GitHub" {
                            *github_rect = Some(text.visual_bounding_rect());
                        } else if s == expected_version {
                            *version_rect = Some(text.visual_bounding_rect());
                        }
                    }
                    egui::Shape::Vec(items) => {
                        for item in items {
                            find_texts(item, github_rect, version_rect, expected_version);
                        }
                    }
                    _ => {}
                }
            }
            github_rect = None;
            version_rect = None;
            for clipped in &out.shapes {
                find_texts(
                    &clipped.shape,
                    &mut github_rect,
                    &mut version_rect,
                    &expected_version,
                );
            }
            out.drop_without_applying_deltas();
        }

        let gh_r = github_rect.expect("expected GitHub link in Home sidebar footer");
        let ver_r = version_rect.expect("expected version label in Home sidebar footer");
        assert!(sidebar_rect.contains(gh_r.center()));
        assert!(sidebar_rect.contains(ver_r.center()));
        assert!(gh_r.right() < ver_r.left());
        assert!((gh_r.center().y - ver_r.center().y).abs() <= 2.0);
        assert!(sidebar_rect.bottom() - gh_r.bottom() <= 24.0);

        let click_pos = gh_r.center();
        for (pressed, time) in [(true, 0.05), (false, 0.10)] {
            let raw = egui::RawInput {
                screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, viewport)),
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
                crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                let _ = crate::home::show_inner(&mut app, ui);
            });
            out.drop_without_applying_deltas();
        }
        assert_eq!(
            *opened.lock().unwrap(),
            vec![crate::GITHUB_REPO_URL.to_string()]
        );

        // Popup mode (narrow Browse menu) and Editor screen must not show the Home sidebar footer.
        let t = Tokens::light();
        let popup_out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(320.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                draw_library_nav(&mut app, ui, &t, NavMode::Popup);
            },
        );
        let mut popup_gh = None;
        let mut popup_ver = None;
        fn find_popup(
            shape: &egui::Shape,
            gh: &mut Option<Rect>,
            ver: &mut Option<Rect>,
            expected_version: &str,
        ) {
            match shape {
                egui::Shape::Text(text) => {
                    let s = text.galley.text();
                    if s == "GitHub" {
                        *gh = Some(text.visual_bounding_rect());
                    } else if s == expected_version {
                        *ver = Some(text.visual_bounding_rect());
                    }
                }
                egui::Shape::Vec(items) => {
                    for item in items {
                        find_popup(item, gh, ver, expected_version);
                    }
                }
                _ => {}
            }
        }
        for clipped in &popup_out.shapes {
            find_popup(
                &clipped.shape,
                &mut popup_gh,
                &mut popup_ver,
                &expected_version,
            );
        }
        popup_out.drop_without_applying_deltas();
        assert!(popup_gh.is_none(), "Browse popup must omit GitHub footer");
        assert!(popup_ver.is_none(), "Browse popup must omit version footer");

        crate::open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = crate::motion::ScreenTransition::new(crate::AppScreen::Editor);
        let mut editor_gh = None;
        let mut editor_ver = None;
        for _ in 0..2 {
            let editor_out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, viewport)),
                    ..Default::default()
                },
                |ui| {
                    app.show_ui(ui);
                },
            );
            editor_gh = None;
            editor_ver = None;
            for clipped in &editor_out.shapes {
                find_popup(
                    &clipped.shape,
                    &mut editor_gh,
                    &mut editor_ver,
                    &expected_version,
                );
            }
            editor_out.drop_without_applying_deltas();
        }
        assert!(editor_gh.is_none(), "Editor must omit Home GitHub footer");
        assert!(editor_ver.is_none(), "Editor must omit Home version footer");
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn home_sidebar_update_notice_renders_above_footer_and_opens_modal() {
        use papercraft_updater::{
            AppId, AssetFormat, AvailableUpdate, InstallCapability, PlatformAsset, SemVer,
            StagedUpdate, UpdateStatus,
        };

        let mut app = PaperCraftApp::default();
        let ctx = egui::Context::default();
        let t = Tokens::light();
        let update = AvailableUpdate {
            app: AppId::PaperCraft,
            current_version: SemVer::parse("0.2.0").unwrap(),
            version: SemVer::parse("0.3.0").unwrap(),
            tag: "v0.3.0".to_string(),
            release_url: "https://github.com/alyxhard/papercraft/releases/tag/v0.3.0".to_string(),
            published_at: "2026-11-24T16:40:00Z".to_string(),
            notes: "Bug fixes".to_string(),
            platform: "windows-x86_64".to_string(),
            asset: PlatformAsset {
                file_name: "papercraft-windows-x86_64.exe".to_string(),
                url: "https://github.com/alyxhard/papercraft/releases/download/v0.3.0/papercraft-windows-x86_64.exe".to_string(),
                sha256: "0".repeat(64),
                size_bytes: 20_000_000,
                format: AssetFormat::Exe,
            },
            capability: InstallCapability::Supported {
                target_path: std::path::PathBuf::from("papercraft.exe"),
                staging_dir: std::env::temp_dir(),
            },
            is_custom_feed: false,
        };

        let find_rect = |shapes: &[egui::epaint::ClippedShape], needle: &str| -> Option<Rect> {
            fn visit(shape: &egui::Shape, needle: &str) -> Option<Rect> {
                match shape {
                    egui::Shape::Text(text) if text.galley.text() == needle => {
                        Some(text.visual_bounding_rect())
                    }
                    egui::Shape::Vec(items) => items.iter().find_map(|s| visit(s, needle)),
                    _ => None,
                }
            }
            shapes.iter().find_map(|c| visit(&c.shape, needle))
        };

        // 1. Available notice renders in Sidebar below New Folder and above GitHub footer, and opens modal on click.
        let avail_spec =
            sidebar_update_notice_spec(&UpdateStatus::Available(Box::new(update.clone())))
                .expect("Available status should build sidebar notice");
        let mut shapes = Vec::new();
        for _ in 0..2 {
            let mut out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(SIDEBAR_WIDTH, 700.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                    draw_library_nav_with_notice(
                        &mut app,
                        ui,
                        &t,
                        NavMode::Sidebar,
                        Some(&avail_spec),
                    );
                },
            );
            shapes = std::mem::take(&mut out.shapes);
            out.drop_without_applying_deltas();
        }
        let folders_r = find_rect(&shapes, "New Folder").expect("New Folder visible");
        let notice_title_r = find_rect(&shapes, "Update available").expect("Update notice title");
        let notice_badge_r = find_rect(&shapes, "v0.3.0").expect("Update notice badge");
        let gh_r = find_rect(&shapes, "GitHub").expect("GitHub footer visible");
        assert!(folders_r.bottom() < notice_title_r.top());
        assert!(notice_title_r.right() < notice_badge_r.left());
        assert!(notice_title_r.bottom() < gh_r.top());

        let click_pos = notice_title_r.center();
        for (pressed, time) in [(true, 0.05), (false, 0.10)] {
            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(
                        egui::Pos2::ZERO,
                        Vec2::new(SIDEBAR_WIDTH, 700.0),
                    )),
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
                },
                |ui| {
                    crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                    draw_library_nav_with_notice(
                        &mut app,
                        ui,
                        &t,
                        NavMode::Sidebar,
                        Some(&avail_spec),
                    );
                },
            );
            out.drop_without_applying_deltas();
        }
        assert_eq!(app.view.active_modal, Some(ActiveModal::AppUpdate));

        // 2. Downloading and ReadyToInstall specs render cleanly, and Popup mode omits the notice.
        let dl_spec = sidebar_update_notice_spec(&UpdateStatus::Downloading {
            update: Box::new(update.clone()),
            downloaded_bytes: 10_000_000,
            total_bytes: 20_000_000,
        })
        .expect("Downloading status should build sidebar notice");
        let dl_out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(SIDEBAR_WIDTH, 700.0),
                )),
                ..Default::default()
            },
            |ui| {
                draw_library_nav_with_notice(&mut app, ui, &t, NavMode::Sidebar, Some(&dl_spec));
            },
        );
        assert!(find_rect(&dl_out.shapes, "Downloading update...").is_some());
        assert!(find_rect(&dl_out.shapes, "50%").is_some());
        dl_out.drop_without_applying_deltas();

        let ready_spec = sidebar_update_notice_spec(&UpdateStatus::ReadyToInstall {
            update: Box::new(update),
            staged: StagedUpdate {
                app: AppId::PaperCraft,
                version: SemVer::parse("0.3.0").unwrap(),
                staged_path: std::path::PathBuf::from("staged.exe"),
                target_path: std::path::PathBuf::from("papercraft.exe"),
                format: AssetFormat::Exe,
                sha256: "0".repeat(64),
            },
        })
        .expect("ReadyToInstall status should build sidebar notice");
        let ready_out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(SIDEBAR_WIDTH, 700.0),
                )),
                ..Default::default()
            },
            |ui| {
                draw_library_nav_with_notice(&mut app, ui, &t, NavMode::Sidebar, Some(&ready_spec));
            },
        );
        assert!(find_rect(&ready_out.shapes, "Update ready").is_some());
        ready_out.drop_without_applying_deltas();

        let popup_out = ctx.run_ui(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    Vec2::new(320.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                draw_library_nav_with_notice(&mut app, ui, &t, NavMode::Popup, Some(&avail_spec));
            },
        );
        assert!(find_rect(&popup_out.shapes, "Update available").is_none());
        popup_out.drop_without_applying_deltas();
    }
}
