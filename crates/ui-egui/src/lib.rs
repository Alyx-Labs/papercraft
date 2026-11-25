//! L6 thin egui/eframe application shell for PaperCraft.

#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod account_modal;
pub mod canvas;
pub mod color_picker;
pub mod customization_modals;
pub mod export_modal;
pub mod folder_modals;
pub(crate) mod gpu_ink;
pub mod home;
pub mod icons;
pub mod inspector;
pub mod library_folders;
pub mod modal;
pub mod motion;
pub mod page_format_picker;
pub mod pages_sidebar;
pub mod pdf_view;
pub mod profile_screen;
pub mod shortcuts_modal;
pub mod stylus;
pub mod sync_worker;
pub mod tab_strip;
pub mod theme;
pub mod toolbar;
#[cfg(not(target_arch = "wasm32"))]
pub mod update_modal;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
#[cfg(not(target_arch = "wasm32"))]
use std::sync::mpsc;

use papercraft_doc::{
    Document, InkStroke, MAX_TEXT_CHARS, PenKind, ShapeKind, StrokeId, TextItem,
    base64_encode_bytes, sanitize_link_url,
};
use papercraft_engine::Session;
use papercraft_paint::RibbonMesh;
use serde_json::json;

/// Public source repository URL displayed in Home and Profile links.
// TODO(org): Update GITHUB_REPO_URL (and profile_screen.rs display text) once transferred to the Alyx Labs GitHub organization slug.
pub(crate) const GITHUB_REPO_URL: &str = "https://github.com/alyxhard/papercraft";

/// Top-level workspace screen mode.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AppScreen {
    #[default]
    Home,
    Editor,
    Profile,
}

/// Active section tab inside the right-hand Settings studio drawer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SettingsTab {
    #[default]
    Pen,
    Appearance,
    System,
}

/// Pending drag-and-drop page reorder awaiting modal confirmation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PendingPageReorder {
    pub notebook_index: usize,
    pub from: usize,
    pub to: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RulerDragMode {
    Move,
    Rotate,
}

/// Object highlighted by the non-editing Select arrow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SelectedObject {
    Stroke(StrokeId),
    Shape(usize),
    Text(usize),
    Image(usize),
}

/// Target channel when adjusting geometric shape colors in the floating color dock.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ShapeColorTarget {
    #[default]
    Border,
    Fill,
}

#[derive(Clone, Debug)]
pub enum SelectionDragMode {
    Marquee {
        start: [f32; 2],
        current: [f32; 2],
        additive: bool,
    },
    Lasso {
        points: Vec<[f32; 2]>,
        additive: bool,
        dragged: bool,
    },
    Move {
        last: [f32; 2],
        record_history: bool,
    },
    Resize {
        pivot: [f32; 2],
        start_last: [f32; 2],
        last_pointer: [f32; 2],
        applied_scale: [f64; 2],
        last_constrained: bool,
        sign: [f32; 2],
        resize_x: bool,
        resize_y: bool,
        proportional: bool,
        cursor: egui::CursorIcon,
        record_history: bool,
    },
    ResizeTextWidth {
        index: usize,
        from_left: bool,
        start_pointer_page: [f64; 2],
        start_origin: [f64; 2],
        start_width: f64,
        angle_degrees: f64,
        record_history: bool,
    },
    Rotate {
        pivot: [f32; 2],
        last_angle: f32,
        record_history: bool,
    },
}

#[derive(Clone, Debug)]
pub struct SelectionDragState {
    pub mode: SelectionDragMode,
    pub touch_id: Option<egui::TouchId>,
}

/// Persistent UI viewport & panel state.
/// Placement mode requested when opening a PDF file from the UI or drag-and-drop.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum PdfImportTarget {
    #[default]
    NewNotebook,
    InsertIntoActive,
}

/// Destination context when importing an image or PDF file as a reusable custom paper template.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PendingTemplateImport {
    Image {
        apply_to_page: bool,
        apply_to_new_notebook: bool,
    },
    Pdf {
        apply_to_page: bool,
        apply_to_new_notebook: bool,
    },
}

#[derive(Clone, Copy, Debug)]
pub struct LaserPoint {
    pub x: f64,
    pub y: f64,
    pub time_s: f64,
    pub stroke_break: bool,
}

pub(crate) fn effective_doc_ink_context(app: &PaperCraftApp) -> (bool, papercraft_doc::PaperColor) {
    if app.session.doc.pdf_backdrop.is_some() {
        (false, papercraft_doc::PaperColor::PureWhite)
    } else {
        (
            app.session.prefs.auto_contrast_with_theme,
            app.session.doc.paper_color,
        )
    }
}

pub(crate) fn effective_page_ink_context(
    page: &papercraft_doc::NotebookPage,
    prefs: &papercraft_doc::UserStudioConfig,
) -> (bool, papercraft_doc::PaperColor) {
    if page.pdf_backdrop.is_some() {
        (false, papercraft_doc::PaperColor::PureWhite)
    } else {
        (prefs.auto_contrast_with_theme, page.paper_color)
    }
}

fn is_pdf_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("pdf"))
}

fn is_package_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("pcraftpkg") || e.eq_ignore_ascii_case("ncraftpkg"))
}

fn is_picked_pdf(picked: &PickedOpenFile) -> bool {
    match picked {
        PickedOpenFile::Path(path) => is_pdf_path(path),
        PickedOpenFile::Bytes { name, bytes } => {
            papercraft_doc::looks_like_pdf_bytes(bytes) || is_pdf_path(std::path::Path::new(name))
        }
    }
}

fn is_picked_package(picked: &PickedOpenFile) -> bool {
    match picked {
        PickedOpenFile::Path(path) => is_package_path(path),
        PickedOpenFile::Bytes { name, bytes } => {
            papercraft_engine::looks_like_package_bytes(bytes)
                || is_package_path(std::path::Path::new(name))
        }
    }
}

#[derive(Clone, Debug)]
pub struct ViewState {
    pub screen: AppScreen,
    pub profile_return_screen: AppScreen,
    pub open_tabs: Vec<tab_strip::OpenTab>,
    pub library_search: String,
    pub library_show_trash: bool,
    pub last_autosave_s: f64,
    pub last_trash_cleanup_s: f64,
    pub zoom: f32,
    pub pan: [f64; 2],
    pub fit_requested: bool,
    pub pages_sidebar_open: bool,
    pub inspector_open: bool,
    pub active_modal: Option<modal::ActiveModal>,
    pub text_insert_mode: bool,
    pub text_editing: bool,
    pub text_editor_needs_focus: bool,
    pub text_insert_position: [f64; 2],
    pub text_draft: String,
    pub text_edit_index: Option<usize>,
    pub text_size_draft: f32,
    pub text_color_draft: papercraft_color::Color,
    pub text_font_draft: papercraft_doc::TextFont,
    pub text_line_spacing_draft: f32,
    pub text_letter_spacing_draft: f32,
    pub text_max_width_draft: Option<f32>,
    pub text_link_draft: Option<String>,
    pub text_link_input: String,
    pub text_link_editor_open: bool,
    pub copied_text_item: Option<TextItem>,
    pub last_canvas_page_pos: Option<[f64; 2]>,
    pub text_insert_error: Option<String>,
    pub image_insert_error: Option<String>,
    pub pdf_import_error: Option<String>,
    pub pdf_import_in_flight: Option<String>,
    pub pdf_import_target: PdfImportTarget,
    pub selected_text: Option<usize>,
    pub selected_object: Option<SelectedObject>,
    pub selected_objects: Vec<SelectedObject>,
    pub selection_drag: Option<SelectionDragState>,
    pub text_style_open: bool,
    pub paste_requested: bool,
    pub shape_insert_mode: bool,
    pub selected_shape_kind: ShapeKind,
    pub shape_angle_degrees: f64,
    pub shape_curve: f64,
    pub shape_drag_start: Option<[f64; 2]>,
    pub shape_drag_end: Option<[f64; 2]>,
    pub shape_drag_touch_id: Option<egui::TouchId>,
    pub ruler_active: bool,
    pub ruler_initialized: bool,
    pub ruler_center: [f64; 2],
    pub ruler_angle: f64,
    pub ruler_length: f64,
    pub ruler_depth: f64,
    pub ruler_drag_mode: Option<RulerDragMode>,
    pub ruler_drag_touch_id: Option<egui::TouchId>,
    pub ruler_drag_anchor: Option<[f64; 2]>,
    pub ruler_drag_origin_center: [f64; 2],
    pub ruler_drag_origin_angle: f64,
    pub ruler_stroke_edge: Option<f64>,
    pub pending_delete_notebook: Option<Arc<papercraft_doc::Document>>,
    pub library_folder_filter: Option<String>,
    pub library_favorites_only: bool,
    pub library_tag_filter: Option<String>,
    /// Lower-cased folder paths whose subfolders are hidden in the Library sidebar tree.
    pub library_collapsed_folders: Vec<String>,
    /// Existing folder path targeted by the Rename / Delete Folder dialogs.
    pub folder_dialog_target: String,
    /// Single-segment folder name typed into the New / Rename Folder dialogs.
    pub folder_dialog_name: String,
    /// Parent location chosen in the New / Rename Folder dialogs (`""` = top level).
    pub folder_dialog_parent: String,
    /// Notebook to move into the folder once the New Folder dialog creates it.
    pub folder_dialog_move_notebook: Option<usize>,
    pub folder_dialog_focus_pending: bool,
    /// Optional custom color selected in the New / Rename / Color Folder dialogs.
    pub folder_dialog_color: Option<papercraft_color::Color>,
    pub folder_color_draft: folder_modals::FolderColorDraftState,
    /// Existing tag targeted by the Tag Color dialog.
    pub tag_dialog_target: String,
    /// Optional custom color selected in the Tag Color dialog.
    pub tag_dialog_color: Option<papercraft_color::Color>,
    pub tag_color_draft: folder_modals::FolderColorDraftState,
    /// Inline "New folder" field state of the notebook folder picker.
    pub folder_picker_new_open: bool,
    pub folder_picker_new_name: String,
    pub notebook_folder_draft: String,
    pub notebook_favorite_draft: bool,
    pub notebook_tags_draft: Vec<String>,
    pub notebook_tag_input_draft: String,
    pub new_notebook_folder: String,
    pub new_notebook_favorite: bool,
    pub new_notebook_tags: Vec<String>,
    pub new_notebook_tag_input: String,
    pub new_notebook_custom_template_id: Option<String>,
    pub tag_edit_index: Option<usize>,
    pub tag_edit_draft: String,
    pub tag_edit_focus_pending: bool,
    pub pages_bookmarked_only: bool,
    pub pages_select_mode: bool,
    pub pages_footer_expanded: bool,
    pub selected_page_indices: Vec<usize>,
    pub pages_drag_source: Option<usize>,
    pub pending_page_reorder: Option<PendingPageReorder>,
    pub transfer_pages_list: Vec<usize>,
    pub transfer_move_mode: bool,
    pub transfer_create_new_notebook: bool,
    pub transfer_target_notebook: usize,
    pub transfer_new_title: String,
    pub transfer_merge_whole_notebook: Option<usize>,
    pub transfer_merge_delete_source: bool,
    pub pending_template_import: Option<PendingTemplateImport>,
    pub template_import_name: String,
    pub template_import_pdf_page: usize,
    pub template_import_error: Option<String>,
    pub settings_tab: SettingsTab,
    pub pan_dragging: bool,
    pub finger_pan_touch_id: Option<egui::TouchId>,
    pub finger_pan_start: Option<egui::Pos2>,
    pub finger_pan_last: Option<egui::Pos2>,
    pub finger_pan_engaged: bool,
    pub active_touch_id: Option<egui::TouchId>,
    pub active_pointer_stroke: bool,
    pub straighten_reference_point: Option<papercraft_geom::Point>,
    pub straighten_candidate_since_s: Option<f64>,
    pub export_path_stem: String,
    pub export_format: FileDialogKind,
    pub export_target_pages: Option<Vec<usize>>,
    pub export_separate_files: bool,
    pub export_feedback: Option<(bool, String)>,
    pub notebook_title_draft: String,
    pub notebook_cover_draft: papercraft_doc::NotebookCover,
    pub new_notebook_step: usize,
    pub new_notebook_preset: papercraft_doc::NotebookPreset,
    pub new_notebook_title: String,
    pub new_notebook_cover: papercraft_doc::NotebookCover,
    pub new_notebook_template: papercraft_doc::PaperTemplate,
    pub new_notebook_color: papercraft_doc::PaperColor,
    pub new_notebook_format: page_format_picker::PageFormatDraft,
    pub notebook_settings_step: usize,
    pub page_setup_format: page_format_picker::PageFormatDraft,
    pub page_setup_template: papercraft_doc::PaperTemplate,
    pub page_setup_color: papercraft_doc::PaperColor,
    pub page_setup_custom_template_id: Option<String>,
    pub page_setup_clear_custom: bool,
    pub page_setup_quarter_turns: i32,
    pub page_setup_apply_all: bool,
    pub cover_color_draft: customization_modals::CoverColorDraftState,
    pub cached_page_key: (usize, usize),
    pub cached_color_key: Option<(papercraft_doc::UiTheme, bool, papercraft_doc::PaperColor)>,
    pub motion: motion::MotionState,
    pub(crate) context_tray: toolbar::ContextTrayTransition,
    pub(crate) color_dock: toolbar::ColorDockState,
    pub color_picker: color_picker::ColorPickerState,
    pub account: account_modal::AccountDraftState,
    pub last_ctx_zoom: Option<f32>,
    pub freeform_lasso_mode: bool,
    pub shape_color_target: ShapeColorTarget,
    pub shape_border_enabled: bool,
    pub shape_border_color: Option<papercraft_color::Color>,
    pub shape_fill_enabled: bool,
    pub shape_fill_color: Option<papercraft_color::Color>,
    pub shape_line_style: papercraft_doc::ShapeLineStyle,
    pub laser_trail_mode: bool,
    pub laser_color: papercraft_color::Color,
    pub laser_points: Vec<LaserPoint>,
    pub elements_popover_open: bool,
    pub smoothing_popover_open: bool,
    pub smoothing_popover_anchor: Option<egui::Rect>,
    pub stickers_tab: StickersTab,
    pub modal_open_seq: u64,
    pub element_name_draft: String,
    pub text_alignment_draft: papercraft_doc::TextAlignment,
    pub text_spans_draft: Vec<papercraft_doc::TextSpan>,
    pub text_prev_draft: String,
    pub text_selection_bytes: Option<[usize; 2]>,
    pub text_selection_needs_restore: bool,
    pub text_editor_page_size: Option<[f32; 2]>,
    pub last_copied_plain_text: Option<String>,
    pub rich_fonts_installed: bool,
    pub last_canvas_rect: Option<egui::Rect>,
    pub last_home_rect: Option<egui::Rect>,
    pub first_access_nickname: String,
    pub first_access_error: Option<String>,
    pub first_access_focus_pending: bool,
    pub desktop_shortcut_feedback: Option<(bool, String)>,
}

impl Default for ViewState {
    fn default() -> Self {
        let default_preset = papercraft_doc::NotebookPreset::EverydayLined;
        Self {
            screen: AppScreen::Home,
            profile_return_screen: AppScreen::Home,
            open_tabs: Vec::new(),
            library_search: String::new(),
            library_show_trash: false,
            last_autosave_s: 0.0,
            last_trash_cleanup_s: 0.0,
            zoom: 1.0,
            pan: [40.0, 24.0],
            fit_requested: true,
            pages_sidebar_open: true,
            inspector_open: false,
            active_modal: None,
            text_insert_mode: false,
            text_editing: false,
            text_editor_needs_focus: false,
            text_insert_position: [0.0, 0.0],
            text_draft: String::new(),
            text_edit_index: None,
            text_size_draft: 24.0,
            text_color_draft: papercraft_color::INK_PALETTE[1].color,
            text_font_draft: papercraft_doc::TextFont::Sans,
            text_line_spacing_draft: papercraft_doc::DEFAULT_TEXT_LINE_SPACING,
            text_letter_spacing_draft: papercraft_doc::DEFAULT_TEXT_LETTER_SPACING,
            text_max_width_draft: None,
            text_link_draft: None,
            text_link_input: String::new(),
            text_link_editor_open: false,
            copied_text_item: None,
            last_canvas_page_pos: None,
            text_insert_error: None,
            image_insert_error: None,
            pdf_import_error: None,
            pdf_import_in_flight: None,
            pdf_import_target: PdfImportTarget::NewNotebook,
            selected_text: None,
            selected_object: None,
            selected_objects: Vec::new(),
            selection_drag: None,
            text_style_open: false,
            paste_requested: false,
            shape_insert_mode: false,
            selected_shape_kind: ShapeKind::Rectangle,
            shape_angle_degrees: 0.0,
            shape_curve: 0.0,
            shape_drag_start: None,
            shape_drag_end: None,
            shape_drag_touch_id: None,
            ruler_active: false,
            ruler_initialized: false,
            ruler_center: [0.0, 0.0],
            ruler_angle: 0.0,
            ruler_length: 0.0,
            ruler_depth: 0.0,
            ruler_drag_mode: None,
            ruler_drag_touch_id: None,
            ruler_drag_anchor: None,
            ruler_drag_origin_center: [0.0, 0.0],
            ruler_drag_origin_angle: 0.0,
            ruler_stroke_edge: None,
            pending_delete_notebook: None,
            library_folder_filter: None,
            library_favorites_only: false,
            library_tag_filter: None,
            library_collapsed_folders: Vec::new(),
            folder_dialog_target: String::new(),
            folder_dialog_name: String::new(),
            folder_dialog_parent: String::new(),
            folder_dialog_move_notebook: None,
            folder_dialog_focus_pending: false,
            folder_dialog_color: None,
            folder_color_draft: folder_modals::FolderColorDraftState::default(),
            tag_dialog_target: String::new(),
            tag_dialog_color: None,
            tag_color_draft: folder_modals::FolderColorDraftState::default(),
            folder_picker_new_open: false,
            folder_picker_new_name: String::new(),
            notebook_folder_draft: String::new(),
            notebook_favorite_draft: false,
            notebook_tags_draft: Vec::new(),
            notebook_tag_input_draft: String::new(),
            new_notebook_folder: String::new(),
            new_notebook_favorite: false,
            new_notebook_tags: Vec::new(),
            new_notebook_tag_input: String::new(),
            new_notebook_custom_template_id: None,
            tag_edit_index: None,
            tag_edit_draft: String::new(),
            tag_edit_focus_pending: false,
            pages_bookmarked_only: false,
            pages_select_mode: false,
            pages_footer_expanded: false,
            selected_page_indices: Vec::new(),
            pages_drag_source: None,
            pending_page_reorder: None,
            transfer_pages_list: Vec::new(),
            transfer_move_mode: false,
            transfer_create_new_notebook: false,
            transfer_target_notebook: 0,
            transfer_new_title: String::new(),
            transfer_merge_whole_notebook: None,
            transfer_merge_delete_source: false,
            pending_template_import: None,
            template_import_name: String::new(),
            template_import_pdf_page: 0,
            template_import_error: None,
            settings_tab: SettingsTab::Pen,
            pan_dragging: false,
            finger_pan_touch_id: None,
            finger_pan_start: None,
            finger_pan_last: None,
            finger_pan_engaged: false,
            active_touch_id: None,
            active_pointer_stroke: false,
            straighten_reference_point: None,
            straighten_candidate_since_s: None,
            export_path_stem: "exports/Quick_Notes".to_string(),
            export_format: FileDialogKind::Pdf,
            export_target_pages: None,
            export_separate_files: false,
            export_feedback: None,
            notebook_title_draft: "Quick Notes".to_string(),
            notebook_cover_draft: papercraft_doc::NotebookCover::ClassicNavy,
            new_notebook_step: 0,
            new_notebook_preset: default_preset,
            new_notebook_title: default_preset.default_title().to_string(),
            new_notebook_cover: default_preset.cover(),
            new_notebook_template: default_preset.template(),
            new_notebook_color: default_preset.paper_color(),
            new_notebook_format: page_format_picker::PageFormatDraft::default_standard(),
            notebook_settings_step: 0,
            page_setup_format: page_format_picker::PageFormatDraft::default_standard(),
            page_setup_template: default_preset.template(),
            page_setup_color: default_preset.paper_color(),
            page_setup_custom_template_id: None,
            page_setup_clear_custom: false,
            page_setup_quarter_turns: 0,
            page_setup_apply_all: false,
            cover_color_draft: customization_modals::CoverColorDraftState::from_cover(
                default_preset.cover(),
            ),
            cached_page_key: (0, 0),
            cached_color_key: None,
            motion: motion::MotionState::default(),
            context_tray: toolbar::ContextTrayTransition::default(),
            color_dock: toolbar::ColorDockState::default(),
            color_picker: color_picker::ColorPickerState::default(),
            account: account_modal::AccountDraftState::default(),
            last_ctx_zoom: None,
            freeform_lasso_mode: false,
            shape_color_target: ShapeColorTarget::Border,
            shape_border_enabled: true,
            shape_border_color: None,
            shape_fill_enabled: false,
            shape_fill_color: None,
            shape_line_style: papercraft_doc::ShapeLineStyle::Solid,
            laser_trail_mode: true,
            laser_color: papercraft_color::Color::from_rgba8(0xEF, 0x44, 0x44, 255),
            laser_points: Vec::new(),
            elements_popover_open: false,
            smoothing_popover_open: false,
            smoothing_popover_anchor: None,
            stickers_tab: StickersTab::Starter,
            modal_open_seq: 0,
            element_name_draft: String::new(),
            text_alignment_draft: papercraft_doc::TextAlignment::Left,
            text_spans_draft: Vec::new(),
            text_prev_draft: String::new(),
            text_selection_bytes: None,
            text_selection_needs_restore: false,
            text_editor_page_size: None,
            last_copied_plain_text: None,
            rich_fonts_installed: false,
            last_canvas_rect: None,
            last_home_rect: None,
            first_access_nickname: String::new(),
            first_access_error: None,
            first_access_focus_pending: false,
            desktop_shortcut_feedback: None,
        }
    }
}

impl ViewState {
    pub fn clear_selection(&mut self) {
        self.selected_text = None;
        self.selected_object = None;
        self.selected_objects.clear();
        self.selection_drag = None;
    }
}

/// Active tab inside the Stickers & Elements popover.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StickersTab {
    #[default]
    Starter,
    Saved,
}

/// File format selected in native Desktop or browser Open/Save dialogs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileDialogKind {
    Pdf,
    Svg,
    Png,
    Pcraft,
    Package,
}

impl FileDialogKind {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Svg => "svg",
            Self::Png => "png",
            Self::Pcraft => "pcraft",
            Self::Package => "pcraftpkg",
        }
    }

    pub const fn open_extensions(self) -> &'static [&'static str] {
        match self {
            Self::Pdf => &["pdf"],
            Self::Svg => &["svg"],
            Self::Png => &["png"],
            Self::Pcraft => &["pcraft", "ncraft"],
            Self::Package => &["pcraftpkg", "ncraftpkg"],
        }
    }

    pub const fn filter_label(self) -> &'static str {
        match self {
            Self::Pdf => "PDF Document",
            Self::Svg => "SVG Vector Artwork",
            Self::Png => "PNG Image",
            Self::Pcraft => "PaperCraft Notebook",
            Self::Package => "PaperCraft Complete Package",
        }
    }

    pub const fn mime_type(self) -> &'static str {
        match self {
            Self::Pdf => "application/pdf",
            Self::Svg => "image/svg+xml",
            Self::Png => "image/png",
            Self::Pcraft | Self::Package => "application/octet-stream",
        }
    }
}

/// File payload returned by an Open File picker or drag-and-drop operation.
#[derive(Clone, Debug)]
pub enum PickedOpenFile {
    Path(PathBuf),
    Bytes { name: String, bytes: Vec<u8> },
}

/// Completion event emitted by asynchronous browser file pickers.
#[derive(Clone, Debug)]
pub enum AsyncFileEvent {
    Opened(PickedOpenFile),
    Saved { message: String },
    Cancelled,
    Error(String),
}

/// Image payload read from the OS clipboard by a platform hook.
#[derive(Clone, Debug)]
pub enum ClipboardImagePayload {
    EncodedBytes(Vec<u8>),
    FilePath(PathBuf),
}

pub type TabletPollFn = Box<dyn FnMut() -> Vec<stylus::PenSample>>;
pub type PenPointerIdFn = Box<dyn Fn() -> Option<u64>>;
pub type PenInProximityFn = Box<dyn Fn() -> bool>;
pub type PointerDeviceKindFn = Box<dyn Fn() -> stylus::PointerDeviceKind>;
pub type PenEndFn = Box<dyn Fn() -> Option<bool>>;
pub type BarrelDownFn = Box<dyn Fn() -> bool>;
pub type ButtonPressCountFn = Box<dyn FnMut() -> u32>;
pub type PenContactEventsFn = Box<dyn FnMut() -> Vec<stylus::PenContactEvent>>;
pub type ConnectedPenDevicesFn = Box<dyn FnMut() -> Vec<stylus::ConnectedStylusInfo>>;
pub type PenDevicesDirtyFn = Box<dyn FnMut() -> bool>;
pub type PickSavePathFn = Box<dyn FnMut(FileDialogKind, &str) -> Option<PathBuf>>;
pub type SaveBytesDialogFn = Box<dyn FnMut(FileDialogKind, &str, Vec<u8>)>;
pub type PickOpenFileFn = Box<dyn FnMut(FileDialogKind) -> Option<PickedOpenFile>>;
pub type PollFileEventsFn = Box<dyn FnMut() -> Vec<AsyncFileEvent>>;
pub type OpenDroppedFileFn = Box<dyn FnMut(Arc<dyn egui::DroppedFile + Send + Sync>)>;
pub type SaveWebCacheFn = Box<dyn FnMut(&str)>;
pub type PickImageFileFn = Box<dyn FnMut() -> Option<PickedOpenFile>>;
pub type ClipboardImageFn = Box<dyn FnMut() -> Option<ClipboardImagePayload>>;
pub type ClipboardImageCandidatesFn = Box<dyn FnMut() -> Vec<ClipboardImagePayload>>;
pub type ClipboardTextFn = Box<dyn FnMut() -> Option<String>>;
pub type SetClipboardTextFn = Box<dyn FnMut(&str)>;
pub type OpenUrlFn = Box<dyn FnMut(&str)>;
pub type PastePollFn = Box<dyn FnMut() -> bool>;
pub type SyncDesktopShortcutFn = Box<dyn FnMut(bool) -> Result<String, String>>;

#[derive(Default)]
pub struct PlatformHooks {
    pub tablet_poll: Option<TabletPollFn>,
    pub pen_pointer_id: Option<PenPointerIdFn>,
    pub pen_in_proximity: Option<PenInProximityFn>,
    pub pointer_device_kind: Option<PointerDeviceKindFn>,
    pub pen_end: Option<PenEndFn>,
    pub barrel_down: Option<BarrelDownFn>,
    pub secondary_barrel_down: Option<BarrelDownFn>,
    pub barrel_press_count: Option<ButtonPressCountFn>,
    pub secondary_barrel_press_count: Option<ButtonPressCountFn>,
    pub pen_contact_events: Option<PenContactEventsFn>,
    pub connected_pen_devices: Option<ConnectedPenDevicesFn>,
    pub pen_devices_dirty: Option<PenDevicesDirtyFn>,
    pub pick_save_path: Option<PickSavePathFn>,
    pub save_bytes_dialog: Option<SaveBytesDialogFn>,
    pub pick_open_file: Option<PickOpenFileFn>,
    pub poll_file_events: Option<PollFileEventsFn>,
    pub open_dropped_file: Option<OpenDroppedFileFn>,
    pub save_web_cache: Option<SaveWebCacheFn>,
    pub web_sync_dispatch: Option<sync_worker::WebSyncDispatchFn>,
    pub pick_image_file: Option<PickImageFileFn>,
    pub clipboard_image: Option<ClipboardImageFn>,
    pub clipboard_image_candidates: Option<ClipboardImageCandidatesFn>,
    pub clipboard_text: Option<ClipboardTextFn>,
    pub set_clipboard_text: Option<SetClipboardTextFn>,
    pub open_url: Option<OpenUrlFn>,
    pub paste_poll: Option<PastePollFn>,
    pub sync_desktop_shortcut: Option<SyncDesktopShortcutFn>,
    pub enable_auto_update_check: bool,
}

type AutosaveJob = (usize, PathBuf, Arc<Document>);

#[cfg(not(target_arch = "wasm32"))]
struct BackgroundAutosave {
    tx: Option<mpsc::Sender<Vec<AutosaveJob>>>,
    failed_rx: mpsc::Receiver<(usize, PathBuf)>,
    handle: Option<std::thread::JoinHandle<()>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl BackgroundAutosave {
    fn start() -> Result<Self, Vec<AutosaveJob>> {
        let (tx, rx) = mpsc::channel::<Vec<AutosaveJob>>();
        let (failed_tx, failed_rx) = mpsc::channel::<(usize, PathBuf)>();
        let handle = std::thread::Builder::new()
            .name("papercraft-autosave".to_string())
            .spawn(move || {
                while let Ok(mut batch) = rx.recv() {
                    while let Ok(more) = rx.try_recv() {
                        batch.extend(more);
                    }
                    flush_autosave_batch(batch, &failed_tx);
                }
            })
            .ok();
        match handle {
            Some(handle) => Ok(Self {
                tx: Some(tx),
                failed_rx,
                handle: Some(handle),
            }),
            None => Err(Vec::new()),
        }
    }

    fn enqueue(&mut self, jobs: Vec<AutosaveJob>) {
        if jobs.is_empty() {
            return;
        }
        if let Some(tx) = &self.tx
            && tx.send(jobs).is_err()
        {
            self.tx = None;
        }
    }

    fn requeue_failures(&self, session: &mut Session) {
        while let Ok((slot_idx, path)) = self.failed_rx.try_recv() {
            session.mark_slot_dirty_if_matching(slot_idx, &path);
        }
    }

    fn shutdown(&mut self, session: &mut Session) {
        self.tx = None;
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
        self.requeue_failures(session);
    }
}

#[cfg(target_arch = "wasm32")]
struct BackgroundAutosave;

#[cfg(target_arch = "wasm32")]
impl BackgroundAutosave {
    fn start() -> Result<Self, Vec<AutosaveJob>> {
        Err(Vec::new())
    }

    fn enqueue(&mut self, _jobs: Vec<AutosaveJob>) {}

    fn requeue_failures(&self, _session: &mut Session) {}

    fn shutdown(&mut self, _session: &mut Session) {}
}

#[cfg(not(target_arch = "wasm32"))]
fn flush_autosave_batch(jobs: Vec<AutosaveJob>, failed_tx: &mpsc::Sender<(usize, PathBuf)>) {
    // Keep only the latest snapshot per file path when multiple edits accumulated.
    let mut latest_by_path: HashMap<PathBuf, (usize, Arc<Document>)> = HashMap::new();
    for (slot_idx, path, doc) in jobs {
        latest_by_path.insert(path, (slot_idx, doc));
    }
    for (path, (slot_idx, doc)) in latest_by_path {
        if papercraft_engine::library::write_document_snapshot(&path, &doc).is_err() {
            let _ = failed_tx.send((slot_idx, path));
        }
    }
}

fn write_jobs_sync(session: &mut Session, jobs: Vec<AutosaveJob>) {
    for (slot_idx, path, doc) in jobs {
        if papercraft_engine::library::write_document_snapshot(&path, &doc).is_err() {
            session.mark_slot_dirty_if_matching(slot_idx, &path);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
struct PendingPdfImportState {
    target: PdfImportTarget,
    rx: mpsc::Receiver<Result<papercraft_engine::PreparedPdfImport, String>>,
}

/// Top-level `eframe::App` wrapping the headless [`Session`].
pub struct PaperCraftApp {
    pub session: Session,
    pub stylus: stylus::Stylus,
    pub view: ViewState,
    pub hooks: PlatformHooks,
    pub mesh_cache: HashMap<StrokeId, (Arc<InkStroke>, f32, RibbonMesh)>,
    pub(crate) image_textures: HashMap<usize, (Arc<Vec<u8>>, egui::TextureHandle)>,
    pub(crate) canvas_cache: canvas::CanvasRenderCache,
    autosave: Option<BackgroundAutosave>,
    pub(crate) sync_worker: sync_worker::BackgroundSyncWorker,
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) updater: papercraft_updater::UpdateWorker,
    #[cfg(not(target_arch = "wasm32"))]
    startup_update_checked: bool,
    #[cfg(not(target_arch = "wasm32"))]
    startup_desktop_shortcut_synced: bool,
    #[cfg(not(target_arch = "wasm32"))]
    pdf_import_job: Option<PendingPdfImportState>,
    last_ctx: Option<egui::Context>,
    frame_counter: u64,
    last_saved_cache_state: Option<WebCacheStateKey>,
    last_stylus_device_poll_s: f64,
    stylus_synced_in_raw_hook: bool,
    pub(crate) wgpu_enabled: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct WebCacheStateKey {
    revision: u64,
    notebook_count: usize,
    trashed_count: usize,
    signed_in: bool,
    viewing_cloud: bool,
    local_profile_count: usize,
    last_sync_at_ms: Option<i64>,
    active_notebook: usize,
}

impl Default for PaperCraftApp {
    fn default() -> Self {
        Self::new(Session::default(), PlatformHooks::default())
    }
}

pub(crate) const BRAND_DISPLAY_FAMILY: &str = "BrandDisplay";
pub(crate) const BRAND_DISPLAY_BOLD_TTF: &[u8] =
    include_bytes!("../assets/fonts/BricolageGrotesque-Bold.ttf");

impl PaperCraftApp {
    pub fn new(session: Session, mut hooks: PlatformHooks) -> Self {
        let account = account_modal::AccountDraftState {
            server_url: session.server_url().to_string(),
            ..Default::default()
        };
        let initial_modal = if session.needs_initial_nickname() {
            Some(modal::ActiveModal::FirstAccessNickname)
        } else {
            None
        };
        let view = ViewState {
            notebook_title_draft: session.doc.title.clone(),
            notebook_cover_draft: session.doc.cover,
            export_path_stem: inspector::title_to_export_stem(&session.doc.title),
            account,
            active_modal: initial_modal,
            first_access_focus_pending: initial_modal.is_some(),
            ..Default::default()
        };
        let mut stylus = stylus::Stylus::default();
        if let Some(query) = hooks.connected_pen_devices.as_mut() {
            stylus.set_connected_devices(query());
        }
        Self {
            session,
            stylus,
            view,
            hooks,
            mesh_cache: HashMap::new(),
            image_textures: HashMap::new(),
            canvas_cache: canvas::CanvasRenderCache::default(),
            autosave: None,
            sync_worker: sync_worker::BackgroundSyncWorker::new(),
            #[cfg(not(target_arch = "wasm32"))]
            updater: papercraft_updater::UpdateWorker::new(
                papercraft_updater::AppId::PaperCraft,
                env!("CARGO_PKG_VERSION"),
            ),
            #[cfg(not(target_arch = "wasm32"))]
            startup_update_checked: false,
            #[cfg(not(target_arch = "wasm32"))]
            startup_desktop_shortcut_synced: false,
            #[cfg(not(target_arch = "wasm32"))]
            pdf_import_job: None,
            last_ctx: None,
            frame_counter: 0,
            last_saved_cache_state: None,
            last_stylus_device_poll_s: -10.0,
            stylus_synced_in_raw_hook: false,
            wgpu_enabled: false,
        }
    }

    /// True when a hardware stylus or pen digitizer has been detected or seen in this session.
    #[must_use]
    pub fn is_stylus_connected(&self) -> bool {
        self.stylus.is_connected()
    }

    /// Initializes the GPU-retained stroke rendering pipelines when running under `eframe`'s `wgpu` backend.
    pub fn init_wgpu(&mut self, render_state: Option<&eframe::egui_wgpu::RenderState>) {
        gpu_ink::init_wgpu_state(self, render_state);
    }

    pub(crate) fn load_picked_open_file(&mut self, picked: PickedOpenFile) -> bool {
        if is_picked_package(&picked) {
            return self.import_package_from_picked(picked);
        }
        if is_picked_pdf(&picked) {
            let target = self.view.pdf_import_target;
            return self.import_pdf_from_picked(picked, target);
        }
        let result = match picked {
            PickedOpenFile::Path(path) => self
                .session
                .execute("notebook.load", &json!({ "path": path.to_string_lossy() })),
            PickedOpenFile::Bytes { name, bytes } => {
                if papercraft_engine::looks_like_package_bytes(&bytes) {
                    return self.import_package_from_picked(PickedOpenFile::Bytes { name, bytes });
                }
                self.session.load_notebook_bytes(&bytes, Some(&name), false)
            }
        };
        match result {
            Ok(_) => {
                sync_view_to_active_notebook(self);
                self.view.inspector_open = false;
                self.view.screen = AppScreen::Editor;
                tab_strip::sync_open_tabs(self);
                if self.view.active_modal == Some(modal::ActiveModal::Export) {
                    modal::close(self);
                }
                true
            }
            Err(err) => {
                self.session.last_export_status = Some(err.to_string());
                false
            }
        }
    }

    pub(crate) fn import_package_from_picked(&mut self, picked: PickedOpenFile) -> bool {
        let res = match picked {
            PickedOpenFile::Path(path) => self
                .session
                .execute("package.import", &json!({ "path": path.to_string_lossy() }))
                .map(|_| ()),
            PickedOpenFile::Bytes { bytes, .. } => {
                self.session.import_package_bytes(&bytes).map(|_| ())
            }
        };
        match res {
            Ok(()) => {
                sync_view_to_active_notebook(self);
                self.mesh_cache.clear();
                self.view.inspector_open = false;
                if self.view.active_modal == Some(modal::ActiveModal::Export) {
                    modal::close(self);
                }
                true
            }
            Err(err) => {
                self.session.last_export_status = Some(err.to_string());
                false
            }
        }
    }

    pub(crate) fn trigger_export_package_dialog(&mut self) -> bool {
        let default_name = "PaperCraft_Complete_Package.pcraftpkg".to_string();
        if let Some(pick_save) = self.hooks.pick_save_path.as_mut() {
            let Some(path) = pick_save(FileDialogKind::Package, &default_name) else {
                self.session.last_export_status = Some("Package export cancelled.".to_string());
                return false;
            };
            return match self
                .session
                .execute("package.export", &json!({ "path": path.to_string_lossy() }))
            {
                Ok(_) => true,
                Err(err) => {
                    self.session.last_export_status = Some(err.to_string());
                    false
                }
            };
        }
        if let Some(save_bytes) = self.hooks.save_bytes_dialog.as_mut() {
            return match self.session.export_package_bytes() {
                Ok(bytes) => {
                    save_bytes(FileDialogKind::Package, &default_name, bytes);
                    true
                }
                Err(err) => {
                    self.session.last_export_status = Some(err.to_string());
                    false
                }
            };
        }
        let path = inspector::export_target_with_ext(&self.view.export_path_stem, "pcraftpkg");
        match self
            .session
            .execute("package.export", &json!({ "path": path }))
        {
            Ok(_) => true,
            Err(err) => {
                self.session.last_export_status = Some(err.to_string());
                false
            }
        }
    }

    pub(crate) fn trigger_import_package_dialog(&mut self) -> bool {
        if let Some(pick) = self.hooks.pick_open_file.as_mut() {
            return match pick(FileDialogKind::Package) {
                Some(picked) => self.import_package_from_picked(picked),
                None => false,
            };
        }
        let path = inspector::export_target_with_ext(&self.view.export_path_stem, "pcraftpkg");
        self.import_package_from_picked(PickedOpenFile::Path(PathBuf::from(path)))
    }

    fn apply_prepared_pdf_result(
        &mut self,
        res: Result<papercraft_engine::PreparedPdfImport, String>,
        target: PdfImportTarget,
        persist_now: bool,
    ) -> bool {
        match res.and_then(|prepared| {
            self.session
                .apply_prepared_pdf_import(prepared, false, persist_now)
                .map_err(|err| err.to_string())
        }) {
            Ok(_) => {
                self.view.pdf_import_error = None;
                match target {
                    PdfImportTarget::NewNotebook => {
                        sync_view_to_active_notebook(self);
                        self.view.inspector_open = false;
                    }
                    PdfImportTarget::InsertIntoActive => {
                        self.mesh_cache.clear();
                        self.view.clear_selection();
                        self.view.fit_requested = true;
                    }
                }
                self.view.screen = AppScreen::Editor;
                tab_strip::sync_open_tabs(self);
                if self.view.active_modal == Some(modal::ActiveModal::Export) {
                    modal::close(self);
                }
                // Trigger background autosave promptly on the next frame instead of freezing the UI.
                if !persist_now {
                    self.view.last_autosave_s = -10.0;
                }
                true
            }
            Err(msg) => {
                self.session.last_export_status = Some(msg.clone());
                self.view.pdf_import_error = Some(msg);
                false
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn start_async_pdf_import(
        &mut self,
        picked: PickedOpenFile,
        target: PdfImportTarget,
        ctx: egui::Context,
    ) -> bool {
        let mode = match target {
            PdfImportTarget::NewNotebook => papercraft_engine::PdfImportMode::NewNotebook,
            PdfImportTarget::InsertIntoActive => papercraft_engine::PdfImportMode::InsertPages,
        };
        let display_name = match &picked {
            PickedOpenFile::Path(path) => path
                .file_name()
                .and_then(|n| n.to_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("PDF")
                .to_string(),
            PickedOpenFile::Bytes { name, .. } => {
                if name.eq_ignore_ascii_case("dropped.pcraft")
                    || name.eq_ignore_ascii_case("dropped.ncraft")
                {
                    "Imported.pdf".to_string()
                } else {
                    name.clone()
                }
            }
        };
        let (tx, rx) = mpsc::channel();
        let spawn_name = display_name.clone();
        let spawned = std::thread::Builder::new()
            .name("papercraft-pdf-import".to_string())
            .spawn(move || {
                let res = match picked {
                    PickedOpenFile::Path(path) => papercraft_engine::prepare_pdf_import_from_path(
                        &path,
                        Some(spawn_name),
                        mode,
                    ),
                    PickedOpenFile::Bytes { bytes, .. } => {
                        papercraft_engine::prepare_pdf_import_from_bytes(
                            bytes,
                            Some(spawn_name),
                            mode,
                        )
                    }
                }
                .map_err(|err| err.to_string());
                let _ = tx.send(res);
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => {
                self.view.pdf_import_error = None;
                self.view.pdf_import_in_flight = Some(display_name);
                self.pdf_import_job = Some(PendingPdfImportState { target, rx });
                true
            }
            Err(_) => false,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn poll_async_pdf_import(&mut self, ctx: &egui::Context) {
        let Some(job) = self.pdf_import_job.take() else {
            return;
        };
        match job.rx.try_recv() {
            Ok(res) => {
                self.view.pdf_import_in_flight = None;
                let _ = self.apply_prepared_pdf_result(res, job.target, false);
                ctx.request_repaint();
            }
            Err(mpsc::TryRecvError::Empty) => {
                self.pdf_import_job = Some(job);
                ctx.request_repaint_after(std::time::Duration::from_millis(80));
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.view.pdf_import_in_flight = None;
                let msg = "PDF import stopped unexpectedly.".to_string();
                self.session.last_export_status = Some(msg.clone());
                self.view.pdf_import_error = Some(msg);
            }
        }
    }

    pub(crate) fn import_pdf_from_picked(
        &mut self,
        picked: PickedOpenFile,
        target: PdfImportTarget,
    ) -> bool {
        let mode = match target {
            PdfImportTarget::NewNotebook => papercraft_engine::PdfImportMode::NewNotebook,
            PdfImportTarget::InsertIntoActive => papercraft_engine::PdfImportMode::InsertPages,
        };
        if target == PdfImportTarget::InsertIntoActive {
            canvas::commit_pending_text(self);
        }

        #[cfg(not(target_arch = "wasm32"))]
        if let Some(ctx) = self.last_ctx.clone() {
            let should_async = match &picked {
                PickedOpenFile::Path(_) => true,
                PickedOpenFile::Bytes { bytes, .. } => bytes.len() > 256 * 1024,
            };
            if should_async && self.start_async_pdf_import(picked.clone(), target, ctx) {
                return true;
            }
        }

        let res = match picked {
            PickedOpenFile::Path(path) => {
                let source_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_string);
                papercraft_engine::prepare_pdf_import_from_path(&path, source_name, mode)
            }
            PickedOpenFile::Bytes { name, bytes } => {
                let clean_name = if name.eq_ignore_ascii_case("dropped.pcraft")
                    || name.eq_ignore_ascii_case("dropped.ncraft")
                {
                    "Imported.pdf"
                } else {
                    name.as_str()
                };
                papercraft_engine::prepare_pdf_import_from_bytes(
                    bytes,
                    Some(clean_name.to_string()),
                    mode,
                )
            }
        }
        .map_err(|err| err.to_string());
        let persist_now = self.last_ctx.is_none();
        self.apply_prepared_pdf_result(res, target, persist_now)
    }

    pub(crate) fn trigger_import_pdf_dialog(&mut self, target: PdfImportTarget) -> bool {
        self.view.pdf_import_target = target;
        self.view.pdf_import_error = None;
        if let Some(pick) = self.hooks.pick_open_file.as_mut() {
            return match pick(FileDialogKind::Pdf) {
                Some(picked) => self.import_pdf_from_picked(picked, target),
                None => false,
            };
        }
        let msg = "No file picker hook available to browse for a PDF file.".to_string();
        self.session.last_export_status = Some(msg.clone());
        self.view.pdf_import_error = Some(msg);
        false
    }

    pub(crate) fn trigger_open_notebook_dialog(&mut self) -> bool {
        if let Some(pick) = self.hooks.pick_open_file.as_mut() {
            return match pick(FileDialogKind::Pcraft) {
                Some(picked) => self.load_picked_open_file(picked),
                None => false,
            };
        }
        let path = inspector::export_target_with_ext(&self.view.export_path_stem, "pcraft");
        match self
            .session
            .execute("notebook.load", &json!({ "path": path }))
        {
            Ok(_) => {
                sync_view_to_active_notebook(self);
                self.view.inspector_open = false;
                self.view.screen = AppScreen::Editor;
                tab_strip::sync_open_tabs(self);
                true
            }
            Err(err) => {
                self.session.last_export_status = Some(err.to_string());
                false
            }
        }
    }

    pub(crate) fn trigger_import_custom_template(&mut self, target: PendingTemplateImport) -> bool {
        self.view.template_import_error = None;
        self.view.pending_template_import = Some(target);
        match target {
            PendingTemplateImport::Image { .. } => {
                if let Some(pick) = self.hooks.pick_image_file.as_mut() {
                    if let Some(picked) = pick() {
                        self.view.pending_template_import = None;
                        return self.finish_template_import_from_picked(picked, target);
                    }
                    return true;
                }
                self.view.pending_template_import = None;
                let msg =
                    "No image picker hook available to browse for a template image.".to_string();
                self.view.template_import_error = Some(msg.clone());
                self.session.last_export_status = Some(msg);
                false
            }
            PendingTemplateImport::Pdf { .. } => {
                if let Some(pick) = self.hooks.pick_open_file.as_mut() {
                    if let Some(picked) = pick(FileDialogKind::Pdf) {
                        self.view.pending_template_import = None;
                        return self.finish_template_import_from_picked(picked, target);
                    }
                    return true;
                }
                self.view.pending_template_import = None;
                let msg = "No file picker hook available to browse for a PDF template.".to_string();
                self.view.template_import_error = Some(msg.clone());
                self.session.last_export_status = Some(msg);
                false
            }
        }
    }

    pub(crate) fn finish_template_import_from_picked(
        &mut self,
        picked: PickedOpenFile,
        target: PendingTemplateImport,
    ) -> bool {
        let (file_name, bytes_res) = match picked {
            PickedOpenFile::Path(path) => {
                let stem = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Custom Template")
                    .to_string();
                (stem, std::fs::read(&path).map_err(|e| e.to_string()))
            }
            PickedOpenFile::Bytes { name, bytes } => {
                let stem = std::path::Path::new(&name)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Custom Template")
                    .to_string();
                (stem, Ok(bytes))
            }
        };
        let bytes = match bytes_res {
            Ok(b) => b,
            Err(err) => {
                let msg = format!("Failed reading template file: {err}");
                self.view.template_import_error = Some(msg.clone());
                self.session.last_export_status = Some(msg);
                return false;
            }
        };
        let tpl_name = if !self.view.template_import_name.trim().is_empty() {
            self.view.template_import_name.trim().to_string()
        } else {
            file_name
        };
        let (cmd, payload, apply_to_page, apply_to_new_notebook) = match target {
            PendingTemplateImport::Image {
                apply_to_page,
                apply_to_new_notebook,
            } => (
                "template.saveFromImage",
                json!({ "name": tpl_name, "bytes": bytes }),
                apply_to_page,
                apply_to_new_notebook,
            ),
            PendingTemplateImport::Pdf {
                apply_to_page,
                apply_to_new_notebook,
            } => (
                "template.saveFromPdf",
                json!({
                    "name": tpl_name,
                    "bytes": bytes,
                    "page_index": self.view.template_import_pdf_page,
                }),
                apply_to_page,
                apply_to_new_notebook,
            ),
        };
        match self.session.execute(cmd, &payload) {
            Ok(val) => {
                self.view.template_import_name.clear();
                self.view.template_import_error = None;
                if let Some(id) = val["id"].as_str() {
                    if apply_to_page {
                        canvas::commit_pending_text(self);
                        let _ = self
                            .session
                            .execute("page.setTemplate", &json!({ "custom_template_id": id }));
                        self.mesh_cache.clear();
                        self.view.fit_requested = true;
                    }
                    if apply_to_new_notebook {
                        self.view.new_notebook_custom_template_id = Some(id.to_string());
                        if let Some(tpl) = self
                            .session
                            .prefs
                            .custom_templates
                            .iter()
                            .find(|t| t.id == id)
                        {
                            self.view.new_notebook_format =
                                page_format_picker::PageFormatDraft::from_dimensions(
                                    tpl.width, tpl.height,
                                );
                        }
                    }
                    if self.view.active_modal == Some(modal::ActiveModal::NotebookSettings) {
                        self.view.page_setup_custom_template_id = Some(id.to_string());
                        self.view.page_setup_clear_custom = false;
                        if let Some(tpl) = self
                            .session
                            .prefs
                            .custom_templates
                            .iter()
                            .find(|t| t.id == id)
                        {
                            self.view.page_setup_format =
                                page_format_picker::PageFormatDraft::from_dimensions(
                                    tpl.width, tpl.height,
                                );
                            self.view.page_setup_quarter_turns = 0;
                        }
                    }
                }
                true
            }
            Err(err) => {
                let msg = err.to_string();
                self.view.template_import_error = Some(msg.clone());
                self.session.last_export_status = Some(msg);
                false
            }
        }
    }

    fn handle_opened_file_event(&mut self, picked: PickedOpenFile) {
        if let Some(target) = self.view.pending_template_import.take() {
            let _ = self.finish_template_import_from_picked(picked, target);
            return;
        }
        if is_picked_pdf(&picked) {
            let target = self.view.pdf_import_target;
            let _ = self.import_pdf_from_picked(picked, target);
            return;
        }
        let is_image = match &picked {
            PickedOpenFile::Path(p) => {
                is_supported_image_path(p) || looks_like_raster_image_path(p)
            }
            PickedOpenFile::Bytes { name, bytes } => {
                is_supported_image_path(std::path::Path::new(name))
                    || looks_like_raster_image(bytes)
            }
        };
        if is_image {
            if let PickedOpenFile::Bytes { bytes, .. } = picked {
                let _ = self.insert_image_from_bytes(&bytes);
            } else if let PickedOpenFile::Path(path) = picked {
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        let _ = self.insert_image_from_bytes(&bytes);
                    }
                    Err(err) => {
                        let msg = format!("Failed to read image: {err}");
                        self.session.last_export_status = Some(msg.clone());
                        self.view.image_insert_error = Some(msg);
                    }
                }
            }
        } else {
            let _ = self.load_picked_open_file(picked);
        }
    }

    fn poll_async_file_events_and_drops(&mut self, ctx: &egui::Context) {
        if let Some(poll) = self.hooks.poll_file_events.as_mut() {
            let events = poll();
            for event in events {
                match event {
                    AsyncFileEvent::Opened(picked) => self.handle_opened_file_event(picked),
                    AsyncFileEvent::Saved { message } => {
                        self.session.last_export_status = Some(message);
                    }
                    AsyncFileEvent::Cancelled => {
                        self.view.pending_template_import = None;
                        self.session.last_export_status =
                            Some("File dialog cancelled.".to_string());
                    }
                    AsyncFileEvent::Error(msg) => {
                        self.view.pending_template_import = None;
                        self.session.last_export_status = Some(msg);
                    }
                }
            }
        }

        let dropped = ctx.input(|i| i.raw.dropped_files.clone());
        let drop_pdf_target = if self.view.screen == AppScreen::Editor {
            PdfImportTarget::InsertIntoActive
        } else {
            PdfImportTarget::NewNotebook
        };
        for file in dropped {
            let path = file.path();
            let is_notebook = path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                e.eq_ignore_ascii_case("pcraft") || e.eq_ignore_ascii_case("ncraft")
            });
            let is_pkg = is_package_path(path);
            if is_notebook || is_pkg {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let path_buf = path.to_path_buf();
                    if path_buf.as_os_str().is_empty() {
                        if let Ok(bytes) = file.bytes() {
                            let default_name = if is_pkg {
                                "dropped.pcraftpkg"
                            } else {
                                "dropped.pcraft"
                            };
                            let _ = self.load_picked_open_file(PickedOpenFile::Bytes {
                                name: default_name.to_string(),
                                bytes,
                            });
                        }
                    } else {
                        let _ = self.load_picked_open_file(PickedOpenFile::Path(path_buf));
                    }
                }
                #[cfg(target_arch = "wasm32")]
                if let Some(open_drop) = self.hooks.open_dropped_file.as_mut() {
                    open_drop(file);
                }
                continue;
            }

            let is_pdf = is_pdf_path(path);
            if is_pdf {
                #[cfg(not(target_arch = "wasm32"))]
                {
                    let path_buf = path.to_path_buf();
                    if path_buf.as_os_str().is_empty() {
                        if let Ok(bytes) = file.bytes() {
                            let _ = self.import_pdf_from_picked(
                                PickedOpenFile::Bytes {
                                    name: "Imported.pdf".to_string(),
                                    bytes,
                                },
                                drop_pdf_target,
                            );
                        }
                    } else {
                        let _ = self.import_pdf_from_picked(
                            PickedOpenFile::Path(path_buf),
                            drop_pdf_target,
                        );
                    }
                }
                #[cfg(target_arch = "wasm32")]
                if let Some(open_drop) = self.hooks.open_dropped_file.as_mut() {
                    self.view.pdf_import_target = drop_pdf_target;
                    open_drop(file);
                }
                continue;
            }

            #[cfg(target_arch = "wasm32")]
            if let Some(open_drop) = self.hooks.open_dropped_file.as_mut() {
                self.view.pdf_import_target = drop_pdf_target;
                open_drop(file);
                continue;
            }

            #[cfg(not(target_arch = "wasm32"))]
            if let Ok(bytes) = file.bytes() {
                if papercraft_doc::looks_like_pdf_bytes(&bytes) {
                    let _ = self.import_pdf_from_picked(
                        PickedOpenFile::Bytes {
                            name: "Imported.pdf".to_string(),
                            bytes,
                        },
                        drop_pdf_target,
                    );
                } else if papercraft_engine::looks_like_package_bytes(&bytes) {
                    let _ = self.import_package_from_picked(PickedOpenFile::Bytes {
                        name: "dropped.pcraftpkg".to_string(),
                        bytes,
                    });
                } else if self.view.screen == AppScreen::Editor {
                    let _ = self.insert_image_from_bytes(&bytes);
                }
            } else if self.view.screen == AppScreen::Editor
                && (is_supported_image_path(path) || looks_like_raster_image_path(path))
                && let Ok(bytes) = std::fs::read(path)
            {
                let _ = self.insert_image_from_bytes(&bytes);
            }
        }
    }

    pub(crate) fn flush_web_cache_if_needed(&mut self) {
        if self.hooks.save_web_cache.is_none()
            || self.session.live_stroke.is_some()
            || self.session.live_eraser.is_some()
        {
            return;
        }
        let state_key = WebCacheStateKey {
            revision: self.session.revision,
            notebook_count: self.session.notebooks.len(),
            trashed_count: self.session.trashed_notebooks.len(),
            signed_in: self.session.is_signed_in(),
            viewing_cloud: self.session.is_viewing_cloud(),
            local_profile_count: self.session.accessible_local_profiles().len(),
            last_sync_at_ms: self.session.sync_status_summary().last_sync_at_ms,
            active_notebook: self.session.active_notebook,
        };
        if self.last_saved_cache_state == Some(state_key) {
            return;
        }
        if let Ok(json) = self.session.export_web_cache_json()
            && let Some(save) = self.hooks.save_web_cache.as_mut()
        {
            save(&json);
            self.last_saved_cache_state = Some(state_key);
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.view.active_modal.is_some() {
            if self.view.active_modal == Some(modal::ActiveModal::KeyboardShortcuts)
                && ctx.input(is_shortcuts_toggle_pressed)
            {
                modal::close(self);
                return;
            }
            if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
                if self.view.active_modal == Some(modal::ActiveModal::FirstAccessNickname) {
                    return;
                }
                if self.view.tag_edit_index.is_some() {
                    self.view.tag_edit_index = None;
                    self.view.tag_edit_draft.clear();
                    self.view.tag_edit_focus_pending = false;
                } else {
                    modal::close(self);
                }
            }
            return;
        }
        if self.view.color_picker.open && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.view.color_picker.close();
            return;
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        // Extract shortcut actions inside `ctx.input` first so dispatching never runs while holding ContextImpl's RwLock.
        let action = ctx.input(|i| self.detect_keyboard_shortcut(i));
        if let Some(action) = action {
            self.execute_keyboard_shortcut(ctx, action);
        }
    }

    fn handle_editor_escape_shortcut(&mut self) -> bool {
        if self.view.text_editing {
            canvas::commit_pending_text(self);
            return true;
        }
        if self.view.text_insert_mode {
            self.view.text_insert_mode = false;
            return true;
        }
        if self.view.shape_insert_mode {
            self.view.shape_insert_mode = false;
            self.view.shape_drag_start = None;
            self.view.shape_drag_end = None;
            self.view.shape_drag_touch_id = None;
            return true;
        }
        if self.view.ruler_active {
            self.view.ruler_active = false;
            self.view.ruler_drag_mode = None;
            self.view.ruler_drag_touch_id = None;
            self.view.ruler_drag_anchor = None;
            return true;
        }
        false
    }

    fn detect_home_shortcut(&self, i: &egui::InputState) -> Option<KeyboardShortcutAction> {
        if i.modifiers.command && i.key_pressed(egui::Key::N) {
            Some(KeyboardShortcutAction::HomeNewNotebook)
        } else if i.key_pressed(egui::Key::S) && !i.modifiers.command {
            Some(KeyboardShortcutAction::HomeToggleSettings)
        } else if i.key_pressed(egui::Key::Escape) && !self.view.library_search.is_empty() {
            Some(KeyboardShortcutAction::HomeClearSearch)
        } else {
            None
        }
    }

    fn detect_keyboard_shortcut(&self, i: &egui::InputState) -> Option<KeyboardShortcutAction> {
        if is_shortcuts_toggle_pressed(i) {
            return Some(KeyboardShortcutAction::ToggleKeyboardShortcuts);
        }
        if i.modifiers.command && i.key_pressed(egui::Key::O) {
            return Some(KeyboardShortcutAction::OpenNotebookDialog);
        }
        if self.view.screen == AppScreen::Home {
            return self.detect_home_shortcut(i);
        }
        if self.view.screen == AppScreen::Profile {
            if i.key_pressed(egui::Key::Escape) {
                return Some(KeyboardShortcutAction::CloseProfile);
            }
            if i.key_pressed(egui::Key::S) && !i.modifiers.command {
                return Some(KeyboardShortcutAction::HomeToggleSettings);
            }
            return None;
        }
        let has_escape_target = self.view.text_editing
            || self.view.text_insert_mode
            || self.view.shape_insert_mode
            || self.view.ruler_active;
        if i.key_pressed(egui::Key::Escape) && has_escape_target {
            return Some(KeyboardShortcutAction::EditorEscape);
        }
        if i.key_pressed(egui::Key::V) && !i.modifiers.command {
            return Some(KeyboardShortcutAction::ActivateSelectTool);
        }
        if i.modifiers.command && i.key_pressed(egui::Key::Z) {
            return Some(if i.modifiers.shift {
                KeyboardShortcutAction::Redo
            } else {
                KeyboardShortcutAction::Undo
            });
        }
        if i.modifiers.command && i.key_pressed(egui::Key::Y) {
            return Some(KeyboardShortcutAction::Redo);
        }
        if !self.view.text_editing && i.modifiers.command && i.key_pressed(egui::Key::W) {
            return Some(KeyboardShortcutAction::CloseActiveTab);
        }
        if !self.view.text_editing && i.modifiers.command && i.key_pressed(egui::Key::V) {
            return Some(KeyboardShortcutAction::RequestPaste);
        }
        self.detect_selection_or_tool_shortcut(i)
    }

    fn detect_selection_or_tool_shortcut(
        &self,
        i: &egui::InputState,
    ) -> Option<KeyboardShortcutAction> {
        let has_selection =
            self.view.selected_object.is_some() || !self.view.selected_objects.is_empty();
        if !self.view.text_editing
            && !self.view.text_link_editor_open
            && i.modifiers.command
            && has_selection
        {
            if i.key_pressed(egui::Key::D) {
                return Some(KeyboardShortcutAction::DuplicateSelection);
            }
            if i.key_pressed(egui::Key::C) || i.key_pressed(egui::Key::X) {
                return Some(KeyboardShortcutAction::CopySelection {
                    cut: i.key_pressed(egui::Key::X),
                });
            }
        }
        if !self.view.text_editing
            && !i.modifiers.command
            && (i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace))
            && has_selection
        {
            return Some(KeyboardShortcutAction::DeleteSelection);
        }

        let tool_keys = [
            (egui::Key::Num1, "FountainPen"),
            (egui::Key::Num2, "BallPen"),
            (egui::Key::Num3, "BrushPen"),
            (egui::Key::Num4, "Highlighter"),
            (egui::Key::Num5, "Eraser"),
            (egui::Key::Num6, "Pencil"),
            (egui::Key::Num7, "LaserPointer"),
        ];
        for (key, tool) in tool_keys {
            if i.key_pressed(key) {
                return Some(KeyboardShortcutAction::SelectTool(tool));
            }
        }

        if i.key_pressed(egui::Key::OpenBracket) {
            Some(KeyboardShortcutAction::AdjustPenSize(-0.5))
        } else if i.key_pressed(egui::Key::CloseBracket) {
            Some(KeyboardShortcutAction::AdjustPenSize(0.5))
        } else if i.key_pressed(egui::Key::S) && !i.modifiers.command {
            Some(KeyboardShortcutAction::ToggleInspector)
        } else if i.key_pressed(egui::Key::P) && !i.modifiers.command {
            Some(KeyboardShortcutAction::TogglePagesSidebar)
        } else if i.key_pressed(egui::Key::H) && !i.modifiers.command {
            Some(KeyboardShortcutAction::ReturnToHome)
        } else if i.key_pressed(egui::Key::PageUp) {
            Some(KeyboardShortcutAction::StepPage(-1))
        } else if i.key_pressed(egui::Key::PageDown) {
            Some(KeyboardShortcutAction::StepPage(1))
        } else {
            None
        }
    }

    fn execute_keyboard_shortcut(&mut self, ctx: &egui::Context, action: KeyboardShortcutAction) {
        match action {
            KeyboardShortcutAction::OpenNotebookDialog => {
                let _ = self.trigger_open_notebook_dialog();
            }
            KeyboardShortcutAction::HomeNewNotebook => {
                modal::open(self, modal::ActiveModal::NewNotebook);
            }
            KeyboardShortcutAction::HomeToggleSettings => {
                if !self.view.inspector_open {
                    self.view.settings_tab = SettingsTab::Appearance;
                }
                self.view.inspector_open = !self.view.inspector_open;
            }
            KeyboardShortcutAction::HomeClearSearch => {
                self.view.library_search.clear();
            }
            KeyboardShortcutAction::EditorEscape => {
                let _ = self.handle_editor_escape_shortcut();
            }
            KeyboardShortcutAction::ActivateSelectTool => {
                activate_select_tool(self);
            }
            KeyboardShortcutAction::Undo => {
                self.view.selection_drag = None;
                let _ = self.session.execute("edit.undo", &json!({}));
            }
            KeyboardShortcutAction::Redo => {
                self.view.selection_drag = None;
                let _ = self.session.execute("edit.redo", &json!({}));
            }
            KeyboardShortcutAction::RequestPaste => {
                self.view.paste_requested = true;
            }
            KeyboardShortcutAction::DuplicateSelection => {
                let _ = duplicate_selection(self);
            }
            KeyboardShortcutAction::CopySelection { cut } => {
                let _ = copy_selection(self, Some(ctx), cut);
            }
            KeyboardShortcutAction::DeleteSelection => {
                let _ = delete_selection(self);
            }
            KeyboardShortcutAction::SelectTool(tool) => {
                canvas::commit_pending_text(self);
                self.view.text_insert_mode = false;
                self.view.clear_selection();
                self.view.text_style_open = false;
                self.view.shape_insert_mode = false;
                self.view.shape_drag_start = None;
                self.view.shape_drag_end = None;
                self.view.shape_drag_touch_id = None;
                let _ = self
                    .session
                    .execute("tool.select", &json!({ "tool": tool }));
            }
            KeyboardShortcutAction::AdjustPenSize(delta) => {
                let kind = self.session.active_pen_kind();
                let next = (self.session.pen(kind).size + delta).clamp(0.5, 80.0);
                let _ = self
                    .session
                    .execute("pen.setSettings", &json!({ "kind": kind, "size": next }));
            }
            KeyboardShortcutAction::ToggleInspector => {
                self.view.inspector_open = !self.view.inspector_open;
                if self.view.inspector_open
                    && ctx.content_rect().width() < DUAL_SIDEBAR_MIN_VIEWPORT
                {
                    self.view.pages_sidebar_open = false;
                }
            }
            KeyboardShortcutAction::TogglePagesSidebar => {
                self.view.pages_sidebar_open = !self.view.pages_sidebar_open;
                if self.view.pages_sidebar_open
                    && ctx.content_rect().width() < DUAL_SIDEBAR_MIN_VIEWPORT
                {
                    self.view.inspector_open = false;
                }
            }
            KeyboardShortcutAction::ReturnToHome => {
                return_to_home(self);
            }
            KeyboardShortcutAction::CloseActiveTab => {
                tab_strip::close_active_tab(self);
            }
            KeyboardShortcutAction::CloseProfile => {
                close_profile_screen(self);
            }
            KeyboardShortcutAction::ToggleKeyboardShortcuts => {
                if self.view.active_modal == Some(modal::ActiveModal::KeyboardShortcuts) {
                    modal::close(self);
                } else {
                    modal::open(self, modal::ActiveModal::KeyboardShortcuts);
                }
            }
            KeyboardShortcutAction::StepPage(delta) => {
                let idx = self.session.doc.active_page_index();
                let target = if delta < 0 {
                    idx.checked_sub(1)
                } else {
                    let next = idx + 1;
                    (next < self.session.doc.page_count()).then_some(next)
                };
                if let Some(next_idx) = target {
                    canvas::commit_pending_text(self);
                    if self
                        .session
                        .execute("page.select", &json!({ "index": next_idx }))
                        .is_ok()
                    {
                        self.mesh_cache.clear();
                    }
                }
            }
        }
    }
}

enum KeyboardShortcutAction {
    OpenNotebookDialog,
    HomeNewNotebook,
    HomeToggleSettings,
    HomeClearSearch,
    EditorEscape,
    ActivateSelectTool,
    Undo,
    Redo,
    RequestPaste,
    DuplicateSelection,
    CopySelection { cut: bool },
    DeleteSelection,
    SelectTool(&'static str),
    AdjustPenSize(f32),
    ToggleInspector,
    TogglePagesSidebar,
    ReturnToHome,
    CloseActiveTab,
    CloseProfile,
    ToggleKeyboardShortcuts,
    StepPage(isize),
}

fn is_shortcuts_toggle_pressed(i: &egui::InputState) -> bool {
    if i.events
        .iter()
        .any(|ev| matches!(ev, egui::Event::Text(text) if text == "?"))
    {
        return true;
    }
    !i.modifiers.command
        && !i.modifiers.alt
        && (i.key_pressed(egui::Key::Questionmark)
            || (i.modifiers.shift && i.key_pressed(egui::Key::Slash)))
}

impl PaperCraftApp {
    pub(crate) fn flush_pending_autosaves(&mut self) {
        if let Some(mut worker) = self.autosave.take() {
            worker.shutdown(&mut self.session);
        }
        let _ = self.session.save_library();
    }

    fn pump_background_autosave(&mut self, now_s: f64) {
        if let Some(worker) = &self.autosave {
            worker.requeue_failures(&mut self.session);
        }
        if self.session.has_dirty_notebooks()
            && self.session.live_stroke.is_none()
            && self.session.live_eraser.is_none()
            && now_s - self.view.last_autosave_s >= 1.5
        {
            if let Ok(jobs) = self.session.take_dirty_notebook_snapshots()
                && !jobs.is_empty()
            {
                if self.autosave.is_none()
                    && let Ok(worker) = BackgroundAutosave::start()
                {
                    self.autosave = Some(worker);
                }
                if let Some(worker) = &mut self.autosave {
                    worker.enqueue(jobs);
                } else {
                    write_jobs_sync(&mut self.session, jobs);
                }
            }
            self.view.last_autosave_s = now_s;
        }
    }

    fn sync_ui_scale_before_render(&mut self, ctx: &egui::Context) {
        let ctx_zoom = ctx.zoom_factor();
        let prefs_scale =
            papercraft_doc::UserStudioConfig::sanitize_ui_scale(self.session.prefs.ui_scale);
        match self.view.last_ctx_zoom {
            None => {
                if !ctx_zoom.is_finite() || (ctx_zoom - prefs_scale).abs() > 0.005 {
                    ctx.set_zoom_factor(prefs_scale);
                }
                self.view.last_ctx_zoom = Some(prefs_scale);
            }
            Some(prev_zoom) => {
                if !ctx_zoom.is_finite() || (ctx_zoom - prev_zoom).abs() > 0.005 {
                    let clamped = papercraft_doc::UserStudioConfig::sanitize_ui_scale(ctx_zoom);
                    if !ctx_zoom.is_finite() || (ctx_zoom - clamped).abs() > 0.005 {
                        ctx.set_zoom_factor(clamped);
                    }
                    if (prefs_scale - clamped).abs() > 0.005 {
                        let _ = self
                            .session
                            .execute("theme.set", &json!({ "ui_scale": clamped }));
                    }
                    self.view.last_ctx_zoom = Some(clamped);
                } else if (prefs_scale - prev_zoom).abs() > 0.005 {
                    ctx.set_zoom_factor(prefs_scale);
                    self.view.last_ctx_zoom = Some(prefs_scale);
                }
            }
        }
    }

    fn sync_ui_scale_after_render(&mut self, ctx: &egui::Context) {
        let prefs_scale =
            papercraft_doc::UserStudioConfig::sanitize_ui_scale(self.session.prefs.ui_scale);
        if self
            .view
            .last_ctx_zoom
            .is_some_and(|prev| (prefs_scale - prev).abs() > 0.005)
        {
            ctx.set_zoom_factor(prefs_scale);
            self.view.last_ctx_zoom = Some(prefs_scale);
        }
    }

    pub(crate) fn install_rich_fonts_if_needed(&mut self, ctx: &egui::Context) {
        if self.view.rich_fonts_installed {
            return;
        }
        let mut fonts = egui::FontDefinitions::default();
        fonts.font_data.insert(
            "bricolage_bold".to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(BRAND_DISPLAY_BOLD_TTF)),
        );
        fonts.font_data.insert(
            "roboto_regular".to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(
                papercraft_engine::ROBOTO_REGULAR_TTF,
            )),
        );
        fonts.font_data.insert(
            "roboto_bold".to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(
                papercraft_engine::ROBOTO_BOLD_TTF,
            )),
        );
        fonts.font_data.insert(
            "roboto_italic".to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(
                papercraft_engine::ROBOTO_ITALIC_TTF,
            )),
        );
        fonts.font_data.insert(
            "roboto_bold_italic".to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(
                papercraft_engine::ROBOTO_BOLD_ITALIC_TTF,
            )),
        );
        let prop_fallback = fonts
            .families
            .get(&egui::FontFamily::Proportional)
            .cloned()
            .unwrap_or_default();
        let mono_fallback = fonts
            .families
            .get(&egui::FontFamily::Monospace)
            .cloned()
            .unwrap_or_default();
        let mut brand_list = vec!["bricolage_bold".to_owned()];
        brand_list.extend(prop_fallback.iter().cloned());
        fonts.families.insert(
            egui::FontFamily::Name(BRAND_DISPLAY_FAMILY.into()),
            brand_list,
        );
        for (name, primary) in [
            ("RichSansRegular", "roboto_regular"),
            ("RichSansBold", "roboto_bold"),
            ("RichSansItalic", "roboto_italic"),
            ("RichSansBoldItalic", "roboto_bold_italic"),
        ] {
            let mut list = vec![primary.to_owned()];
            list.extend(prop_fallback.iter().cloned());
            fonts
                .families
                .insert(egui::FontFamily::Name(name.into()), list);
        }
        for (name, primary) in [
            ("RichMonoBold", "roboto_bold"),
            ("RichMonoItalic", "roboto_italic"),
            ("RichMonoBoldItalic", "roboto_bold_italic"),
        ] {
            let mut list = mono_fallback.clone();
            list.push(primary.to_owned());
            list.extend(prop_fallback.iter().cloned());
            fonts
                .families
                .insert(egui::FontFamily::Name(name.into()), list);
        }
        ctx.set_fonts(fonts);
        self.view.rich_fonts_installed = true;
    }

    /// Renders one complete UI frame including screen, sidebar, canvas, and modal transitions.
    pub fn show_ui(&mut self, ui: &mut egui::Ui) {
        self.frame_counter = self.frame_counter.saturating_add(1);
        let ctx = ui.ctx().clone();
        self.last_ctx = Some(ctx.clone());
        self.install_rich_fonts_if_needed(&ctx);
        self.sync_ui_scale_before_render(&ctx);
        pdf_view::pump_completed_renders(&ctx);
        #[cfg(not(target_arch = "wasm32"))]
        self.poll_async_pdf_import(&ctx);
        #[cfg(not(target_arch = "wasm32"))]
        self.pump_updater(&ctx);
        let (events, now_s, raw_dt) = ctx.input(|i| (i.events.clone(), i.time, i.stable_dt));
        let dt = motion::sanitize_dt(raw_dt);
        let now_ms = now_s * 1000.0;
        if now_s - self.view.last_trash_cleanup_s >= 60.0 {
            let _ = self
                .session
                .execute("notebook.trash.cleanupExpired", &serde_json::Value::Null);
            self.view.last_trash_cleanup_s = now_s;
        }
        self.stylus
            .update_for_frame(self.frame_counter, &events, now_ms);
        self.poll_connected_stylus_devices(now_s);
        if !std::mem::take(&mut self.stylus_synced_in_raw_hook) {
            self.apply_stylus_shortcuts(&ctx);
        }
        self.handle_shortcuts(&ctx);
        self.handle_page_clipboard_events(&ctx, &events);
        self.poll_async_file_events_and_drops(&ctx);
        sync_worker::poll_and_tick(self, &ctx, now_ms);
        self.pump_background_autosave(now_s);

        if self.session.tick_live_catch_up(now_ms) {
            ctx.request_repaint();
        }
        if self.session.live_stroke.is_some() || self.session.live_eraser.is_some() {
            ctx.request_repaint();
        }

        theme::apply_visuals(ui.ctx(), self.session.prefs.theme);
        if self.view.motion.screen.tick(self.view.screen, dt) {
            ctx.request_repaint();
        }

        let tokens = theme::Tokens::for_theme(self.session.prefs.theme);
        let exiting_screen = self.view.motion.screen.is_exiting();
        let tab_out = tab_strip::show(self, ui);
        match self.view.motion.screen.rendered {
            AppScreen::Home => {
                home::show(self, ui);
                tab_strip::paint_active_tab_bridge(ui, &tab_out, &tokens);
                let home_rect = self
                    .view
                    .last_home_rect
                    .unwrap_or_else(|| ui.available_rect_before_wrap());
                shortcuts_modal::show_trigger_button(self, ui, home_rect, &tokens);
            }
            AppScreen::Editor => {
                let header_rect = toolbar::show_inner(self, ui);
                tab_strip::paint_active_tab_bridge(ui, &tab_out, &tokens);
                pages_sidebar::show(self, ui);
                inspector::show(self, ui);
                let canvas_rect = ui.available_rect_before_wrap();
                toolbar::show_context_tray_in_rect(self, ui, header_rect, canvas_rect);
                shortcuts_modal::show_trigger_button(self, ui, canvas_rect, &tokens);
                canvas::show(self, ui);
                color_picker::show_popover(self, ui);
                toolbar::show_elements_popover(self, ui);
                toolbar::show_smoothing_popover(self, ui);
            }
            AppScreen::Profile => {
                profile_screen::show(self, ui);
            }
        }
        if self.view.motion.screen.rendered != self.view.screen
            || self.view.motion.screen.is_animating()
        {
            ctx.request_repaint();
        }
        if exiting_screen && self.view.motion.modal.rendered.is_none() {
            let viewport = ctx.content_rect();
            let guard_id = ui.make_persistent_id("papercraft_screen_transition_guard");
            egui::Area::new(guard_id)
                .order(egui::Order::Middle)
                .fixed_pos(viewport.min)
                .fade_in(false)
                .interactable(true)
                .show(&ctx, |ui| {
                    let _ = ui.allocate_exact_size(viewport.size(), egui::Sense::click_and_drag());
                });
        }
        motion::paint_screen_veil(ui, self.view.motion.screen.veil_alpha(), tokens.panel_bg);
        modal::show(self, ui);
        if let Some(name) = self.view.pdf_import_in_flight.as_deref() {
            render_importing_pdf_banner(&ctx, name, &tokens);
        }
        self.sync_ui_scale_after_render(&ctx);
        self.forward_output_commands(&ctx);
        self.flush_web_cache_if_needed();
    }

    fn handle_page_clipboard_events(&mut self, ctx: &egui::Context, events: &[egui::Event]) {
        let can_use_canvas_clipboard = self.view.screen == AppScreen::Editor
            && self.view.active_modal.is_none()
            && !self.view.text_editing
            && !self.view.text_link_editor_open
            && !ctx.egui_wants_keyboard_input();
        let paste_triggered = self.view.paste_requested
            || events
                .iter()
                .any(|event| matches!(event, egui::Event::PasteImage(_) | egui::Event::Paste(_)));
        self.view.paste_requested = false;
        if !can_use_canvas_clipboard {
            return;
        }
        if events.iter().any(|e| matches!(e, egui::Event::Copy)) {
            let _ = copy_selection(self, Some(ctx), false);
        }
        if events.iter().any(|e| matches!(e, egui::Event::Cut)) {
            let _ = copy_selection(self, Some(ctx), true);
        }
        if paste_triggered {
            let _ = trigger_paste_on_page(self, events);
        }
    }

    fn forward_output_commands(&mut self, ctx: &egui::Context) {
        let forwarded: Vec<(bool, String)> = ctx.output(|o| {
            o.commands
                .iter()
                .filter_map(|cmd| match cmd {
                    egui::OutputCommand::CopyText(text) => Some((true, text.clone())),
                    egui::OutputCommand::OpenUrl(open_url) => Some((false, open_url.url.clone())),
                    _ => None,
                })
                .collect()
        });
        for (is_copy, payload) in forwarded {
            if is_copy {
                if let Some(set_cb) = self.hooks.set_clipboard_text.as_mut() {
                    set_cb(&payload);
                }
            } else if let Some(clean) = sanitize_link_url(&payload)
                && let Some(open) = self.hooks.open_url.as_mut()
            {
                open(&clean);
            }
        }
    }

    fn poll_connected_stylus_devices(&mut self, now_s: f64) {
        let dirty = self
            .hooks
            .pen_devices_dirty
            .as_mut()
            .is_some_and(|poll| poll());
        let interval_due = if now_s.is_finite() {
            !self.last_stylus_device_poll_s.is_finite()
                || (now_s - self.last_stylus_device_poll_s).abs() >= 1.5
        } else {
            false
        };
        if !dirty && !interval_due {
            return;
        }
        if now_s.is_finite() {
            self.last_stylus_device_poll_s = now_s;
        }
        if let Some(query) = self.hooks.connected_pen_devices.as_mut() {
            let prev_connected = self.stylus.is_connected();
            let devices = query();
            self.stylus.set_connected_devices(devices);
            if prev_connected != self.stylus.is_connected()
                && let Some(ctx) = self.last_ctx.as_ref()
            {
                ctx.request_repaint();
            }
        }
    }

    fn cancel_active_editor_modes(&mut self) {
        self.view.text_insert_mode = false;
        self.view.text_style_open = false;
        self.view.text_link_editor_open = false;
        self.view.shape_insert_mode = false;
        self.view.shape_drag_start = None;
        self.view.shape_drag_end = None;
        self.view.shape_drag_touch_id = None;
        self.view.ruler_drag_mode = None;
        self.view.ruler_drag_touch_id = None;
        self.view.ruler_drag_anchor = None;
        self.view.color_picker.close();
    }

    fn apply_stylus_shortcuts(&mut self, ctx: &egui::Context) {
        let platform_end = self.hooks.pen_end.as_ref().and_then(|f| f());
        let platform_barrel = self.hooks.barrel_down.as_ref().is_some_and(|f| f());
        let platform_secondary_barrel = self
            .hooks
            .secondary_barrel_down
            .as_ref()
            .is_some_and(|f| f());
        let barrel_presses = self.hooks.barrel_press_count.as_mut().map_or(0, |f| f());
        let secondary_barrel_presses = self
            .hooks
            .secondary_barrel_press_count
            .as_mut()
            .map_or(0, |f| f());

        let allow_shortcuts = self.view.screen == AppScreen::Editor
            && self.view.active_modal.is_none()
            && !self.view.text_editing;
        let allow_instant_actions = allow_shortcuts && !ctx.egui_wants_keyboard_input();

        let outcome = self.stylus.evaluate_button_shortcuts(
            &self.session,
            self.view.freeform_lasso_mode,
            stylus::StylusButtonInput {
                platform_end,
                platform_barrel,
                platform_secondary_barrel,
                barrel_presses,
                secondary_barrel_presses,
                allow_shortcuts,
                allow_instant_actions,
                selection_drag_active: self.view.selection_drag.is_some(),
            },
        );

        let mut changed = outcome.pan_override_changed;
        if outcome.cancel_modes {
            self.cancel_active_editor_modes();
            changed = true;
        }
        if outcome.clear_selection {
            self.view.clear_selection();
            changed = true;
        }
        if let Some(tool) = outcome.switch_tool {
            let _ = self
                .session
                .execute("tool.select", &json!({ "tool": tool_command_name(tool) }));
            changed = true;
        }
        if let Some(pen_kind) = outcome.restore_last_pen_kind {
            self.session.last_pen_kind = pen_kind;
            changed = true;
        }
        if let Some(lasso) = outcome.set_freeform_lasso {
            self.view.freeform_lasso_mode = lasso;
            changed = true;
        }
        if let Some(cmd) = outcome.execute_command {
            self.view.selection_drag = None;
            let _ = self.session.execute(cmd, &json!({}));
            changed = true;
        }
        if changed {
            ctx.request_repaint();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn pump_updater(&mut self, ctx: &egui::Context) {
        if !self.startup_desktop_shortcut_synced {
            self.startup_desktop_shortcut_synced = true;
            #[cfg(not(debug_assertions))]
            if self.session.prefs.auto_create_desktop_shortcut
                && let Some(sync_fn) = self.hooks.sync_desktop_shortcut.as_mut()
                && let Err(err) = sync_fn(true)
            {
                log::warn!("Startup Desktop shortcut sync failed: {err}");
                self.view.desktop_shortcut_feedback = Some((true, err));
            }
        }
        if self.updater.poll() {
            ctx.request_repaint();
        }
        if !self.startup_update_checked && self.hooks.enable_auto_update_check {
            self.startup_update_checked = true;
            if self.session.prefs.auto_check_updates {
                self.trigger_update_check(true, ctx);
            }
        }
        if self.updater.is_busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(120));
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn set_desktop_shortcut_enabled(&mut self, enabled: bool) {
        let _ = self
            .session
            .execute("updates.setDesktopShortcut", &json!({ "enabled": enabled }));
        self.view.desktop_shortcut_feedback = None;
        if let Some(sync_fn) = self.hooks.sync_desktop_shortcut.as_mut()
            && let Err(err) = sync_fn(enabled)
        {
            self.view.desktop_shortcut_feedback = Some((true, err));
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn trigger_update_check(&mut self, silent: bool, ctx: &egui::Context) {
        let repaint_ctx = ctx.clone();
        self.updater
            .check_now(silent, Arc::new(move || repaint_ctx.request_repaint()));
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn start_update_download(&mut self, ctx: &egui::Context) {
        let repaint_ctx = ctx.clone();
        self.updater
            .start_download(Arc::new(move || repaint_ctx.request_repaint()));
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn cancel_update_download(&mut self) {
        self.updater.cancel_download();
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn apply_ready_update_and_restart(
        &mut self,
        ctx: &egui::Context,
    ) -> Result<(), String> {
        canvas::commit_pending_text(self);
        self.session.cancel_live_stroke();
        self.session.cancel_live_eraser();
        self.flush_pending_autosaves();
        self.sync_worker.shutdown();
        self.updater
            .apply_ready_update_and_relaunch_with_shortcut(true)?;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        Ok(())
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn open_external_url(&mut self, url: &str) {
        if let Some(clean) = sanitize_link_url(url)
            && let Some(open) = self.hooks.open_url.as_mut()
        {
            open(&clean);
        }
    }
}

fn tool_command_name(tool: papercraft_engine::ToolKind) -> &'static str {
    match tool {
        papercraft_engine::ToolKind::Select => "Select",
        papercraft_engine::ToolKind::Pen(PenKind::FountainPen) => "FountainPen",
        papercraft_engine::ToolKind::Pen(PenKind::BallPen) => "BallPen",
        papercraft_engine::ToolKind::Pen(PenKind::BrushPen) => "BrushPen",
        papercraft_engine::ToolKind::Pen(PenKind::Highlighter) => "Highlighter",
        papercraft_engine::ToolKind::Pen(PenKind::Pencil) => "Pencil",
        papercraft_engine::ToolKind::Eraser => "Eraser",
        papercraft_engine::ToolKind::LaserPointer => "LaserPointer",
    }
}

impl eframe::App for PaperCraftApp {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        let os_pen_id = self.hooks.pen_pointer_id.as_ref().and_then(|f| f());
        let pen_in_proximity = self
            .hooks
            .pen_in_proximity
            .as_ref()
            .map_or(os_pen_id.is_some(), |f| f());
        let pointer_kind = self
            .hooks
            .pointer_device_kind
            .as_ref()
            .map_or(stylus::PointerDeviceKind::Unknown, |f| f());
        self.stylus.set_pointer_device_kind(pointer_kind);
        self.stylus.sanitize_raw_input_with_proximity(
            raw_input,
            os_pen_id,
            pen_in_proximity,
            self.hooks.pen_pointer_id.is_some(),
        );
        if let Some(poll) = self.hooks.tablet_poll.as_mut() {
            let samples = poll();
            if !samples.is_empty() {
                self.stylus.feed.push_batch(samples);
            }
        }
        self.apply_stylus_shortcuts(ctx);
        self.stylus.remap_barrel_contact_events(raw_input);
        if let Some(poll_contacts) = self.hooks.pen_contact_events.as_mut() {
            let contacts = poll_contacts();
            self.stylus.apply_pen_contact_events(raw_input, &contacts);
        }
        self.stylus_synced_in_raw_hook = true;
        if self.hooks.paste_poll.as_mut().is_some_and(|poll| poll()) {
            self.view.paste_requested = true;
            let already_has_paste = raw_input
                .events
                .iter()
                .any(|e| matches!(e, egui::Event::Paste(_) | egui::Event::PasteImage(_)));
            let text_widget_active = self.view.text_editing
                || self.view.text_link_editor_open
                || ctx.egui_wants_keyboard_input();
            if !already_has_paste
                && text_widget_active
                && let Some(text) = self.hooks.clipboard_text.as_mut().and_then(|f| f())
                && !text.is_empty()
            {
                raw_input.events.push(egui::Event::Paste(text));
            }
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show_ui(ui);
    }

    fn on_exit(&mut self) {
        self.session.cancel_live_stroke();
        self.session.cancel_live_eraser();
        self.sync_worker.shutdown();
        if let Some(mut worker) = self.autosave.take() {
            worker.shutdown(&mut self.session);
        }
        let _ = self.session.save_library();
        self.flush_web_cache_if_needed();
    }
}

fn render_importing_pdf_banner(ctx: &egui::Context, name: &str, t: &theme::Tokens) {
    let rect = ctx.content_rect();
    egui::Area::new(egui::Id::unique("papercraft_pdf_importing_banner"))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::CENTER_TOP)
        .fixed_pos(egui::pos2(rect.center().x, rect.top() + 14.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(t.panel_bg)
                .stroke(egui::Stroke::new(1.0, t.accent))
                .corner_radius(egui::CornerRadius::same(8))
                .inner_margin(egui::Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        ui.add(egui::Spinner::new().size(14.0).color(t.accent));
                        ui.label(
                            egui::RichText::new(format!("Importing \"{name}\"..."))
                                .size(12.5)
                                .color(t.text),
                        );
                    });
                });
        });
}

pub(crate) fn sync_view_to_active_notebook(app: &mut PaperCraftApp) {
    app.view.text_insert_mode = false;
    app.view.text_editing = false;
    app.view.text_edit_index = None;
    app.view.text_max_width_draft = None;
    app.view.text_link_draft = None;
    app.view.text_link_input.clear();
    app.view.text_link_editor_open = false;
    app.view.text_alignment_draft = papercraft_doc::TextAlignment::Left;
    app.view.text_spans_draft.clear();
    app.view.text_prev_draft.clear();
    app.view.text_selection_bytes = None;
    app.view.text_selection_needs_restore = false;
    app.view.text_editor_page_size = None;
    app.view.elements_popover_open = false;
    app.view.laser_points.clear();
    app.view.last_canvas_page_pos = None;
    app.view.text_insert_error = None;
    app.view.image_insert_error = None;
    app.view.clear_selection();
    app.view.paste_requested = false;
    app.image_textures.clear();
    app.view.text_style_open = false;
    app.view.shape_insert_mode = false;
    app.view.shape_drag_start = None;
    app.view.shape_drag_end = None;
    app.view.shape_drag_touch_id = None;
    app.view.ruler_active = false;
    app.view.ruler_initialized = false;
    app.view.ruler_depth = 0.0;
    app.view.ruler_drag_mode = None;
    app.view.ruler_drag_touch_id = None;
    app.view.ruler_drag_anchor = None;
    app.view.ruler_stroke_edge = None;
    app.mesh_cache.clear();
    app.image_textures.clear();
    app.view.paste_requested = false;
    app.view.fit_requested = true;
    app.view.motion.camera_glide = None;
    app.view.motion.fit_glide_requested = false;
    app.view.cached_page_key = (
        app.session.active_notebook,
        app.session.doc.active_page_index(),
    );
    app.view.notebook_title_draft = app.session.doc.title.clone();
    app.view.notebook_cover_draft = app.session.doc.cover;
    app.view.notebook_folder_draft = app.session.doc.folder_path.clone();
    app.view.notebook_favorite_draft = app.session.doc.favorite;
    app.view.notebook_tags_draft = app.session.doc.tags.clone();
    app.view.notebook_tag_input_draft.clear();
    app.view.pages_bookmarked_only = false;
    app.view.pages_select_mode = false;
    app.view.selected_page_indices.clear();
    app.view.export_target_pages = None;
    app.view.pages_drag_source = None;
    app.view.pending_page_reorder = None;
    app.view.export_path_stem = inspector::title_to_export_stem(&app.session.doc.title);
}

pub(crate) fn open_notebook_in_editor(app: &mut PaperCraftApp, index: usize) {
    canvas::commit_pending_text(app);
    if app
        .session
        .execute("notebook.select", &json!({ "index": index }))
        .is_ok()
    {
        sync_view_to_active_notebook(app);
        app.view.inspector_open = false;
        app.view.screen = AppScreen::Editor;
        tab_strip::sync_open_tabs(app);
    }
}

pub(crate) fn activate_select_tool(app: &mut PaperCraftApp) {
    canvas::commit_pending_text(app);
    app.view.text_insert_mode = false;
    app.view.text_style_open = false;
    app.view.text_link_editor_open = false;
    app.view.selection_drag = None;
    app.view.shape_insert_mode = false;
    app.view.shape_drag_start = None;
    app.view.shape_drag_end = None;
    app.view.shape_drag_touch_id = None;
    app.view.ruler_active = false;
    app.view.ruler_drag_mode = None;
    app.view.ruler_drag_touch_id = None;
    app.view.ruler_drag_anchor = None;
    app.view.color_picker.close();
    let _ = app
        .session
        .execute("tool.select", &json!({ "tool": "Select" }));
}

fn is_supported_image_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "png"
                    | "jpg"
                    | "jpeg"
                    | "jfif"
                    | "webp"
                    | "gif"
                    | "bmp"
                    | "dib"
                    | "ico"
                    | "tif"
                    | "tiff"
                    | "tga"
                    | "pnm"
                    | "ppm"
                    | "pgm"
                    | "pbm"
                    | "qoi"
            )
        })
}

fn looks_like_raster_image(bytes: &[u8]) -> bool {
    image::guess_format(bytes).is_ok()
        || (bytes.len() >= 26 && &bytes[bytes.len() - 18..bytes.len() - 2] == b"TRUEVISION-XFILE")
}

#[cfg(not(target_arch = "wasm32"))]
fn looks_like_raster_image_path(path: &std::path::Path) -> bool {
    use std::io::Read;
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() || meta.len() == 0 || meta.len() > 64 * 1024 * 1024 {
        return false;
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0_u8; 64];
    let Ok(n) = file.read(&mut header) else {
        return false;
    };
    image::guess_format(&header[..n]).is_ok()
}

#[cfg(target_arch = "wasm32")]
fn looks_like_raster_image_path(_path: &std::path::Path) -> bool {
    false
}

impl PaperCraftApp {
    pub(crate) fn active_camera_viewport(&self) -> Option<(egui::Rect, f32, [f64; 2])> {
        if self.view.fit_requested {
            return None;
        }
        let rect = self.view.last_canvas_rect?;
        if !(rect.is_finite() && rect.width() > 1.0 && rect.height() > 1.0) {
            return None;
        }
        let (zoom, pan) = match self.view.motion.camera_glide {
            Some(glide) => (glide.target_zoom, glide.target_pan),
            None => (self.view.zoom, self.view.pan),
        };
        if !(zoom.is_finite() && zoom > 1e-6 && pan[0].is_finite() && pan[1].is_finite()) {
            return None;
        }
        Some((rect, zoom, pan))
    }

    pub(crate) fn camera_center_page_point(&self) -> papercraft_geom::Point {
        let page_w = f64::from(self.session.doc.width);
        let page_h = f64::from(self.session.doc.height);
        let fallback = papercraft_geom::Point::new(page_w * 0.5, page_h * 0.5);
        let Some((rect, zoom, pan)) = self.active_camera_viewport() else {
            return fallback;
        };
        let xf = papercraft_geom::Affine2::new(f64::from(zoom), pan[0], pan[1]);
        let center_screen = rect.center();
        let pt = xf.to_page(papercraft_geom::Point::new(
            f64::from(center_screen.x),
            f64::from(center_screen.y),
        ));
        if !pt.is_finite() {
            return fallback;
        }
        papercraft_geom::Point::new(pt.x.clamp(0.0, page_w), pt.y.clamp(0.0, page_h))
    }

    pub(crate) fn insert_image_from_bytes(&mut self, bytes: &[u8]) -> bool {
        const MAX_UI_RAW_IMAGE_BYTES: usize = 64 * 1024 * 1024;
        if bytes.is_empty() {
            let msg = "Selected image file is empty.".to_string();
            self.session.last_export_status = Some(msg.clone());
            self.view.image_insert_error = Some(msg);
            return false;
        }
        if bytes.len() > MAX_UI_RAW_IMAGE_BYTES {
            let msg = "Image exceeds the 64 MiB maximum file size.".to_string();
            self.session.last_export_status = Some(msg.clone());
            self.view.image_insert_error = Some(msg);
            return false;
        }
        let center = self.camera_center_page_point();
        let payload = json!({
            "base64_bytes": base64_encode_bytes(bytes),
            "center_x": center.x,
            "center_y": center.y,
        });
        match self.session.execute("image.insert", &payload) {
            Ok(result) => {
                canvas::commit_pending_text(self);
                self.view.text_insert_mode = false;
                self.view.text_style_open = false;
                self.view.shape_insert_mode = false;
                self.view.shape_drag_start = None;
                self.view.shape_drag_end = None;
                self.view.shape_drag_touch_id = None;
                self.view.ruler_active = false;
                self.view.image_insert_error = None;
                self.view.clear_selection();
                let _ = self
                    .session
                    .execute("tool.select", &json!({ "tool": "Select" }));
                if let Some(index) = result["imageIndex"]
                    .as_u64()
                    .and_then(|idx| usize::try_from(idx).ok())
                {
                    let obj = SelectedObject::Image(index);
                    self.view.selected_object = Some(obj);
                    self.view.selected_objects = vec![obj];
                }
                self.view.screen = AppScreen::Editor;
                true
            }
            Err(err) => {
                let msg = err.to_string();
                self.session.last_export_status = Some(msg.clone());
                self.view.image_insert_error = Some(msg);
                false
            }
        }
    }
}

pub(crate) fn trigger_open_image_dialog(app: &mut PaperCraftApp) -> bool {
    let picked = app.hooks.pick_image_file.as_mut().and_then(|pick| pick());
    match picked {
        Some(PickedOpenFile::Path(path)) => match std::fs::read(&path) {
            Ok(bytes) => app.insert_image_from_bytes(&bytes),
            Err(err) => {
                let msg = format!("Failed to read image: {err}");
                app.session.last_export_status = Some(msg.clone());
                app.view.image_insert_error = Some(msg);
                false
            }
        },
        Some(PickedOpenFile::Bytes { bytes, .. }) => app.insert_image_from_bytes(&bytes),
        None => false,
    }
}

fn collect_clipboard_image_candidates(app: &mut PaperCraftApp) -> Vec<ClipboardImagePayload> {
    if let Some(get_all) = app.hooks.clipboard_image_candidates.as_mut() {
        let candidates = get_all();
        if !candidates.is_empty() {
            return candidates;
        }
    }
    app.hooks
        .clipboard_image
        .as_mut()
        .and_then(|f| f())
        .into_iter()
        .collect()
}

pub(crate) fn trigger_paste_image(app: &mut PaperCraftApp, events: &[egui::Event]) -> bool {
    let candidates = collect_clipboard_image_candidates(app);
    for candidate in candidates {
        let inserted = match candidate {
            ClipboardImagePayload::EncodedBytes(bytes) => app.insert_image_from_bytes(&bytes),
            ClipboardImagePayload::FilePath(path) => match std::fs::read(&path) {
                Ok(bytes) => app.insert_image_from_bytes(&bytes),
                Err(_) => false,
            },
        };
        if inserted {
            return true;
        }
    }

    for event in events {
        if let egui::Event::PasteImage(color_image) = event {
            let w = u32::try_from(color_image.width()).unwrap_or(0);
            let h = u32::try_from(color_image.height()).unwrap_or(0);
            if w > 0 && h > 0 {
                let mut rgba = Vec::with_capacity(color_image.pixels.len().saturating_mul(4));
                for pixel in &color_image.pixels {
                    rgba.extend_from_slice(&pixel.to_srgba_unmultiplied());
                }
                if let Ok(png) = papercraft_raster::png_encode(w, h, &rgba)
                    && app.insert_image_from_bytes(&png)
                {
                    return true;
                }
            }
        }
    }
    false
}

fn clean_clipboard_text(raw: &str) -> String {
    raw.replace("\r\n", "\n")
        .replace('\r', "\n")
        .chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .take(MAX_TEXT_CHARS)
        .collect()
}

pub(crate) fn selected_text_copy_payload(app: &mut PaperCraftApp) -> Option<String> {
    let mut indices = Vec::new();
    if app.view.selected_objects.is_empty() {
        if let Some(SelectedObject::Text(index)) = app.view.selected_object {
            indices.push(index);
        }
    } else {
        for obj in &app.view.selected_objects {
            if let SelectedObject::Text(index) = *obj {
                indices.push(index);
            }
        }
    }
    if indices.is_empty() {
        return None;
    }
    let mut texts = Vec::with_capacity(indices.len());
    for index in indices {
        if let Some(item) = app.session.doc.texts.get(index) {
            let clean = item.as_ref().sanitized();
            if app.view.copied_text_item.is_none() || texts.is_empty() {
                app.view.copied_text_item = Some(clean.clone());
            }
            texts.push(clean.text);
        }
    }
    (!texts.is_empty()).then(|| texts.join("\n"))
}

pub(crate) fn open_external_url(app: &mut PaperCraftApp, ctx: &egui::Context, url: &str) -> bool {
    let Some(clean) = sanitize_link_url(url) else {
        return false;
    };
    if let Some(open) = app.hooks.open_url.as_mut() {
        open(&clean);
    } else {
        ctx.open_url(egui::OpenUrl::new_tab(clean));
    }
    true
}

pub(crate) fn insert_text_on_page(
    app: &mut PaperCraftApp,
    raw_text: &str,
    explicit_link: Option<String>,
) -> bool {
    let cleaned = clean_clipboard_text(raw_text);
    if cleaned.trim().is_empty() {
        return false;
    }
    let page_w = f64::from(app.session.doc.width.max(64));
    let page_h = f64::from(app.session.doc.height.max(64));
    let copied_match = app
        .view
        .copied_text_item
        .as_ref()
        .filter(|item| item.text == cleaned)
        .cloned();

    let mut payload = if let Some(mut item) = copied_match {
        let next_x = (item.x + 18.0).clamp(16.0, (page_w - 48.0).max(16.0));
        let next_y = (item.y + 18.0).clamp(16.0, (page_h - 48.0).max(16.0));
        item.x = next_x;
        item.y = next_y;
        app.view.copied_text_item = Some(item.clone());
        let resolved_link = explicit_link
            .as_deref()
            .and_then(sanitize_link_url)
            .or(item.link_url);
        let mut p = json!({
            "text": cleaned,
            "x": next_x,
            "y": next_y,
            "size": item.size,
            "angle_degrees": item.angle_degrees,
            "color": item.color,
            "font": item.font,
            "line_spacing": item.line_spacing,
            "letter_spacing": item.letter_spacing,
            "max_width": item.max_width,
        });
        if let Some(link) = resolved_link {
            p["link_url"] = json!(link);
        }
        p
    } else {
        let [x, y] = app.view.last_canvas_page_pos.unwrap_or([
            (page_w * 0.18).clamp(24.0, (page_w - 64.0).max(24.0)),
            (page_h * 0.25).clamp(24.0, (page_h - 64.0).max(24.0)),
        ]);
        let auto_link = sanitize_link_url(cleaned.trim());
        let resolved_link = explicit_link
            .as_deref()
            .and_then(sanitize_link_url)
            .or_else(|| {
                if app.view.text_insert_mode {
                    app.view
                        .text_link_draft
                        .as_deref()
                        .and_then(sanitize_link_url)
                } else {
                    None
                }
            })
            .or(auto_link);
        let mut p = json!({
            "text": cleaned,
            "x": x.clamp(8.0, (page_w - 32.0).max(8.0)),
            "y": y.clamp(8.0, (page_h - 32.0).max(8.0)),
            "size": app.view.text_size_draft,
            "color": app.view.text_color_draft,
            "font": app.view.text_font_draft,
            "line_spacing": app.view.text_line_spacing_draft,
            "letter_spacing": app.view.text_letter_spacing_draft,
        });
        if let Some(link) = resolved_link {
            p["link_url"] = json!(link);
        }
        p
    };
    let _ = &mut payload;

    match app.session.execute("text.insert", &payload) {
        Ok(result) => {
            canvas::commit_pending_text(app);
            app.view.text_insert_mode = false;
            app.view.text_style_open = false;
            app.view.text_link_editor_open = false;
            app.view.text_link_draft = None;
            app.view.text_link_input.clear();
            app.view.shape_insert_mode = false;
            app.view.shape_drag_start = None;
            app.view.shape_drag_end = None;
            app.view.shape_drag_touch_id = None;
            app.view.ruler_active = false;
            app.view.clear_selection();
            let _ = app
                .session
                .execute("tool.select", &json!({ "tool": "Select" }));
            if let Some(index) = result["textIndex"]
                .as_u64()
                .and_then(|idx| usize::try_from(idx).ok())
                .or_else(|| app.session.doc.texts.len().checked_sub(1))
            {
                let obj = SelectedObject::Text(index);
                app.view.selected_object = Some(obj);
                app.view.selected_objects = vec![obj];
                app.view.selected_text = Some(index);
            }
            app.view.screen = AppScreen::Editor;
            true
        }
        Err(err) => {
            app.session.last_export_status = Some(err.to_string());
            false
        }
    }
}

pub(crate) fn active_selection_list(app: &PaperCraftApp) -> Vec<SelectedObject> {
    if !app.view.selected_objects.is_empty() {
        app.view.selected_objects.clone()
    } else {
        app.view.selected_object.into_iter().collect()
    }
}

pub(crate) fn select_stamped_result(app: &mut PaperCraftApp, res: &serde_json::Value) {
    let mut selected = Vec::new();
    if let Some(arr) = res["stroke_ids"].as_array() {
        for v in arr {
            if let Some(id) = v.as_u64() {
                selected.push(SelectedObject::Stroke(papercraft_doc::StrokeId(id)));
            }
        }
    }
    if let Some(arr) = res["shape_indices"].as_array() {
        for v in arr {
            if let Some(idx) = v.as_u64() {
                selected.push(SelectedObject::Shape(idx as usize));
            }
        }
    }
    if let Some(arr) = res["text_indices"].as_array() {
        for v in arr {
            if let Some(idx) = v.as_u64() {
                selected.push(SelectedObject::Text(idx as usize));
            }
        }
    }
    if let Some(arr) = res["image_indices"].as_array() {
        for v in arr {
            if let Some(idx) = v.as_u64() {
                selected.push(SelectedObject::Image(idx as usize));
            }
        }
    }
    if !selected.is_empty() {
        let _ = app
            .session
            .execute("tool.select", &json!({ "tool": "Select" }));
        app.view.text_insert_mode = false;
        app.view.shape_insert_mode = false;
        app.view.selected_text = selected.iter().find_map(|o| match o {
            SelectedObject::Text(idx) => Some(*idx),
            _ => None,
        });
        app.view.selected_object = selected.first().copied();
        app.view.selected_objects = selected;
        app.mesh_cache.clear();
    }
}

pub(crate) fn copy_selection(
    app: &mut PaperCraftApp,
    ctx: Option<&egui::Context>,
    cut: bool,
) -> bool {
    let objects = active_selection_list(app);
    let targets = toolbar::selected_targets_json(&objects);
    if targets.is_empty() {
        return false;
    }
    let text_payload = selected_text_copy_payload(app);
    match app
        .session
        .execute("selection.copy", &json!({ "targets": targets, "cut": cut }))
    {
        Ok(_) => {
            app.view.last_copied_plain_text = text_payload.clone();
            if let Some(text) = text_payload {
                if let Some(set_cb) = app.hooks.set_clipboard_text.as_mut() {
                    set_cb(&text);
                }
                if let Some(ctx) = ctx {
                    ctx.copy_text(text);
                }
            }
            if cut {
                app.view.clear_selection();
                app.image_textures.clear();
            }
            true
        }
        Err(err) => {
            app.session.last_export_status = Some(err.to_string());
            false
        }
    }
}

pub(crate) fn duplicate_selection(app: &mut PaperCraftApp) -> bool {
    let objects = active_selection_list(app);
    let targets = toolbar::selected_targets_json(&objects);
    if targets.is_empty() {
        return false;
    }
    match app
        .session
        .execute("selection.duplicate", &json!({ "targets": targets }))
    {
        Ok(val) => {
            select_stamped_result(app, &val);
            true
        }
        Err(err) => {
            app.session.last_export_status = Some(err.to_string());
            false
        }
    }
}

pub(crate) fn trigger_paste_on_page(app: &mut PaperCraftApp, events: &[egui::Event]) -> bool {
    let has_paste_image_event = events
        .iter()
        .any(|event| matches!(event, egui::Event::PasteImage(_)));
    if has_paste_image_event && trigger_paste_image(app, events) {
        return true;
    }
    let event_paste_text = events.iter().find_map(|event| match event {
        egui::Event::Paste(text) if !text.trim().is_empty() => Some(text.as_str()),
        _ => None,
    });
    if app.session.clipboard.is_some() {
        let external_text_override = event_paste_text.is_some_and(|txt| {
            app.view
                .last_copied_plain_text
                .as_deref()
                .is_none_or(|last| last != txt)
        });
        if !external_text_override
            && !has_paste_image_event
            && let Ok(val) = app.session.execute("selection.paste", &json!({}))
        {
            select_stamped_result(app, &val);
            return true;
        }
    }
    if event_paste_text.is_none() && trigger_paste_image(app, events) {
        return true;
    }
    if let Some(text) = event_paste_text {
        return insert_text_on_page(app, text, None);
    }
    if trigger_paste_image(app, events) {
        return true;
    }
    if let Some(text) = app.hooks.clipboard_text.as_mut().and_then(|f| f())
        && !text.trim().is_empty()
    {
        let matches_internal = app
            .view
            .last_copied_plain_text
            .as_deref()
            .is_some_and(|last| last == text);
        if matches_internal
            && app.session.clipboard.is_some()
            && let Ok(val) = app.session.execute("selection.paste", &json!({}))
        {
            select_stamped_result(app, &val);
            return true;
        }
        return insert_text_on_page(app, &text, None);
    }
    false
}

pub(crate) fn delete_selection(app: &mut PaperCraftApp) -> bool {
    let objects = if !app.view.selected_objects.is_empty() {
        app.view.selected_objects.clone()
    } else if let Some(obj) = app.view.selected_object {
        vec![obj]
    } else {
        return false;
    };
    let targets = toolbar::selected_targets_json(&objects);
    if targets.is_empty() {
        return false;
    }
    match app
        .session
        .execute("selection.delete", &json!({ "targets": targets }))
    {
        Ok(_) => {
            app.view.clear_selection();
            app.image_textures.clear();
            true
        }
        Err(err) => {
            app.session.last_export_status = Some(err.to_string());
            false
        }
    }
}

pub(crate) fn return_to_home(app: &mut PaperCraftApp) {
    canvas::commit_pending_text(app);
    app.view.text_insert_mode = false;
    app.view.text_editing = false;
    app.view.text_edit_index = None;
    app.view.text_max_width_draft = None;
    app.view.text_link_draft = None;
    app.view.text_link_input.clear();
    app.view.text_link_editor_open = false;
    app.view.text_alignment_draft = papercraft_doc::TextAlignment::Left;
    app.view.text_spans_draft.clear();
    app.view.text_prev_draft.clear();
    app.view.text_selection_bytes = None;
    app.view.text_selection_needs_restore = false;
    app.view.text_editor_page_size = None;
    app.view.elements_popover_open = false;
    app.view.laser_points.clear();
    app.view.last_canvas_page_pos = None;
    app.view.text_insert_error = None;
    app.view.image_insert_error = None;
    app.view.clear_selection();
    app.view.paste_requested = false;
    app.image_textures.clear();
    app.view.text_style_open = false;
    app.view.shape_insert_mode = false;
    app.view.shape_drag_start = None;
    app.view.shape_drag_end = None;
    app.view.shape_drag_touch_id = None;
    app.view.ruler_active = false;
    app.view.ruler_drag_mode = None;
    app.view.ruler_drag_touch_id = None;
    app.view.ruler_drag_anchor = None;
    app.view.ruler_stroke_edge = None;
    app.view.color_picker.close();
    app.session.cancel_live_stroke();
    app.session.cancel_live_eraser();
    app.session.sync_active_notebook_slot();
    let _ = app.session.save_library();
    app.flush_web_cache_if_needed();
    app.view.inspector_open = false;
    app.view.profile_return_screen = AppScreen::Home;
    app.view.screen = AppScreen::Home;
}

pub(crate) fn open_profile_screen(app: &mut PaperCraftApp) {
    if app.view.screen == AppScreen::Profile {
        return;
    }
    canvas::commit_pending_text(app);
    app.view.profile_return_screen = app.view.screen;
    app.view.inspector_open = false;
    crate::account_modal::sync_draft_from_session(app);
    app.view.screen = AppScreen::Profile;
}

pub(crate) fn close_profile_screen(app: &mut PaperCraftApp) {
    app.view.inspector_open = false;
    let target = match app.view.profile_return_screen {
        AppScreen::Editor if app.session.active_notebook < app.session.notebooks.len() => {
            AppScreen::Editor
        }
        _ => AppScreen::Home,
    };
    app.view.screen = target;
}

/// Standardized width shared by the left navigation sidebars (Library and Pages)
/// across Home and Editor; the right Settings drawer keeps its wider `DEFAULT_INSPECTOR_WIDTH`.
pub(crate) const STANDARD_SIDEBAR_WIDTH: f32 = 280.0;

/// Minimum viewport width at which interactive Editor drawer toggles keep both
/// left (`STANDARD_SIDEBAR_WIDTH`) and right (`DEFAULT_INSPECTOR_WIDTH`) sidebars
/// open simultaneously instead of auto-collapsing the opposite one.
pub(crate) const DUAL_SIDEBAR_MIN_VIEWPORT: f32 =
    STANDARD_SIDEBAR_WIDTH + inspector::DEFAULT_INSPECTOR_WIDTH + 184.0;

/// Clamps a fixed side-panel width to available horizontal space, shrinking
/// symmetrically when both left and right panels share a narrow viewport so
/// neither sidebar starves the opposite panel or the central workspace.
pub(crate) fn responsive_side_panel_width(
    ui: &egui::Ui,
    default_width: f32,
    peer_open: bool,
) -> f32 {
    let target = if default_width.is_finite() {
        default_width.max(0.0)
    } else {
        0.0
    };
    let avail = ui.available_rect_before_wrap().width();
    if !avail.is_finite() {
        return target;
    }
    let avail = avail.max(0.0);
    let content_w = ui.ctx().content_rect().width();
    let total_w = if content_w.is_finite() && content_w > 0.0 {
        content_w.max(avail)
    } else {
        avail
    };
    let taken_by_peer = (total_w - avail).max(0.0);
    let dual_active = peer_open || taken_by_peer > 0.5;
    let min_center = if dual_active {
        (total_w * 0.30).clamp(80.0, 184.0).min(total_w * 0.5)
    } else {
        (total_w * 0.35).clamp(80.0, 184.0).min(total_w * 0.5)
    };
    let budget = if peer_open && taken_by_peer <= 0.5 {
        // First of two active sidebars: reserve half of the non-center budget.
        (total_w - min_center).max(0.0) * 0.5
    } else {
        // Only active sidebar, or second sidebar subtracting space already taken.
        (total_w - min_center - taken_by_peer).max(0.0)
    };
    target.min(budget.min(avail)).max(0.0)
}

/// Builds a non-resizable side panel locked to `default_width` (clamped responsively).
pub(crate) fn fixed_side_panel_with_peer(
    ui: &egui::Ui,
    panel: egui::Panel,
    default_width: f32,
    peer_open: bool,
    frame: egui::Frame,
) -> egui::Panel {
    let width = responsive_side_panel_width(ui, default_width, peer_open);
    panel.resizable(false).exact_size(width).frame(frame)
}

/// Builds a non-resizable side panel locked to `default_width` (clamped to available space).
pub(crate) fn fixed_side_panel(
    ui: &egui::Ui,
    panel: egui::Panel,
    default_width: f32,
    frame: egui::Frame,
) -> egui::Panel {
    fixed_side_panel_with_peer(ui, panel, default_width, false, frame)
}

/// Helper used by tests to inspect default pen kind without importing engine internals.
pub fn default_pen_kind() -> PenKind {
    PenKind::FountainPen
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_view_state_splits_sidebars_and_hides_modals() {
        let view = ViewState::default();
        assert_eq!(view.screen, AppScreen::Home);
        assert!(view.pages_sidebar_open);
        assert!(!view.inspector_open);
        assert_eq!(view.active_modal, None);
        assert_eq!(view.settings_tab, SettingsTab::Pen);
        assert_eq!(default_pen_kind(), PenKind::FountainPen);
    }

    #[test]
    fn opening_notebook_from_home_and_returning_preserves_state() {
        let mut app = PaperCraftApp::default();
        assert_eq!(app.view.screen, AppScreen::Home);

        let _ = app.session.execute(
            "notebook.create",
            &json!({ "preset": "LectureCornell", "title": "Biology 101" }),
        );
        // Pretend we were back on Home with active notebook 0.
        let _ = app
            .session
            .execute("notebook.select", &json!({ "index": 0 }));
        app.view.screen = AppScreen::Home;

        open_notebook_in_editor(&mut app, 1);
        assert_eq!(app.view.screen, AppScreen::Editor);
        assert_eq!(app.session.active_notebook, 1);
        assert_eq!(app.session.doc.title, "Biology 101");
        assert!(app.view.fit_requested);
        assert_eq!(app.view.export_path_stem, "exports/Biology_101");

        return_to_home(&mut app);
        assert_eq!(app.view.screen, AppScreen::Home);
        assert_eq!(app.session.active_notebook, 1);
        assert_eq!(app.view.open_tabs.len(), 1);
    }

    #[test]
    fn opening_customization_modals_syncs_drafts_cleanly() {
        let mut app = PaperCraftApp::default();
        modal::open(&mut app, modal::ActiveModal::NotebookSettings);
        assert_eq!(
            app.view.active_modal,
            Some(modal::ActiveModal::NotebookSettings)
        );
        assert_eq!(app.view.notebook_title_draft, app.session.doc.title);
        assert_eq!(app.view.notebook_cover_draft, app.session.doc.cover);

        modal::open(&mut app, modal::ActiveModal::NewNotebook);
        modal::sync_new_notebook_from_preset(
            &mut app,
            papercraft_doc::NotebookPreset::MidnightSlate,
        );
        assert_eq!(
            app.view.new_notebook_preset,
            papercraft_doc::NotebookPreset::MidnightSlate
        );
        assert_eq!(
            app.view.new_notebook_color,
            papercraft_doc::PaperColor::DarkSlate
        );

        modal::close(&mut app);
        assert_eq!(app.view.active_modal, None);
    }

    #[test]
    fn export_path_helpers_normalize_stems_and_extensions() {
        assert_eq!(
            inspector::export_target_with_ext("exports/My_Notes.pdf", "png"),
            "exports/My_Notes.png"
        );
        assert_eq!(
            inspector::export_target_with_ext("   ", "pcraft"),
            "exports/Quick_Notes.pcraft"
        );
        assert_eq!(
            inspector::title_to_export_stem("Physics Lab - Term 1!"),
            "exports/Physics_Lab_Term_1"
        );
        assert_eq!(
            inspector::title_to_export_stem("   "),
            "exports/Quick_Notes"
        );
    }

    #[test]
    fn lateral_sidebars_keep_default_widths_and_ignore_border_drags() {
        let mut app = PaperCraftApp::default();
        open_notebook_in_editor(&mut app, 0);
        app.view.inspector_open = true;
        let ctx = egui::Context::default();
        let viewport = egui::vec2(1280.0, 800.0);

        let mut left_rect = egui::Rect::NOTHING;
        let mut right_rect = egui::Rect::NOTHING;
        let run_editor_frame = |ctx: &egui::Context,
                                app: &mut PaperCraftApp,
                                size: egui::Vec2,
                                time: f64,
                                events: Vec<egui::Event>| {
            let mut l_rect = egui::Rect::NOTHING;
            let mut r_rect = egui::Rect::NOTHING;
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                time: Some(time),
                events,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                toolbar::show(app, ui);
                l_rect = pages_sidebar::show_inner(app, ui).panel_rect;
                r_rect = inspector::show_inner(app, ui);
                canvas::show(app, ui);
            });
            let cursor = out.platform_output.cursor_icon;
            out.drop_without_applying_deltas();
            (l_rect, r_rect, cursor)
        };

        for idx in 0..2 {
            (left_rect, right_rect, _) =
                run_editor_frame(&ctx, &mut app, viewport, f64::from(idx) * 0.05, Vec::new());
        }
        assert!((left_rect.width() - pages_sidebar::DEFAULT_SIDEBAR_WIDTH).abs() <= 0.5);
        assert!((right_rect.width() - inspector::DEFAULT_INSPECTOR_WIDTH).abs() <= 0.5);

        // Attempt to hover and drag both inner panel boundaries outward and inward.
        let drag_attempts = [
            (left_rect.right(), left_rect.right() + 70.0),
            (left_rect.right(), left_rect.right() - 70.0),
            (right_rect.left(), right_rect.left() - 70.0),
            (right_rect.left(), right_rect.left() + 70.0),
        ];
        let mid_y = left_rect.center().y;
        let mut t = 0.2;
        for (start_x, end_x) in drag_attempts {
            let start = egui::pos2(start_x, mid_y);
            let mid = egui::pos2((start_x + end_x) * 0.5, mid_y);
            let end = egui::pos2(end_x, mid_y);
            let step_events = [
                vec![egui::Event::PointerMoved(start)],
                vec![
                    egui::Event::PointerMoved(start),
                    egui::Event::PointerButton {
                        pos: start,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
                vec![egui::Event::PointerMoved(mid)],
                vec![egui::Event::PointerMoved(end)],
                vec![
                    egui::Event::PointerMoved(end),
                    egui::Event::PointerButton {
                        pos: end,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            ];
            for events in step_events {
                t += 0.05;
                let (l_rect, r_rect, cursor) =
                    run_editor_frame(&ctx, &mut app, viewport, t, events);
                assert!(
                    (l_rect.width() - pages_sidebar::DEFAULT_SIDEBAR_WIDTH).abs() <= 0.5,
                    "left sidebar resized to {} during drag {start_x}->{end_x}",
                    l_rect.width()
                );
                assert!(
                    (r_rect.width() - inspector::DEFAULT_INSPECTOR_WIDTH).abs() <= 0.5,
                    "right sidebar resized to {} during drag {start_x}->{end_x}",
                    r_rect.width()
                );
                assert!(
                    !matches!(
                        cursor,
                        egui::CursorIcon::ResizeHorizontal
                            | egui::CursorIcon::ResizeWest
                            | egui::CursorIcon::ResizeEast
                    ),
                    "unexpected resize cursor {cursor:?} during border interaction {start_x}->{end_x}"
                );
            }
        }

        // Narrow viewport narrower than combined default widths clamps both sidebars
        // without panicking or leaving full 280px panels overflowing the screen.
        let narrow_ctx = egui::Context::default();
        let (l_narrow, r_narrow, _) = run_editor_frame(
            &narrow_ctx,
            &mut app,
            egui::vec2(320.0, 560.0),
            0.1,
            Vec::new(),
        );
        assert!(l_narrow.width() < pages_sidebar::DEFAULT_SIDEBAR_WIDTH);
        assert!(r_narrow.width() < inspector::DEFAULT_INSPECTOR_WIDTH);
        assert!(l_narrow.width() <= 320.0);
        assert!(r_narrow.width() <= 320.0);
    }

    #[test]
    fn home_pages_and_settings_sidebars_keep_expected_widths_and_adapt_responsively() {
        let mut app = PaperCraftApp::default();
        app.view.inspector_open = true;
        let ctx = egui::Context::default();

        // 1. On desktop Home (1280x800), left Library sidebar uses STANDARD_SIDEBAR_WIDTH (280px)
        //    while the right Settings drawer keeps its wider DEFAULT_INSPECTOR_WIDTH (372px).
        let mut home_left = egui::Rect::NOTHING;
        let mut home_right = egui::Rect::NOTHING;
        for f in 0..3 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(f as f64 * 0.08 + 0.1),
                predicted_dt: 0.08,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                let home_m = home::show_inner(&mut app, ui);
                home_left = home_m.sidebar_rect.expect("Home sidebar must be visible");
            });
            out.drop_without_applying_deltas();
        }
        // Verify right Settings drawer width on Home separately via direct panel render at 1280x800.
        let right_ctx = egui::Context::default();
        for f in 0..3 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(f as f64 * 0.08 + 0.1),
                predicted_dt: 0.08,
                ..Default::default()
            };
            let out = right_ctx.run_ui(raw, |ui| {
                theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                home_right = inspector::show_inner(&mut app, ui);
            });
            out.drop_without_applying_deltas();
        }
        assert!((home_left.width() - STANDARD_SIDEBAR_WIDTH).abs() <= 0.5);
        assert!((home_right.width() - inspector::DEFAULT_INSPECTOR_WIDTH).abs() <= 0.5);

        // 2. Opening a notebook in Editor keeps Library/Pages matched at 280px and Settings at 372px.
        open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = motion::ScreenTransition::new(AppScreen::Editor);
        app.view.pages_sidebar_open = true;
        app.view.inspector_open = true;

        let ed_ctx = egui::Context::default();
        let mut ed_left = egui::Rect::NOTHING;
        let mut ed_right = egui::Rect::NOTHING;
        for f in 0..4 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(f as f64 * 0.08 + 0.1),
                predicted_dt: 0.08,
                ..Default::default()
            };
            let out = ed_ctx.run_ui(raw, |ui| {
                theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                toolbar::show_inner(&mut app, ui);
                ed_left = pages_sidebar::show_inner(&mut app, ui).panel_rect;
                ed_right = inspector::show_inner(&mut app, ui);
                canvas::show(&mut app, ui);
            });
            out.drop_without_applying_deltas();
        }
        assert!((ed_left.width() - home_left.width()).abs() <= 0.5);
        assert!((ed_right.width() - home_right.width()).abs() <= 0.5);

        // 3. On a narrow 600px Editor viewport with both panels open, both shrink equally and keep >= 180px canvas.
        let narrow_ctx = egui::Context::default();
        let mut n_left = egui::Rect::NOTHING;
        let mut n_right = egui::Rect::NOTHING;
        for f in 0..4 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(600.0, 700.0),
                )),
                time: Some(f as f64 * 0.08 + 0.1),
                predicted_dt: 0.08,
                ..Default::default()
            };
            let out = narrow_ctx.run_ui(raw, |ui| {
                theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                toolbar::show_inner(&mut app, ui);
                n_left = pages_sidebar::show_inner(&mut app, ui).panel_rect;
                n_right = inspector::show_inner(&mut app, ui);
                canvas::show(&mut app, ui);
            });
            out.drop_without_applying_deltas();
        }
        assert!((n_left.width() - n_right.width()).abs() <= 0.5);
        assert!((n_left.width() - 210.0).abs() <= 0.5);
        let canvas_w = 600.0 - n_left.width() - n_right.width();
        assert!(
            canvas_w >= 179.0,
            "expected >= 180px canvas, got {canvas_w}"
        );

        // 4. On a compact viewport (< DUAL_SIDEBAR_MIN_VIEWPORT), keyboard toggle opens one drawer and closes the other.
        app.view.pages_sidebar_open = true;
        app.view.inspector_open = false;
        let compact_ctx = egui::Context::default();
        let raw_s = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(600.0, 700.0),
            )),
            events: vec![egui::Event::Key {
                key: egui::Key::S,
                physical_key: Some(egui::Key::S),
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        let out = compact_ctx.run_ui(raw_s, |ui| app.show_ui(ui));
        out.drop_without_applying_deltas();
        assert!(app.view.inspector_open);
        assert!(!app.view.pages_sidebar_open);
    }

    #[test]
    fn screen_switch_dissolves_smoothly_between_home_and_editor() {
        let mut app = PaperCraftApp::default();
        let ctx = egui::Context::default();
        let viewport = egui::vec2(1280.0, 800.0);
        let run_step = |ctx: &egui::Context, app: &mut PaperCraftApp, t: f64| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport)),
                time: Some(t),
                predicted_dt: 0.025,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| app.show_ui(ui));
            let repaints = !out.viewport_output.is_empty();
            out.drop_without_applying_deltas();
            repaints
        };

        // Initial Home frame settles cleanly with PaperCraft's standardized animation timing.
        run_step(&ctx, &mut app, 0.0);
        assert_eq!(app.view.motion.screen.rendered, AppScreen::Home);
        assert_eq!(app.view.motion.screen.phase, motion::ScreenPhase::Steady);
        let anim_time = ctx.style_of(ctx.theme()).animation_time;
        assert!((anim_time - motion::STANDARD_DURATION_S).abs() <= 1e-4);

        // Request switch to Editor -> target changes immediately while rendered dissolves out.
        open_notebook_in_editor(&mut app, 0);
        assert_eq!(app.view.screen, AppScreen::Editor);

        let mut t = 0.025;
        run_step(&ctx, &mut app, t);
        assert_eq!(app.view.motion.screen.rendered, AppScreen::Home);
        assert!(app.view.motion.screen.is_exiting());
        assert!(app.view.motion.screen.veil_alpha() > 0.0);

        for _ in 0..4 {
            t += 0.025;
            run_step(&ctx, &mut app, t);
        }
        assert_eq!(app.view.motion.screen.rendered, AppScreen::Editor);
        assert!(matches!(
            app.view.motion.screen.phase,
            motion::ScreenPhase::Entering { .. }
        ));

        for _ in 0..8 {
            t += 0.025;
            run_step(&ctx, &mut app, t);
        }
        assert_eq!(app.view.motion.screen.phase, motion::ScreenPhase::Steady);
        assert_eq!(app.view.motion.screen.veil_alpha(), 0.0);

        return_to_home(&mut app);
        t += 0.025;
        run_step(&ctx, &mut app, t);
        assert!(app.view.motion.screen.is_exiting());
        for _ in 0..12 {
            t += 0.025;
            run_step(&ctx, &mut app, t);
        }
        assert_eq!(app.view.motion.screen.rendered, AppScreen::Home);
        assert_eq!(app.view.motion.screen.phase, motion::ScreenPhase::Steady);
    }

    #[test]
    fn sidebars_collapse_and_expand_smoothly_across_intermediate_frames() {
        let mut app = PaperCraftApp::default();
        open_notebook_in_editor(&mut app, 0);
        app.view.inspector_open = true;
        let ctx = egui::Context::default();
        let viewport = egui::vec2(1280.0, 800.0);

        let run_panels = |ctx: &egui::Context, app: &mut PaperCraftApp, t: f64| {
            let mut l_rect = egui::Rect::NOTHING;
            let mut r_rect = egui::Rect::NOTHING;
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport)),
                time: Some(t),
                predicted_dt: 0.04,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
                toolbar::show(app, ui);
                l_rect = pages_sidebar::show_inner(app, ui).panel_rect;
                r_rect = inspector::show_inner(app, ui);
                canvas::show(app, ui);
            });
            out.drop_without_applying_deltas();
            (l_rect, r_rect)
        };

        let viewport_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, viewport);
        let vis_w = |r: egui::Rect| r.intersect(viewport_rect).width();

        let (l0, r0) = run_panels(&ctx, &mut app, 0.0);
        assert!(
            (vis_w(l0) - pages_sidebar::DEFAULT_SIDEBAR_WIDTH).abs() <= 0.5,
            "l0={l0:?} vis_w={}",
            vis_w(l0)
        );
        assert!(
            (vis_w(r0) - inspector::DEFAULT_INSPECTOR_WIDTH).abs() <= 0.5,
            "r0={r0:?} vis_w={} expected={}",
            vis_w(r0),
            inspector::DEFAULT_INSPECTOR_WIDTH
        );

        app.view.pages_sidebar_open = false;
        app.view.inspector_open = false;

        // First frames after closing show smooth intermediate slide rather than snapping to zero.
        let (l_mid, r_mid) = run_panels(&ctx, &mut app, 0.04);
        assert!(l_mid != egui::Rect::NOTHING);
        assert!(r_mid != egui::Rect::NOTHING);
        assert!(l_mid.left() < 0.0 && l_mid.right() > 0.0);
        assert!(r_mid.right() > viewport.x && r_mid.left() < viewport.x);
        assert!(vis_w(l_mid) > 0.0 && vis_w(l_mid) < pages_sidebar::DEFAULT_SIDEBAR_WIDTH);
        assert!(vis_w(r_mid) > 0.0 && vis_w(r_mid) < inspector::DEFAULT_INSPECTOR_WIDTH);

        let mut t = 0.04;
        for _ in 0..8 {
            t += 0.04;
            run_panels(&ctx, &mut app, t);
        }
        let (l_done, r_done) = run_panels(&ctx, &mut app, t + 0.04);
        assert_eq!(l_done, egui::Rect::NOTHING);
        assert_eq!(r_done, egui::Rect::NOTHING);

        // Opening again smoothly animates back through intermediate frames to full width.
        app.view.pages_sidebar_open = true;
        app.view.inspector_open = true;
        t += 0.08;
        let (l_open_mid, r_open_mid) = run_panels(&ctx, &mut app, t);
        assert!(
            vis_w(l_open_mid) > 0.0 && vis_w(l_open_mid) < pages_sidebar::DEFAULT_SIDEBAR_WIDTH
        );
        assert!(vis_w(r_open_mid) > 0.0 && vis_w(r_open_mid) < inspector::DEFAULT_INSPECTOR_WIDTH);

        for _ in 0..8 {
            t += 0.04;
            run_panels(&ctx, &mut app, t);
        }
        let (l_restored, r_restored) = run_panels(&ctx, &mut app, t + 0.04);
        assert!((vis_w(l_restored) - pages_sidebar::DEFAULT_SIDEBAR_WIDTH).abs() <= 0.5);
        assert!((vis_w(r_restored) - inspector::DEFAULT_INSPECTOR_WIDTH).abs() <= 0.5);
    }

    #[test]
    fn camera_fit_glide_and_page_turn_veil_run_smoothly_and_cancel_on_draw() {
        let mut app = PaperCraftApp::default();
        let _ = app.session.execute("page.add", &json!({}));
        let _ = app.session.execute("page.select", &json!({ "index": 0 }));
        open_notebook_in_editor(&mut app, 0);
        // Put screen directly in Editor steady state to test canvas camera & page transitions.
        app.view.motion.screen = motion::ScreenTransition::new(AppScreen::Editor);

        let ctx = egui::Context::default();
        let viewport = egui::vec2(1280.0, 800.0);
        let run_canvas =
            |ctx: &egui::Context, app: &mut PaperCraftApp, t: f64, events: Vec<egui::Event>| {
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport)),
                    time: Some(t),
                    predicted_dt: 0.04,
                    events,
                    ..Default::default()
                };
                let out = ctx.run_ui(raw, |ui| app.show_ui(ui));
                out.drop_without_applying_deltas();
            };

        run_canvas(&ctx, &mut app, 0.0, Vec::new());
        let fitted_zoom = app.view.zoom;

        app.view.zoom = fitted_zoom * 2.0;
        app.view.motion.fit_glide_requested = true;
        run_canvas(&ctx, &mut app, 0.04, Vec::new());
        assert!(app.view.motion.camera_glide.is_some());
        assert!(app.view.zoom < fitted_zoom * 2.0 && app.view.zoom > fitted_zoom);

        let mut t = 0.04;
        for _ in 0..6 {
            t += 0.04;
            run_canvas(&ctx, &mut app, t, Vec::new());
        }
        assert!(app.view.motion.camera_glide.is_none());
        assert!((app.view.zoom - fitted_zoom).abs() <= 1e-3);

        // Manual pointer press on canvas immediately cancels an in-flight camera glide.
        app.view.zoom = fitted_zoom * 2.0;
        app.view.motion.fit_glide_requested = true;
        t += 0.04;
        run_canvas(&ctx, &mut app, t, Vec::new());
        assert!(app.view.motion.camera_glide.is_some());
        let press_pos = egui::pos2(640.0, 400.0);
        t += 0.04;
        run_canvas(
            &ctx,
            &mut app,
            t,
            vec![egui::Event::PointerButton {
                pos: press_pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(app.view.motion.camera_glide.is_none());
        t += 0.04;
        run_canvas(
            &ctx,
            &mut app,
            t,
            vec![egui::Event::PointerButton {
                pos: press_pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );

        // Page turn triggers subtle non-displacing veil reveal and clears smoothly.
        let _ = app.session.execute("page.select", &json!({ "index": 1 }));
        t += 0.04;
        run_canvas(&ctx, &mut app, t, Vec::new());
        assert!(app.view.motion.page_fade.veil_alpha() > 0.0);
        for _ in 0..5 {
            t += 0.04;
            run_canvas(&ctx, &mut app, t, Vec::new());
        }
        assert_eq!(app.view.motion.page_fade.veil_alpha(), 0.0);
    }

    #[test]
    fn image_file_picker_clipboard_paste_selection_and_delete_undo_flow() {
        let png = papercraft_raster::png_encode(4, 3, &[180u8; 4 * 3 * 4]).unwrap();
        let hook_png = png.clone();
        let clip_png = png;

        let hooks = PlatformHooks {
            pick_image_file: Some(Box::new(move || {
                Some(PickedOpenFile::Bytes {
                    name: "picked.png".to_string(),
                    bytes: hook_png.clone(),
                })
            })),
            clipboard_image: Some(Box::new(move || {
                Some(ClipboardImagePayload::EncodedBytes(clip_png.clone()))
            })),
            ..PlatformHooks::default()
        };

        let mut app = PaperCraftApp::new(Session::default(), hooks);
        app.view.screen = AppScreen::Editor;
        app.view.motion.screen = motion::ScreenTransition::new(AppScreen::Editor);

        // File picker hook inserts image, activates Select, and highlights Image(0).
        assert!(trigger_open_image_dialog(&mut app));
        assert_eq!(app.session.doc.images.len(), 1);
        assert_eq!(app.session.active_tool, papercraft_engine::ToolKind::Select);
        assert_eq!(app.view.selected_object, Some(SelectedObject::Image(0)));

        // Clipboard hook inserts a second image.
        assert!(trigger_paste_image(&mut app, &[]));
        assert_eq!(app.session.doc.images.len(), 2);
        assert_eq!(app.view.selected_object, Some(SelectedObject::Image(1)));

        // Direct egui::Event::PasteImage works even when no platform clipboard hook is present.
        app.hooks.clipboard_image = None;
        let color_img = std::sync::Arc::new(egui::ColorImage::from_rgba_unmultiplied(
            [2, 2],
            &[
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ],
        ));
        assert!(trigger_paste_image(
            &mut app,
            &[egui::Event::PasteImage(color_img)],
        ));
        assert_eq!(app.session.doc.images.len(), 3);
        assert_eq!(app.view.selected_object, Some(SelectedObject::Image(2)));

        // Render full editor frame (toolbar, sidebar thumbnails, canvas) without panicking.
        let ctx = egui::Context::default();
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            time: Some(0.05),
            predicted_dt: 0.016,
            ..Default::default()
        };
        let out = ctx.run_ui(raw, |ui| app.show_ui(ui));
        out.drop_without_applying_deltas();

        // Delete removes selected image and Undo restores it.
        assert!(delete_selection(&mut app));
        assert_eq!(app.session.doc.images.len(), 2);
        assert!(app.session.execute("edit.undo", &json!({})).is_ok());
        assert_eq!(app.session.doc.images.len(), 3);
    }

    #[test]
    fn inserting_image_centers_at_active_camera_viewport_not_page_center() {
        let png = papercraft_raster::png_encode(120, 80, &[220u8; 120 * 80 * 4]).unwrap();
        let hook_png = png.clone();
        let clip_png = png.clone();

        let hooks = PlatformHooks {
            pick_image_file: Some(Box::new(move || {
                Some(PickedOpenFile::Bytes {
                    name: "picked.png".to_string(),
                    bytes: hook_png.clone(),
                })
            })),
            clipboard_image: Some(Box::new(move || {
                Some(ClipboardImagePayload::EncodedBytes(clip_png.clone()))
            })),
            ..PlatformHooks::default()
        };

        let mut app = PaperCraftApp::new(Session::default(), hooks);
        open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = motion::ScreenTransition::new(AppScreen::Editor);

        let ctx = egui::Context::default();
        let viewport = egui::vec2(1280.0, 800.0);
        for frame in 0..3 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport)),
                time: Some(f64::from(frame) * 0.08 + 0.08),
                predicted_dt: 0.08,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| app.show_ui(ui));
            out.drop_without_applying_deltas();
        }

        // Zoom in and pan so the camera center rests over the upper-left quadrant,
        // far away from the geometric page center (page_w * 0.5, page_h * 0.5).
        app.view.fit_requested = false;
        app.view.motion.camera_glide = None;
        app.view.zoom = 1.8;
        let canvas_rect = app.view.last_canvas_rect.expect("canvas rect recorded");
        let target_page = papercraft_geom::Point::new(220.0, 260.0);
        app.view.pan = [
            f64::from(canvas_rect.center().x) - target_page.x * 1.8,
            f64::from(canvas_rect.center().y) - target_page.y * 1.8,
        ];
        let expected_center = app.camera_center_page_point();
        assert!((expected_center.x - target_page.x).abs() < 1.0);
        assert!((expected_center.y - target_page.y).abs() < 1.0);

        let page_cx = f64::from(app.session.doc.width) * 0.5;
        let page_cy = f64::from(app.session.doc.height) * 0.5;

        // 1. File picker insertion centers at focused viewport.
        assert!(trigger_open_image_dialog(&mut app));
        let picked = &app.session.doc.images[0];
        let picked_center = picked.center();
        assert!(
            (picked_center.x - expected_center.x).abs() < 1.0
                && (picked_center.y - expected_center.y).abs() < 1.0,
            "expected picked image centered near camera {expected_center:?}, got {picked_center:?}"
        );
        assert!(
            (picked_center.x - page_cx).abs() > 100.0 && (picked_center.y - page_cy).abs() > 100.0,
            "picked image should not be placed at geometric page center ({page_cx}, {page_cy})"
        );

        // 2. Pan to lower-right quadrant and verify clipboard paste + dropped file follow focus.
        let target_page_2 = papercraft_geom::Point::new(980.0, 1340.0);
        app.view.zoom = 2.2;
        app.view.pan = [
            f64::from(canvas_rect.center().x) - target_page_2.x * 2.2,
            f64::from(canvas_rect.center().y) - target_page_2.y * 2.2,
        ];
        let expected_center_2 = app.camera_center_page_point();
        assert!(trigger_paste_image(&mut app, &[]));
        let pasted_center = app.session.doc.images[1].center();
        assert!(
            (pasted_center.x - expected_center_2.x).abs() < 1.0
                && (pasted_center.y - expected_center_2.y).abs() < 1.0,
            "expected pasted image centered near camera {expected_center_2:?}, got {pasted_center:?}"
        );

        // 3. Dropped image file follows active focus as well (on native synchronous drop path).
        #[cfg(not(target_arch = "wasm32"))]
        {
            #[derive(Debug)]
            struct TestDroppedFile {
                path: std::path::PathBuf,
                bytes: Vec<u8>,
            }
            impl egui::DroppedFile for TestDroppedFile {
                fn path(&self) -> &std::path::Path {
                    &self.path
                }
                fn bytes(&self) -> Result<Vec<u8>, String> {
                    Ok(self.bytes.clone())
                }
            }
            let dropped_file: egui::DroppedFileHandle = std::sync::Arc::new(TestDroppedFile {
                path: std::path::PathBuf::from("photo.png"),
                bytes: png,
            });
            let dropped_raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport)),
                time: Some(0.4),
                predicted_dt: 0.08,
                dropped_files: vec![dropped_file],
                ..Default::default()
            };
            let out = ctx.run_ui(dropped_raw, |ui| app.show_ui(ui));
            out.drop_without_applying_deltas();
            assert_eq!(app.session.doc.images.len(), 3);
            let dropped_center = app.session.doc.images[2].center();
            assert!(
                (dropped_center.x - expected_center_2.x).abs() < 1.0
                    && (dropped_center.y - expected_center_2.y).abs() < 1.0,
                "expected dropped image centered near camera {expected_center_2:?}, got {dropped_center:?}"
            );
        }
        #[cfg(target_arch = "wasm32")]
        let _ = png;
    }

    #[test]
    fn page_text_copy_paste_link_recognition_and_dock_link_flow() {
        let copied_slot = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let opened_slot = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let set_slot = std::sync::Arc::clone(&copied_slot);
        let get_slot = std::sync::Arc::clone(&copied_slot);
        let url_slot = std::sync::Arc::clone(&opened_slot);

        let hooks = PlatformHooks {
            clipboard_text: Some(Box::new(move || {
                let s = get_slot.lock().ok()?.clone();
                (!s.is_empty()).then_some(s)
            })),
            set_clipboard_text: Some(Box::new(move |text| {
                if let Ok(mut guard) = set_slot.lock() {
                    *guard = text.to_string();
                }
            })),
            open_url: Some(Box::new(move |url| {
                if let Ok(mut guard) = url_slot.lock() {
                    guard.push(url.to_string());
                }
            })),
            ..PlatformHooks::default()
        };

        let mut app = PaperCraftApp::new(Session::default(), hooks);
        app.view.screen = AppScreen::Editor;
        app.view.motion.screen = motion::ScreenTransition::new(AppScreen::Editor);

        // 1. Direct Event::Paste onto the page creates a TextItem and recognizes inline URLs.
        assert!(trigger_paste_on_page(
            &mut app,
            &[egui::Event::Paste(
                "Visit www.rust-lang.org today".to_string()
            )],
        ));
        assert_eq!(app.session.doc.texts.len(), 1);
        assert_eq!(app.view.selected_object, Some(SelectedObject::Text(0)));
        assert_eq!(
            app.session.doc.texts[0].primary_link_url().as_deref(),
            Some("https://www.rust-lang.org")
        );

        // 2. Attach an explicit link URL via text.update (as the floating text dock Link button does).
        app.session
            .execute(
                "text.update",
                &json!({ "index": 0, "text": "Rust Documentation", "link_url": "docs.rs/egui" }),
            )
            .unwrap();
        assert_eq!(
            app.session.doc.texts[0].link_url.as_deref(),
            Some("https://docs.rs/egui")
        );

        // 3. Copy selected text item (Ctrl+C) and paste via OS clipboard hook (Ctrl+V).
        let ctx = egui::Context::default();
        assert!(copy_selection(&mut app, Some(&ctx), false));
        assert_eq!(copied_slot.lock().unwrap().as_str(), "Rust Documentation");
        assert!(trigger_paste_on_page(&mut app, &[]));
        assert_eq!(app.session.doc.texts.len(), 2);
        assert_eq!(app.view.selected_object, Some(SelectedObject::Text(1)));
        assert_eq!(
            app.session.doc.texts[1].link_url.as_deref(),
            Some("https://docs.rs/egui")
        );
        assert!((app.session.doc.texts[1].x - app.session.doc.texts[0].x - 28.0).abs() < 1e-6);

        // 4. Open link dispatches sanitized URL to platform hook and renders full UI with link dock open.
        app.view.text_link_editor_open = true;
        app.view.text_link_input = "https://docs.rs/egui".to_string();
        assert!(open_external_url(&mut app, &ctx, "docs.rs/egui"));
        assert_eq!(
            opened_slot.lock().unwrap().as_slice(),
            &["https://docs.rs/egui".to_string()]
        );

        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            time: Some(0.05),
            predicted_dt: 0.016,
            ..Default::default()
        };
        let out = ctx.run_ui(raw, |ui| app.show_ui(ui));
        out.drop_without_applying_deltas();
    }

    fn minimal_two_page_ui_pdf() -> Vec<u8> {
        let objects = [
            b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".as_slice(),
            b"2 0 obj\n<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>\nendobj\n".as_slice(),
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 400] /Resources << >> >>\nendobj\n".as_slice(),
            b"4 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 400 200] /Resources << >> >>\nendobj\n".as_slice(),
        ];
        let mut pdf = b"%PDF-1.4\n".to_vec();
        let mut offsets = vec![0usize];
        for obj in objects {
            offsets.push(pdf.len());
            pdf.extend_from_slice(obj);
        }
        let xref = pdf.len();
        pdf.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for off in &offsets[1..] {
            pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );
        pdf
    }

    #[test]
    fn pdf_import_ui_workflow_renders_home_editor_sidebar_settings_and_errors() {
        let valid_pdf = minimal_two_page_ui_pdf();
        let valid_clone = valid_pdf.clone();
        let return_invalid = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let invalid_flag = std::sync::Arc::clone(&return_invalid);

        let hooks = PlatformHooks {
            pick_open_file: Some(Box::new(move |kind| {
                assert_eq!(kind, FileDialogKind::Pdf);
                if invalid_flag.load(std::sync::atomic::Ordering::Relaxed) {
                    Some(PickedOpenFile::Bytes {
                        name: "broken.pdf".to_string(),
                        bytes: b"not a valid pdf".to_vec(),
                    })
                } else {
                    Some(PickedOpenFile::Bytes {
                        name: "Syllabus.pdf".to_string(),
                        bytes: valid_clone.clone(),
                    })
                }
            })),
            ..PlatformHooks::default()
        };

        let mut app = PaperCraftApp::new(Session::default(), hooks);
        assert!(app.trigger_import_pdf_dialog(PdfImportTarget::NewNotebook));
        assert_eq!(app.view.screen, AppScreen::Editor);
        assert_eq!(app.session.doc.title, "Syllabus");
        assert_eq!(app.session.doc.page_count(), 2);
        assert!(app.session.doc.pdf_backdrop.is_some());
        assert_eq!(app.view.pdf_import_error, None);

        // Dark theme + auto-contrast must keep ink context un-inverted on white PDF sheets.
        app.session.prefs.theme = papercraft_doc::UiTheme::Dark;
        app.session.prefs.auto_contrast_with_theme = true;
        assert_eq!(
            effective_doc_ink_context(&app),
            (false, papercraft_doc::PaperColor::PureWhite)
        );

        let ctx = egui::Context::default();
        let run_frame = |ctx: &egui::Context, app: &mut PaperCraftApp, t: f64| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(t),
                predicted_dt: 0.016,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| app.show_ui(ui));
            out.drop_without_applying_deltas();
        };

        // Render Editor + Sidebar + Settings -> System tab + NotebookSettings modal without panicking.
        app.view.inspector_open = true;
        app.view.settings_tab = SettingsTab::System;
        run_frame(&ctx, &mut app, 0.05);
        modal::open(&mut app, modal::ActiveModal::NotebookSettings);
        run_frame(&ctx, &mut app, 0.10);
        modal::close(&mut app);

        // Return Home and render library cards with PDF backdrop showcase cleanly.
        return_to_home(&mut app);
        assert_eq!(app.view.screen, AppScreen::Home);
        run_frame(&ctx, &mut app, 0.25);

        // Insert PDF pages into an existing notebook via InsertIntoActive target.
        open_notebook_in_editor(&mut app, 0);
        assert!(app.trigger_import_pdf_dialog(PdfImportTarget::InsertIntoActive));
        assert_eq!(app.session.doc.page_count(), 3);

        // Invalid PDF populates dismissible UI error without crashing.
        return_invalid.store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(!app.trigger_import_pdf_dialog(PdfImportTarget::InsertIntoActive));
        assert!(app.view.pdf_import_error.is_some());
        run_frame(&ctx, &mut app, 0.45);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn async_pdf_import_and_background_render_keep_ui_responsive() {
        let valid_pdf = minimal_two_page_ui_pdf();
        let temp_path =
            std::env::temp_dir().join(format!("papercraft_async_ui_{}.pdf", std::process::id()));
        struct RemoveOnDrop(PathBuf);
        impl Drop for RemoveOnDrop {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        std::fs::write(&temp_path, &valid_pdf).unwrap();
        let _cleanup = RemoveOnDrop(temp_path.clone());

        let path_for_hook = temp_path.clone();
        let hooks = PlatformHooks {
            pick_open_file: Some(Box::new(move |kind| {
                assert_eq!(kind, FileDialogKind::Pdf);
                Some(PickedOpenFile::Path(path_for_hook.clone()))
            })),
            ..PlatformHooks::default()
        };

        let mut app = PaperCraftApp::new(Session::default(), hooks);
        let ctx = egui::Context::default();
        let run_frame = |ctx: &egui::Context, app: &mut PaperCraftApp, t: f64| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(t),
                predicted_dt: 0.016,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| app.show_ui(ui));
            out.drop_without_applying_deltas();
        };

        // Frame 0 initializes `last_ctx` so subsequent Path import runs off the UI thread.
        run_frame(&ctx, &mut app, 0.01);
        assert!(app.trigger_import_pdf_dialog(PdfImportTarget::NewNotebook));

        let start = std::time::Instant::now();
        let mut t = 0.05;
        while app.view.pdf_import_in_flight.is_some()
            && start.elapsed() < std::time::Duration::from_secs(5)
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
            run_frame(&ctx, &mut app, t);
            t += 0.02;
        }
        assert_eq!(app.view.pdf_import_in_flight, None);
        assert_eq!(app.view.pdf_import_error, None);
        assert_eq!(app.view.screen, AppScreen::Editor);
        assert_eq!(app.session.doc.page_count(), 2);

        // Pump a few frames so background PDF renderer finishes page 0 and thumbnails cleanly.
        for _ in 0..8 {
            std::thread::sleep(std::time::Duration::from_millis(15));
            run_frame(&ctx, &mut app, t);
            t += 0.02;
        }
    }

    #[test]
    fn app_scale_syncs_bidirectionally_with_ctrl_zoom_shortcuts_and_home_settings() {
        let mut app = PaperCraftApp::default();
        app.session
            .execute("theme.set", &json!({ "ui_scale": 1.25 }))
            .unwrap();
        let ctx = egui::Context::default();

        let run_step = |ctx: &egui::Context,
                        app: &mut PaperCraftApp,
                        t: f64,
                        events: Vec<egui::Event>|
         -> String {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(t),
                predicted_dt: 0.02,
                events,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| app.show_ui(ui));
            let mut texts = String::new();
            fn collect(shape: &egui::Shape, out: &mut String) {
                match shape {
                    egui::Shape::Text(text) => {
                        out.push_str(text.galley.text());
                        out.push('\n');
                    }
                    egui::Shape::Vec(items) => {
                        for item in items {
                            collect(item, out);
                        }
                    }
                    _ => {}
                }
            }
            for clipped in &out.shapes {
                collect(&clipped.shape, &mut texts);
            }
            out.drop_without_applying_deltas();
            texts
        };

        // Frame 1 applies persisted ui_scale (1.25) to egui context zoom factor.
        run_step(&ctx, &mut app, 0.02, Vec::new());
        run_step(&ctx, &mut app, 0.04, Vec::new());
        assert!((ctx.zoom_factor() - 1.25).abs() < 1e-4);

        // Pressing S on Home opens Settings sidebar on Appearance showing live scale (125%).
        run_step(
            &ctx,
            &mut app,
            0.06,
            vec![egui::Event::Key {
                key: egui::Key::S,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(app.view.inspector_open);
        assert_eq!(app.view.settings_tab, SettingsTab::Appearance);
        let home_texts = run_step(&ctx, &mut app, 0.25, Vec::new());
        assert!(home_texts.contains("App Scale"));
        assert!(home_texts.contains("125%"));

        // Ctrl + = increases zoom via egui's built-in shortcut and syncs back into prefs & sidebar UI.
        run_step(
            &ctx,
            &mut app,
            0.30,
            vec![egui::Event::Key {
                key: egui::Key::Equals,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
        );
        let zoomed_in_texts = run_step(&ctx, &mut app, 0.34, Vec::new());
        assert!((ctx.zoom_factor() - 1.4).abs() < 1e-4);
        assert!((app.session.prefs.ui_scale - 1.4).abs() < 1e-4);
        assert!(zoomed_in_texts.contains("140%"));

        // Ctrl + - decreases zoom and syncs back into prefs & sidebar UI.
        run_step(
            &ctx,
            &mut app,
            0.38,
            vec![egui::Event::Key {
                key: egui::Key::Minus,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
        );
        let zoomed_out_texts = run_step(&ctx, &mut app, 0.42, Vec::new());
        assert!((ctx.zoom_factor() - 1.3).abs() < 1e-4);
        assert!((app.session.prefs.ui_scale - 1.3).abs() < 1e-4);
        assert!(zoomed_out_texts.contains("130%"));

        // Ctrl + 0 resets zoom to 100%.
        run_step(
            &ctx,
            &mut app,
            0.46,
            vec![egui::Event::Key {
                key: egui::Key::Num0,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }],
        );
        let reset_texts = run_step(&ctx, &mut app, 0.50, Vec::new());
        assert!((ctx.zoom_factor() - 1.0).abs() < 1e-4);
        assert!((app.session.prefs.ui_scale - 1.0).abs() < 1e-4);
        assert!(reset_texts.contains("100%"));

        // Changing ui_scale via theme.set updates egui context zoom factor in Editor too.
        open_notebook_in_editor(&mut app, 0);
        app.view.motion.screen = motion::ScreenTransition::new(AppScreen::Editor);
        app.session
            .execute("theme.set", &json!({ "ui_scale": 1.5 }))
            .unwrap();
        run_step(&ctx, &mut app, 0.54, Vec::new());
        run_step(&ctx, &mut app, 0.56, Vec::new());
        assert!((ctx.zoom_factor() - 1.5).abs() < 1e-4);
    }

    #[test]
    fn ctrl_c_shortcut_on_selected_text_does_not_deadlock() {
        let copied_slot = Arc::new(std::sync::Mutex::new(String::new()));
        let set_slot = Arc::clone(&copied_slot);

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let hooks = PlatformHooks {
                set_clipboard_text: Some(Box::new(move |text| {
                    if let Ok(mut guard) = set_slot.lock() {
                        *guard = text.to_string();
                    }
                })),
                ..PlatformHooks::default()
            };

            let mut app = PaperCraftApp::new(Session::default(), hooks);
            app.session
                .execute(
                    "text.insert",
                    &json!({ "text": "Selected note", "x": 120.0, "y": 140.0 }),
                )
                .unwrap();
            app.view.screen = AppScreen::Editor;
            app.view.motion.screen = motion::ScreenTransition::new(AppScreen::Editor);
            app.view.selected_object = Some(SelectedObject::Text(0));

            let ctx = egui::Context::default();
            let raw_init = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(0.02),
                predicted_dt: 0.016,
                ..Default::default()
            };
            let out_init = ctx.run_ui(raw_init, |ui| app.show_ui(ui));
            out_init.drop_without_applying_deltas();

            let raw_copy = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                time: Some(0.05),
                predicted_dt: 0.016,
                events: vec![
                    egui::Event::ModifiersChanged(egui::Modifiers::COMMAND),
                    egui::Event::Key {
                        key: egui::Key::C,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers: egui::Modifiers::COMMAND,
                    },
                ],
                ..Default::default()
            };
            let out = ctx.run_ui(raw_copy, |ui| app.show_ui(ui));
            let has_copy_cmd =
                out.platform_output.commands.iter().any(
                    |cmd| matches!(cmd, egui::OutputCommand::CopyText(s) if s == "Selected note"),
                );
            out.drop_without_applying_deltas();
            let _ = tx.send(has_copy_cmd);
        });

        let has_copy_cmd = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("Ctrl+C shortcut deadlocked inside ContextImpl RwLock");
        assert!(has_copy_cmd);
        assert_eq!(copied_slot.lock().unwrap().as_str(), "Selected note");
    }

    #[test]
    fn ctrl_x_shortcut_on_selected_text_cuts_without_deadlocking() {
        let copied_slot = Arc::new(std::sync::Mutex::new(String::new()));
        let set_slot = Arc::clone(&copied_slot);

        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let hooks = PlatformHooks {
                set_clipboard_text: Some(Box::new(move |text| {
                    if let Ok(mut guard) = set_slot.lock() {
                        *guard = text.to_string();
                    }
                })),
                ..PlatformHooks::default()
            };

            let mut app = PaperCraftApp::new(Session::default(), hooks);
            app.session
                .execute(
                    "text.insert",
                    &json!({ "text": "Cut note", "x": 120.0, "y": 140.0 }),
                )
                .unwrap();
            app.view.screen = AppScreen::Editor;
            app.view.motion.screen = motion::ScreenTransition::new(AppScreen::Editor);
            app.view.selected_object = Some(SelectedObject::Text(0));

            let ctx = egui::Context::default();
            let out_init = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(0.02),
                    predicted_dt: 0.016,
                    ..Default::default()
                },
                |ui| app.show_ui(ui),
            );
            out_init.drop_without_applying_deltas();

            let out = ctx.run_ui(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    time: Some(0.05),
                    predicted_dt: 0.016,
                    events: vec![
                        egui::Event::ModifiersChanged(egui::Modifiers::COMMAND),
                        egui::Event::Key {
                            key: egui::Key::X,
                            physical_key: None,
                            pressed: true,
                            repeat: false,
                            modifiers: egui::Modifiers::COMMAND,
                        },
                    ],
                    ..Default::default()
                },
                |ui| app.show_ui(ui),
            );
            let has_copy_cmd = out
                .platform_output
                .commands
                .iter()
                .any(|cmd| matches!(cmd, egui::OutputCommand::CopyText(s) if s == "Cut note"));
            out.drop_without_applying_deltas();
            let _ = tx.send((has_copy_cmd, app.session.doc.texts.len()));
        });

        let (has_copy_cmd, remaining_texts) = rx
            .recv_timeout(std::time::Duration::from_secs(2))
            .expect("Ctrl+X shortcut deadlocked inside ContextImpl RwLock");
        assert!(has_copy_cmd);
        assert_eq!(remaining_texts, 0);
        assert_eq!(copied_slot.lock().unwrap().as_str(), "Cut note");
    }
}
