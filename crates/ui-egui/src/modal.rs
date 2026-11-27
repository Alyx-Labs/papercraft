//! Standard 3-part modal dialog architecture for PaperCraft.
//!
//! Every dialog uses the same structural contract:
//! 1. **Header**: semantic icon badge + title/subtitle + top-right aligned `X` close button.
//! 2. **Body**: adaptive content area that scrolls cleanly only when viewport space is tight.
//! 3. **Footer**: sticky bottom action bar (`Cancel` + primary CTA, optional left action) separated
//!    by a full-width divider so actions are always reachable and never occluded by scrollbars.

use egui::Color32;
use egui::emath::GuiRounding as _;

use crate::icons::{Icon, icon_button, paint_icon};
use crate::theme::Tokens;
use crate::{PaperCraftApp, customization_modals, export_modal, folder_modals};

/// Identifies which focused dialog is currently open over the workspace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActiveModal {
    Export,
    NewNotebook,
    NotebookSettings,
    ConfirmDeleteNotebook,
    ConfirmReorderPage,
    AccountSync,
    AddAccount,
    FirstAccessNickname,
    SelectLocalAccount,
    ConvertLocalAccount,
    TransferPages,
    NewFolder,
    RenameFolder,
    DeleteFolder,
    FolderColor,
    TagColor,
    AppUpdate,
    KeyboardShortcuts,
    WelcomeBanner,
    FirstNotebookDataNotice,
    InfiniteTheme,
}

struct DialogSpec {
    icon: Icon,
    title: &'static str,
    subtitle: &'static str,
    width: f32,
    /// Minimum body height locked across stepped flows so switching steps never resizes the modal.
    min_body_height: f32,
}

#[derive(Clone, Copy)]
struct ModalStepSpec {
    label: &'static str,
    icon: Icon,
}

const NOTEBOOK_SECTION_STEPS: &[ModalStepSpec] = &[
    ModalStepSpec {
        label: "Details & Cover",
        icon: Icon::Notebook,
    },
    ModalStepSpec {
        label: "Size & Orientation",
        icon: Icon::FitPage,
    },
    ModalStepSpec {
        label: "Paper & Templates",
        icon: Icon::Paper,
    },
];

impl ActiveModal {
    pub(crate) const fn id_tag(self) -> u8 {
        match self {
            Self::Export => 1,
            Self::NewNotebook => 2,
            Self::NotebookSettings => 3,
            Self::ConfirmDeleteNotebook => 4,
            Self::AccountSync => 5,
            Self::AddAccount => 21,
            Self::FirstAccessNickname => 14,
            Self::SelectLocalAccount => 15,
            Self::ConvertLocalAccount => 20,
            Self::TransferPages => 6,
            Self::NewFolder => 7,
            Self::RenameFolder => 8,
            Self::DeleteFolder => 9,
            Self::ConfirmReorderPage => 11,
            Self::FolderColor => 12,
            Self::TagColor => 13,
            Self::AppUpdate => 16,
            Self::KeyboardShortcuts => 17,
            Self::WelcomeBanner => 18,
            Self::FirstNotebookDataNotice => 19,
            Self::InfiniteTheme => 22,
        }
    }

    const fn spec(self) -> DialogSpec {
        match self {
            Self::Export => DialogSpec {
                icon: Icon::Export,
                title: "Export Notebook",
                subtitle: "Choose a file format to save a copy.",
                width: 460.0,
                min_body_height: 0.0,
            },
            Self::NewNotebook => DialogSpec {
                icon: Icon::Notebook,
                title: "New Notebook",
                subtitle: "Create your notebook step by step: details, cover, sheet size, and paper style.",
                width: 680.0,
                min_body_height: 524.0,
            },
            Self::NotebookSettings => DialogSpec {
                icon: Icon::Notebook,
                title: "Notebook Settings",
                subtitle: "Update notebook details, cover finish, page dimensions, and paper style.",
                width: 680.0,
                min_body_height: 524.0,
            },
            Self::ConfirmDeleteNotebook => DialogSpec {
                icon: Icon::Trash,
                title: "Move Notebook to Trash?",
                subtitle: "You can restore this notebook from Trash for 30 days.",
                width: 440.0,
                min_body_height: 0.0,
            },
            Self::ConfirmReorderPage => DialogSpec {
                icon: Icon::Sort,
                title: "Move Page Position?",
                subtitle: "Confirm changing this page's order in your notebook.",
                width: 440.0,
                min_body_height: 0.0,
            },
            Self::AccountSync => DialogSpec {
                icon: Icon::Account,
                title: "Account & Cloud Sync",
                subtitle: "Switch between device-local accounts and Offline-First cloud accounts.",
                width: 530.0,
                min_body_height: 0.0,
            },
            Self::AddAccount => DialogSpec {
                icon: Icon::Account,
                title: "Add Account",
                subtitle: "Create a local account on this device or sign in to a sync server.",
                width: 460.0,
                min_body_height: 0.0,
            },
            Self::TransferPages => DialogSpec {
                icon: Icon::Transfer,
                title: "Transfer or Merge Pages",
                subtitle: "Copy or move selected pages into another notebook, or merge notebooks.",
                width: 520.0,
                min_body_height: 0.0,
            },
            Self::NewFolder => DialogSpec {
                icon: Icon::FolderPlus,
                title: "New Folder",
                subtitle: "Create a folder to organize related notebooks.",
                width: 440.0,
                min_body_height: 0.0,
            },
            Self::RenameFolder => DialogSpec {
                icon: Icon::FolderPen,
                title: "Rename or Move Folder",
                subtitle: "Change this folder's name or move it to another location.",
                width: 440.0,
                min_body_height: 0.0,
            },
            Self::DeleteFolder => DialogSpec {
                icon: Icon::Trash,
                title: "Delete Folder?",
                subtitle: "Notebooks inside stay safe and move out of this folder.",
                width: 440.0,
                min_body_height: 0.0,
            },
            Self::FolderColor => DialogSpec {
                icon: Icon::Palette,
                title: "Folder Color",
                subtitle: "Choose a color to help spot this folder in your library.",
                width: 440.0,
                min_body_height: 0.0,
            },
            Self::TagColor => DialogSpec {
                icon: Icon::Palette,
                title: "Tag Color",
                subtitle: "Choose a color to help spot this tag in your library.",
                width: 440.0,
                min_body_height: 0.0,
            },
            Self::FirstAccessNickname => DialogSpec {
                icon: Icon::Account,
                title: "Welcome to PaperCraft",
                subtitle: "Enter a local nickname to identify this device session.",
                width: 420.0,
                min_body_height: 0.0,
            },
            Self::SelectLocalAccount => DialogSpec {
                icon: Icon::Account,
                title: "Choose an Account",
                subtitle: "Select a local or server account, create a new local workspace, or sign in to a server.",
                width: 820.0,
                min_body_height: 240.0,
            },
            Self::ConvertLocalAccount => DialogSpec {
                icon: Icon::Sync,
                title: "Sync Local Account to Server",
                subtitle: "Upload local notebooks and settings while keeping an Offline-First copy on this device.",
                width: 480.0,
                min_body_height: 220.0,
            },
            Self::AppUpdate => DialogSpec {
                icon: Icon::MoveDown,
                title: "Software Update",
                subtitle: "Review and install the latest PaperCraft release.",
                width: 480.0,
                min_body_height: 0.0,
            },
            Self::KeyboardShortcuts => DialogSpec {
                icon: Icon::Keyboard,
                title: "Keyboard Shortcuts",
                subtitle: "Quick keys and pointer gestures for tools, editing, and navigation.",
                width: 860.0,
                min_body_height: 0.0,
            },
            Self::WelcomeBanner => DialogSpec {
                icon: Icon::Notebook,
                title: "Study deeper with PaperCraft",
                subtitle: "Built for calm, distraction-free daily study:",
                width: 700.0,
                min_body_height: 0.0,
            },
            Self::FirstNotebookDataNotice => DialogSpec {
                icon: Icon::Lock,
                title: "Your notebooks stay private",
                subtitle: "How your data is saved and protected:",
                width: 700.0,
                min_body_height: 0.0,
            },
            Self::InfiniteTheme => DialogSpec {
                icon: Icon::Palette,
                title: "Color your studio with Infinite",
                subtitle: "How the hue & saturation matrix adapts your UI:",
                width: 700.0,
                min_body_height: 0.0,
            },
        }
    }
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, Debug)]
pub(crate) struct ModalMetrics {
    pub(crate) modal_rect: egui::Rect,
    pub(crate) close_btn_rect: egui::Rect,
    pub(crate) body_rect: egui::Rect,
    pub(crate) footer_rect: egui::Rect,
    pub(crate) inner_margin: f32,
}

/// Opens `target`, initializing its draft fields cleanly from active session state.
pub(crate) fn open(app: &mut PaperCraftApp, target: ActiveModal) {
    app.view.modal_open_seq = app.view.modal_open_seq.wrapping_add(1);
    crate::canvas::commit_pending_text(app);
    app.view.shape_insert_mode = false;
    app.view.shape_drag_start = None;
    app.view.shape_drag_end = None;
    app.view.shape_drag_touch_id = None;
    app.view.ruler_active = false;
    app.view.ruler_drag_mode = None;
    app.view.ruler_drag_touch_id = None;
    app.view.ruler_drag_anchor = None;
    app.view.text_insert_mode = false;
    app.view.text_style_open = false;
    if target != ActiveModal::ConfirmDeleteNotebook {
        app.view.pending_delete_notebook = None;
    }
    if target != ActiveModal::ConfirmReorderPage {
        app.view.pending_page_reorder = None;
    }
    app.view.tag_edit_index = None;
    app.view.tag_edit_draft.clear();
    app.view.tag_edit_focus_pending = false;
    match target {
        ActiveModal::Export => {
            if app.view.export_path_stem.trim().is_empty() {
                app.view.export_path_stem =
                    crate::inspector::title_to_export_stem(&app.session.doc.title);
            }
            if app.view.export_target_pages.is_none()
                && app.view.pages_select_mode
                && !app.view.selected_page_indices.is_empty()
            {
                app.view.export_target_pages = Some(app.view.selected_page_indices.clone());
            }
            app.view.export_page_scope = if app
                .view
                .export_target_pages
                .as_ref()
                .is_some_and(|v| !v.is_empty())
            {
                crate::ExportPageScope::SidebarSelection
            } else {
                crate::ExportPageScope::AllPages
            };
            app.view.export_page_range_draft.clear();
            app.view.export_feedback = None;
            app.view.pdf_export_progress = None;
        }
        ActiveModal::NewNotebook => {
            app.view.new_notebook_step = 0;
            sync_new_notebook_from_preset(app, app.view.new_notebook_preset);
            app.view.new_notebook_folder =
                app.view.library_folder_filter.clone().unwrap_or_default();
            app.view.new_notebook_favorite = false;
            app.view.new_notebook_tags.clear();
            // Notebooks started from Favorites or a tag view land back in that same view.
            match crate::library_folders::current_scope(app) {
                crate::library_folders::LibraryScope::Favorites => {
                    app.view.new_notebook_favorite = true;
                }
                crate::library_folders::LibraryScope::Tag(tag) => {
                    app.view.new_notebook_tags.push(tag);
                }
                _ => {}
            }
            app.view.new_notebook_tag_input.clear();
            app.view.new_notebook_custom_template_id = None;
            app.view.template_import_error = None;
            app.view.folder_picker_new_open = false;
            app.view.folder_picker_new_name.clear();
        }
        ActiveModal::NotebookSettings => {
            app.view.notebook_settings_step = 0;
            app.view.notebook_title_draft = app.session.doc.title.clone();
            app.view.notebook_cover_draft = app.session.doc.cover;
            app.view.notebook_folder_draft = app.session.doc.folder_path.clone();
            app.view.notebook_favorite_draft = app.session.doc.favorite;
            app.view.notebook_tags_draft = app.session.doc.tags.clone();
            app.view.notebook_tag_input_draft.clear();
            app.view.folder_picker_new_open = false;
            app.view.folder_picker_new_name.clear();
            app.view
                .cover_color_draft
                .sync_from_cover(app.session.doc.cover);
            app.view.page_setup_format =
                crate::page_format_picker::PageFormatDraft::from_dimensions(
                    app.session.doc.width,
                    app.session.doc.height,
                );
            app.view.page_setup_template = app.session.doc.paper_template;
            app.view.page_setup_color = app.session.doc.paper_color;
            app.view.page_setup_custom_template_id = app.session.doc.custom_template_id.clone();
            app.view.page_setup_clear_custom = false;
            app.view.page_setup_quarter_turns = 0;
            app.view.page_setup_apply_all = false;
            app.view.template_import_error = None;
        }
        ActiveModal::ConfirmDeleteNotebook
        | ActiveModal::ConfirmReorderPage
        | ActiveModal::NewFolder
        | ActiveModal::RenameFolder
        | ActiveModal::DeleteFolder
        | ActiveModal::FolderColor
        | ActiveModal::TagColor
        | ActiveModal::AppUpdate
        | ActiveModal::KeyboardShortcuts
        | ActiveModal::WelcomeBanner
        | ActiveModal::FirstNotebookDataNotice
        | ActiveModal::InfiniteTheme => {}
        ActiveModal::FirstAccessNickname => {
            app.view.account.select_local_new_name.clear();
            app.view.account.select_local_error = None;
            app.view.account.select_local_focus_pending = true;
            app.view.account.error_message = None;
            app.view.account.status_message = None;
        }
        ActiveModal::SelectLocalAccount => {
            app.view.account.server_url = app.session.preferred_server_url().to_string();
            app.view.account.invalidate_probe_if_url_changed();
            app.view.account.select_local_new_name.clear();
            app.view.account.select_local_error = None;
            app.view.account.select_local_focus_pending = !app.session.has_any_saved_accounts();
            app.view.account.clear_quick_login_draft();
            app.view.account.clear_pending_unsynced_switch();
            app.view.account.exit_password_reset_flow();
            app.view.account.selector_other_server_open = false;
            app.view.account.error_message = None;
            app.view.account.status_message = None;
            app.view.account.profile_probe_pending = true;
            app.view.motion.auth_flow.sync(0, 0);
            app.view.motion.account_banner.reset();
        }
        ActiveModal::ConvertLocalAccount => {
            app.view.account.server_url = app.session.preferred_server_url().to_string();
            app.view.account.invalidate_probe_if_url_changed();
            app.view.account.profile_probe_pending = true;
            app.view.account.convert_error = None;
            app.view.account.convert_busy = false;
            app.view.account.convert_focus_pending = true;
        }
        ActiveModal::AccountSync => {
            crate::account_modal::sync_draft_from_session(app);
            app.view.account.profile_probe_pending = true;
            let auth_key = if app.view.account.password_reset.active {
                2
            } else if app.view.account.tab == crate::account_modal::AuthTab::CreateAccount {
                1
            } else {
                0
            };
            app.view
                .motion
                .auth_flow
                .sync(auth_key, isize::from(auth_key));
            app.view.motion.account_banner.reset();
        }
        ActiveModal::AddAccount => {
            app.view.account.server_url = app.session.preferred_server_url().to_string();
            app.view.account.invalidate_probe_if_url_changed();
            app.view.account.select_local_new_name.clear();
            app.view.account.select_local_error = None;
            app.view.account.select_local_focus_pending =
                app.view.account.add_account_mode == crate::account_modal::AddAccountMode::Local;
            app.view.account.clear_auth_form_credentials();
            app.view.account.profile_probe_pending = true;
            app.view.motion.auth_flow.sync(0, 0);
            app.view.motion.account_banner.reset();
        }
        ActiveModal::TransferPages => {
            if app.view.transfer_pages_list.is_empty()
                && app.view.transfer_merge_whole_notebook.is_none()
            {
                app.view.transfer_pages_list = vec![app.session.doc.active_page_index()];
            }
            let cur = app.session.active_notebook;
            let alt = (0..app.session.notebooks.len())
                .find(|&idx| idx != cur)
                .unwrap_or(0);
            if app.view.transfer_target_notebook >= app.session.notebooks.len()
                || (app.view.transfer_target_notebook == cur && app.session.notebooks.len() > 1)
            {
                app.view.transfer_target_notebook = alt;
            }
            if app.session.notebooks.len() <= 1 && app.view.transfer_merge_whole_notebook.is_none()
            {
                app.view.transfer_create_new_notebook = true;
            }
            if app.view.transfer_new_title.trim().is_empty() {
                app.view.transfer_new_title = format!("{} (Extract)", app.session.doc.title);
            }
        }
    }
    let resolved_target = if target == ActiveModal::NewNotebook
        && app.session.should_show_first_notebook_data_notice()
    {
        ActiveModal::FirstNotebookDataNotice
    } else {
        target
    };
    app.view.active_modal = Some(resolved_target);
    let initial_step = active_step_index(app, resolved_target);
    app.view.motion.modal_flow.sync(
        (
            app.view.modal_open_seq,
            resolved_target.id_tag(),
            initial_step,
        ),
        initial_step as isize,
    );
    app.view.motion.modal_subflow.reset();
}

/// Applies preset defaults to the New Notebook modal drafts.
pub(crate) fn sync_new_notebook_from_preset(
    app: &mut PaperCraftApp,
    preset: papercraft_doc::NotebookPreset,
) {
    app.view.new_notebook_preset = preset;
    app.view.new_notebook_title = preset.default_title().to_string();
    app.view.new_notebook_cover = preset.cover();
    app.view.new_notebook_template = preset.template();
    app.view.new_notebook_color = preset.paper_color();
    app.view.new_notebook_format = crate::page_format_picker::PageFormatDraft::default_standard();
    app.view
        .cover_color_draft
        .sync_from_cover(app.view.new_notebook_cover);
}

/// Closes whichever modal dialog is currently active.
pub(crate) fn close(app: &mut PaperCraftApp) {
    app.view.active_modal = None;
    app.view.pending_delete_notebook = None;
    app.view.pending_page_reorder = None;
    app.view.folder_dialog_move_notebook = None;
    app.view.folder_dialog_focus_pending = false;
    app.view.folder_picker_new_open = false;
    app.view.folder_picker_new_name.clear();
    app.view.tag_edit_index = None;
    app.view.tag_edit_draft.clear();
    app.view.tag_edit_focus_pending = false;
    app.view.template_import_error = None;
    app.cancel_active_pdf_export_silently();
    app.view.export_target_pages = None;
    app.view.export_page_scope = crate::ExportPageScope::AllPages;
    app.view.export_page_range_draft.clear();
    app.view.export_feedback = None;
}

#[derive(Clone, Copy, Debug)]
struct ModalMotion {
    eased: f32,
    backdrop_eased: f32,
    interactive: bool,
    animate_flows: bool,
    flow_opacity: f32,
    flow_offset_x: f32,
    dt: f32,
}

impl ModalMotion {
    const STEADY: Self = Self {
        eased: 1.0,
        backdrop_eased: 1.0,
        interactive: true,
        animate_flows: false,
        flow_opacity: 1.0,
        flow_offset_x: 0.0,
        dt: 0.0,
    };
}

/// Renders the active modal dialog if one is open, or smoothly finishes its exit fade.
pub fn show(app: &mut PaperCraftApp, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let target = app.view.active_modal;
    let prev_rendered = app.view.motion.modal.rendered;
    let first_open_sizing = prev_rendered.is_none()
        && target.is_some_and(|next| {
            let modal_id = ui.make_persistent_id(("papercraft_modal_surface", next.id_tag()));
            egui::AreaState::load(&ctx, modal_id).is_none()
        });
    let dt = ctx.input(|i| i.stable_dt);
    let slot_animating = app.view.motion.modal.advance(target, dt, first_open_sizing);
    let any_visible = app.view.motion.modal.rendered.is_some() || target.is_some();
    let backdrop_animating =
        app.view
            .motion
            .modal_backdrop
            .advance(any_visible, dt, first_open_sizing);

    if app.view.motion.modal.rendered != prev_rendered {
        if let Some(next_active) = app.view.motion.modal.rendered {
            let step = active_step_index(app, next_active);
            app.view.motion.modal_flow.sync(
                (app.view.modal_open_seq, next_active.id_tag(), step),
                step as isize,
            );
            app.view.motion.modal_subflow.reset();
        } else {
            app.view.motion.modal_flow.reset();
            app.view.motion.modal_subflow.reset();
        }
    }

    let Some(active) = app.view.motion.modal.rendered else {
        if backdrop_animating {
            ctx.request_repaint();
        }
        return;
    };

    let flow_animating = if target == Some(active) {
        let step = active_step_index(app, active);
        app.view.motion.modal_flow.advance(
            (app.view.modal_open_seq, active.id_tag(), step),
            step as isize,
            dt,
        )
    } else {
        false
    };

    if slot_animating || backdrop_animating || flow_animating {
        ctx.request_repaint();
    }

    let motion = ModalMotion {
        eased: crate::motion::ease_out_cubic(app.view.motion.modal.progress),
        backdrop_eased: app.view.motion.modal_backdrop.eased(),
        interactive: target == Some(active),
        animate_flows: true,
        flow_opacity: app.view.motion.modal_flow.opacity(),
        flow_offset_x: app.view.motion.modal_flow.offset_x(),
        dt,
    };
    let _ = show_inner_with_motion(app, ui, active, motion);
}

#[cfg_attr(not(test), allow(dead_code))]
fn show_inner(app: &mut PaperCraftApp, ui: &mut egui::Ui, active: ActiveModal) -> ModalMetrics {
    show_inner_with_motion(app, ui, active, ModalMotion::STEADY)
}

fn show_inner_with_motion(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    active: ActiveModal,
    motion: ModalMotion,
) -> ModalMetrics {
    match active {
        ActiveModal::Export => show_dialog(
            app,
            ui,
            active,
            motion,
            export_modal::render_body,
            export_modal::render_footer,
        ),
        ActiveModal::NewNotebook => show_dialog(
            app,
            ui,
            active,
            motion,
            customization_modals::render_new_notebook_body,
            customization_modals::render_new_notebook_footer,
        ),
        ActiveModal::NotebookSettings => show_dialog(
            app,
            ui,
            active,
            motion,
            customization_modals::render_notebook_settings_body,
            customization_modals::render_notebook_settings_footer,
        ),
        ActiveModal::ConfirmDeleteNotebook => show_dialog(
            app,
            ui,
            active,
            motion,
            customization_modals::render_delete_notebook_confirmation_body,
            customization_modals::render_delete_notebook_confirmation_footer,
        ),
        ActiveModal::ConfirmReorderPage => show_dialog(
            app,
            ui,
            active,
            motion,
            customization_modals::render_reorder_page_confirmation_body,
            customization_modals::render_reorder_page_confirmation_footer,
        ),
        ActiveModal::AccountSync => show_dialog(
            app,
            ui,
            active,
            motion,
            crate::account_modal::render_body,
            crate::account_modal::render_footer,
        ),
        ActiveModal::AddAccount => show_dialog(
            app,
            ui,
            active,
            motion,
            crate::account_modal::render_add_account_body,
            crate::account_modal::render_add_account_footer,
        ),
        ActiveModal::FirstAccessNickname => show_dialog(
            app,
            ui,
            active,
            motion,
            crate::account_modal::render_first_access_body,
            crate::account_modal::render_first_access_footer,
        ),
        ActiveModal::SelectLocalAccount => show_dialog(
            app,
            ui,
            active,
            motion,
            crate::account_modal::render_select_local_account_body,
            crate::account_modal::render_select_local_account_footer,
        ),
        ActiveModal::ConvertLocalAccount => show_dialog(
            app,
            ui,
            active,
            motion,
            crate::account_modal::render_convert_local_account_body,
            crate::account_modal::render_convert_local_account_footer,
        ),
        ActiveModal::TransferPages => show_dialog(
            app,
            ui,
            active,
            motion,
            customization_modals::render_transfer_pages_body,
            customization_modals::render_transfer_pages_footer,
        ),
        ActiveModal::NewFolder => show_dialog(
            app,
            ui,
            active,
            motion,
            folder_modals::render_new_folder_body,
            folder_modals::render_new_folder_footer,
        ),
        ActiveModal::RenameFolder => show_dialog(
            app,
            ui,
            active,
            motion,
            folder_modals::render_rename_folder_body,
            folder_modals::render_rename_folder_footer,
        ),
        ActiveModal::DeleteFolder => show_dialog(
            app,
            ui,
            active,
            motion,
            folder_modals::render_delete_folder_body,
            folder_modals::render_delete_folder_footer,
        ),
        ActiveModal::FolderColor => show_dialog(
            app,
            ui,
            active,
            motion,
            folder_modals::render_folder_color_body,
            folder_modals::render_folder_color_footer,
        ),
        ActiveModal::TagColor => show_dialog(
            app,
            ui,
            active,
            motion,
            folder_modals::render_tag_color_body,
            folder_modals::render_tag_color_footer,
        ),
        ActiveModal::AppUpdate => show_dialog(
            app,
            ui,
            active,
            motion,
            render_app_update_body,
            render_app_update_footer,
        ),
        ActiveModal::KeyboardShortcuts => show_dialog(
            app,
            ui,
            active,
            motion,
            crate::shortcuts_modal::render_body,
            crate::shortcuts_modal::render_footer,
        ),
        ActiveModal::WelcomeBanner => crate::welcome_modal::show(
            app,
            ui,
            motion.eased,
            motion.backdrop_eased,
            motion.interactive,
        ),
        ActiveModal::FirstNotebookDataNotice => crate::data_privacy_modal::show(
            app,
            ui,
            motion.eased,
            motion.backdrop_eased,
            motion.interactive,
        ),
        ActiveModal::InfiniteTheme => crate::infinite_theme_modal::show(
            app,
            ui,
            motion.eased,
            motion.backdrop_eased,
            motion.interactive,
        ),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn render_app_update_body(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) -> bool {
    crate::update_modal::render_body(app, ui, t)
}

#[cfg(target_arch = "wasm32")]
fn render_app_update_body(_app: &mut PaperCraftApp, _ui: &mut egui::Ui, _t: &Tokens) -> bool {
    true
}

#[cfg(not(target_arch = "wasm32"))]
fn render_app_update_footer(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) -> bool {
    crate::update_modal::render_footer(app, ui, t)
}

#[cfg(target_arch = "wasm32")]
fn render_app_update_footer(_app: &mut PaperCraftApp, _ui: &mut egui::Ui, _t: &Tokens) -> bool {
    true
}

fn modal_steps(active: ActiveModal) -> Option<&'static [ModalStepSpec]> {
    match active {
        ActiveModal::NewNotebook | ActiveModal::NotebookSettings => Some(NOTEBOOK_SECTION_STEPS),
        ActiveModal::Export
        | ActiveModal::ConfirmDeleteNotebook
        | ActiveModal::ConfirmReorderPage
        | ActiveModal::AccountSync
        | ActiveModal::AddAccount
        | ActiveModal::FirstAccessNickname
        | ActiveModal::SelectLocalAccount
        | ActiveModal::ConvertLocalAccount
        | ActiveModal::TransferPages
        | ActiveModal::NewFolder
        | ActiveModal::RenameFolder
        | ActiveModal::DeleteFolder
        | ActiveModal::FolderColor
        | ActiveModal::TagColor
        | ActiveModal::AppUpdate
        | ActiveModal::KeyboardShortcuts
        | ActiveModal::WelcomeBanner
        | ActiveModal::FirstNotebookDataNotice
        | ActiveModal::InfiniteTheme => None,
    }
}

fn active_step_index(app: &PaperCraftApp, active: ActiveModal) -> usize {
    match active {
        ActiveModal::NewNotebook => app
            .view
            .new_notebook_step
            .min(NOTEBOOK_SECTION_STEPS.len().saturating_sub(1)),
        ActiveModal::NotebookSettings => app
            .view
            .notebook_settings_step
            .min(NOTEBOOK_SECTION_STEPS.len().saturating_sub(1)),
        _ => 0,
    }
}

fn set_active_step_index(app: &mut PaperCraftApp, active: ActiveModal, step: usize) {
    let clamped = step.min(NOTEBOOK_SECTION_STEPS.len().saturating_sub(1));
    match active {
        ActiveModal::NewNotebook => {
            app.view.new_notebook_step = clamped;
        }
        ActiveModal::NotebookSettings => {
            app.view.notebook_settings_step = clamped;
        }
        _ => {}
    }
}

pub(crate) fn is_account_picker_dismissible(app: &PaperCraftApp) -> bool {
    app.session.has_loaded_workspace()
        && !app.view.account.auth_busy
        && !app.view.account.password_reset.busy
}

fn show_dialog(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    active: ActiveModal,
    motion: ModalMotion,
    body: impl FnOnce(&mut PaperCraftApp, &mut egui::Ui, &Tokens) -> bool,
    footer: impl FnOnce(&mut PaperCraftApp, &mut egui::Ui, &Tokens) -> bool,
) -> ModalMetrics {
    let mut spec = active.spec();
    if active == ActiveModal::SelectLocalAccount {
        if app.view.account.selector_other_server_open {
            spec.width = 560.0;
            spec.title = "Sign In with a Server Account";
            spec.subtitle = "Sign in or create an account on your configured sync server.";
        } else if app.session.needs_initial_nickname() && !app.session.has_any_saved_accounts() {
            spec.title = "Welcome to PaperCraft";
            spec.subtitle = "Start immediately with a local offline account, or sign in to a self-hosted server.";
        } else if app.session.has_loaded_workspace() {
            spec.title = "Switch Account";
            spec.subtitle = "Switch between local and server accounts, create a local workspace, or sign in to a server.";
        }
    }
    let tag = active.id_tag();
    let steps = modal_steps(active);
    let current_step = active_step_index(app, active);
    let ctx = ui.ctx().clone();
    let t = Tokens::for_prefs(&app.session.prefs);
    let viewport = ctx.content_rect();
    let margin = 18_i8;
    let margin_f = f32::from(margin);
    let dialog_width = spec
        .width
        .min((viewport.width() - 32.0).max(320.0))
        .floor_ui();
    let inner_w = (dialog_width - margin_f * 2.0 - 2.0).max(260.0).floor_ui();
    let chrome_reserve = if steps.is_some() { 268.0 } else { 240.0 };
    let max_body_height = (viewport.height() - chrome_reserve).max(160.0).floor_ui();

    let backdrop_id = ui.make_persistent_id("papercraft_modal_backdrop");
    let modal_id = ui.make_persistent_id(("papercraft_modal_surface", tag));

    let backdrop_resp = egui::Area::new(backdrop_id)
        .order(egui::Order::Middle)
        .fixed_pos(viewport.min)
        .fade_in(false)
        .interactable(motion.interactive)
        .show(&ctx, |ui| {
            let (_rect, resp) = ui.allocate_exact_size(viewport.size(), egui::Sense::click());
            let backdrop_alpha = if t.is_dark { 145 } else { 92 };
            ui.painter().rect_filled(
                viewport,
                0.0,
                Color32::from_black_alpha(backdrop_alpha).gamma_multiply(motion.backdrop_eased),
            );
            resp
        })
        .inner;

    let mut should_close = false;
    let mut close_btn_rect = egui::Rect::NOTHING;
    let mut body_rect = egui::Rect::NOTHING;
    let mut footer_rect = egui::Rect::NOTHING;

    let shadow = ctx.style_of(ctx.theme()).visuals.window_shadow;
    let modal_frame = egui::Frame::NONE
        .fill(t.field_bg)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(12.0)
        .inner_margin(egui::Margin::same(margin))
        .shadow(shadow);

    let settle_offset = egui::vec2(
        0.0,
        ((1.0 - motion.eased) * crate::motion::MODAL_SETTLE_OFFSET_Y).round_ui(),
    );
    let allow_close = active != ActiveModal::FirstAccessNickname
        && !(active == ActiveModal::SelectLocalAccount && !is_account_picker_dismissible(app))
        && !(active == ActiveModal::ConvertLocalAccount && app.view.account.convert_busy)
        && !(active == ActiveModal::Export && app.is_pdf_export_active());

    let modal_resp = egui::Area::new(modal_id)
        .order(egui::Order::Foreground)
        .anchor(egui::Align2::CENTER_CENTER, settle_offset)
        .default_size(egui::vec2(
            dialog_width,
            (viewport.height() - 48.0).max(260.0).floor_ui(),
        ))
        .constrain(true)
        .fade_in(false)
        .interactable(motion.interactive)
        .show(&ctx, |ui| {
            ui.multiply_opacity(motion.eased);
            modal_frame.show(ui, |ui| {
                ui.set_min_width(inner_w);
                ui.set_max_width(inner_w);
                ui.spacing_mut().item_spacing.y = 0.0;

                let (close_clicked, x_rect) =
                    draw_dialog_header(ui, &spec, inner_w, allow_close, &t);
                close_btn_rect = x_rect;
                if allow_close && close_clicked {
                    should_close = true;
                }

                if let Some(step_list) = steps {
                    ui.add_space(10.0);
                    let picked = if active == ActiveModal::NotebookSettings {
                        draw_dialog_tabs(
                            ui,
                            app.view.modal_open_seq,
                            tag,
                            step_list,
                            current_step,
                            &t,
                        )
                    } else {
                        draw_dialog_stepper(ui, tag, step_list, current_step, inner_w, &t)
                    };
                    if let Some(picked_step) = picked {
                        set_active_step_index(app, active, picked_step);
                    }
                    ui.add_space(10.0);
                } else {
                    ui.add_space(12.0);
                }
                draw_modal_divider(ui, inner_w, &t);
                ui.add_space(12.0);

                let (body_closed, b_rect) = show_adaptive_body(
                    app,
                    ui,
                    tag,
                    current_step,
                    inner_w,
                    spec.min_body_height,
                    max_body_height,
                    motion,
                    &t,
                    body,
                );
                body_rect = b_rect;
                if body_closed {
                    should_close = true;
                }

                ui.add_space(12.0);
                draw_modal_divider(ui, inner_w, &t);
                ui.add_space(12.0);

                let footer_resp = ui.allocate_ui_with_layout(
                    egui::vec2(inner_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_min_width(inner_w);
                        ui.set_max_width(inner_w);
                        ui.spacing_mut().item_spacing = egui::vec2(8.0, 6.0);
                        if footer(app, ui, &t) {
                            should_close = true;
                        }
                    },
                );
                footer_rect = footer_resp.response.rect;
            });
        });

    let modal_rect = modal_resp.response.rect;
    let pointer_in_modal = backdrop_resp
        .interact_pointer_pos()
        .is_some_and(|pos| modal_rect.contains(pos));
    if allow_close && motion.interactive && backdrop_resp.clicked() && !pointer_in_modal {
        should_close = true;
    }

    if motion.interactive && should_close {
        close(app);
    }

    ModalMetrics {
        modal_rect,
        close_btn_rect,
        body_rect,
        footer_rect,
        inner_margin: margin_f + 1.0,
    }
}

fn draw_modal_divider(ui: &mut egui::Ui, inner_w: f32, t: &Tokens) {
    let (sep_rect, _) = ui.allocate_exact_size(egui::vec2(inner_w, 1.0), egui::Sense::hover());
    ui.painter().hline(
        sep_rect.x_range(),
        sep_rect.center().y,
        egui::Stroke::new(1.0, t.border),
    );
}

#[derive(Clone, Copy)]
struct BodyBox {
    width: f32,
    min_height: f32,
    max_height: f32,
}

fn show_body_content(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    bounds: BodyBox,
    motion: ModalMotion,
    t: &Tokens,
    body: impl FnOnce(&mut PaperCraftApp, &mut egui::Ui, &Tokens) -> bool,
) -> (bool, f32, egui::Rect) {
    let mut closed = false;
    let mut natural_h = 0.0_f32;
    let width = bounds.width.floor_ui();
    let min_height = bounds.min_height.round_ui();
    let max_height = bounds.max_height.floor_ui();
    let flow_opacity = motion.flow_opacity;
    let flow_offset_x = motion.flow_offset_x;
    let content = ui.allocate_ui_with_layout(
        egui::vec2(width, max_height),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_min_width(width);
            ui.set_max_width(width);
            ui.spacing_mut().item_spacing.y = 6.0;
            if flow_opacity >= 0.999 && flow_offset_x.abs() <= 0.05 {
                if body(app, ui, t) {
                    closed = true;
                }
                let min_rect = ui.min_rect();
                natural_h = min_rect.height();
                if min_height > natural_h {
                    ui.expand_to_include_rect(
                        egui::Rect::from_min_size(min_rect.min, egui::vec2(width, min_height))
                            .round_ui(),
                    );
                }
            } else {
                let clip_origin = ui.cursor().min;
                let clip = ui.clip_rect().intersect(
                    egui::Rect::from_min_size(clip_origin, egui::vec2(width, max_height + 32.0))
                        .round_ui(),
                );
                let child_max = egui::Rect::from_min_size(
                    clip_origin + egui::vec2(flow_offset_x.round_ui(), 0.0),
                    egui::vec2(width, max_height + 64.0),
                )
                .round_ui();
                let mut child = ui.new_child(
                    egui::UiBuilder::new()
                        .max_rect(child_max)
                        .layout(egui::Layout::top_down(egui::Align::Min)),
                );
                child.set_clip_rect(clip);
                child.set_width(width);
                child.spacing_mut().item_spacing.y = 6.0;
                child.multiply_opacity(flow_opacity);
                if body(app, &mut child, t) {
                    closed = true;
                }
                natural_h = child.min_rect().height().max(0.0).round_ui();
                let alloc_h = natural_h.max(min_height).round_ui();
                let _ = ui.allocate_exact_size(egui::vec2(width, alloc_h), egui::Sense::hover());
            }
        },
    );
    (closed, natural_h, content.response.rect)
}

#[allow(clippy::too_many_arguments)]
fn show_adaptive_body(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    tag: u8,
    step_idx: usize,
    inner_w: f32,
    base_min_body_height: f32,
    max_body_height: f32,
    motion: ModalMotion,
    t: &Tokens,
    body: impl FnOnce(&mut PaperCraftApp, &mut egui::Ui, &Tokens) -> bool,
) -> (bool, egui::Rect) {
    let height_cache_id = ui.make_persistent_id(("papercraft_modal_natural_h", tag, step_idx));
    let last_natural_h = ui
        .ctx()
        .data(|d| d.get_temp::<f32>(height_cache_id))
        .unwrap_or(0.0);
    let needs_scroll = last_natural_h > max_body_height + 2.0;

    let base_min = if base_min_body_height > 0.0 {
        base_min_body_height.min(max_body_height).round_ui()
    } else {
        0.0
    };
    let anim_h_id =
        ui.make_persistent_id(("papercraft_modal_anim_h", tag, app.view.modal_open_seq));
    let prev_anim_h = if motion.animate_flows {
        ui.ctx()
            .data(|d| d.get_temp::<f32>(anim_h_id))
            .filter(|h| h.is_finite() && *h > 1.0)
    } else {
        None
    };
    let effective_min_h = if motion.animate_flows {
        prev_anim_h
            .unwrap_or(base_min)
            .min(max_body_height)
            .round_ui()
    } else {
        base_min
    };

    let mut closed = false;
    let mut measured_natural_h = 0.0_f32;
    let outer_rect = if needs_scroll {
        ui.spacing_mut().scroll.floating = false;
        let scroll_resp = ui
            .allocate_ui_with_layout(
                egui::vec2(inner_w, max_body_height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_width(inner_w);
                    ui.set_max_width(inner_w);
                    egui::ScrollArea::vertical()
                        .id_salt(("papercraft_modal_scroll", tag, step_idx))
                        .min_scrolled_height(effective_min_h)
                        .max_height(max_body_height)
                        .auto_shrink([false, effective_min_h <= 0.0])
                        .show(ui, |ui| {
                            let inner_w_avail = ui.available_width().min(inner_w).floor_ui();
                            ui.set_max_width(inner_w_avail);
                            // Keep enough vertical headroom inside the scrolling body so nested
                            // auto-shrinking ScrollAreas use their full natural height rather than
                            // collapsing to `min_scrolled_size`.
                            let content_max_h =
                                last_natural_h.max(max_body_height + 240.0).round_ui();
                            let (body_closed, nat_h, _rect) = show_body_content(
                                app,
                                ui,
                                BodyBox {
                                    width: inner_w_avail,
                                    min_height: effective_min_h,
                                    max_height: content_max_h,
                                },
                                motion,
                                t,
                                body,
                            );
                            closed = body_closed;
                            measured_natural_h = nat_h;
                        })
                },
            )
            .inner;
        scroll_resp.inner_rect
    } else {
        let (body_closed, nat_h, rect) = show_body_content(
            app,
            ui,
            BodyBox {
                width: inner_w,
                min_height: effective_min_h,
                max_height: max_body_height,
            },
            motion,
            t,
            body,
        );
        closed = body_closed;
        measured_natural_h = nat_h;
        rect
    };

    ui.ctx()
        .data_mut(|d| d.insert_temp(height_cache_id, measured_natural_h));
    if motion.animate_flows {
        let target_h = measured_natural_h
            .max(base_min)
            .min(max_body_height)
            .round_ui();
        let next_h = match prev_anim_h {
            Some(prev) if motion.interactive && (target_h - prev).abs() > 0.5 => {
                let dt = crate::motion::sanitize_dt(motion.dt);
                ui.ctx().request_repaint();
                if dt > 0.0 {
                    let alpha = 1.0 - (-dt / 0.11).exp();
                    prev + (target_h - prev) * alpha
                } else {
                    prev
                }
            }
            _ => target_h,
        };
        ui.ctx().data_mut(|d| d.insert_temp(anim_h_id, next_h));
    }
    (closed, outer_rect)
}

/// Applies directional crossfade + subtle horizontal glide when switching sub-flows inside a modal.
pub(crate) fn apply_subflow_motion(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    modal: ActiveModal,
    subflow_id: u8,
    order: isize,
    add_contents: impl FnOnce(&mut PaperCraftApp, &mut egui::Ui),
) {
    let key = (app.view.modal_open_seq, modal.id_tag(), subflow_id);
    let dt = ui.ctx().input(|i| i.stable_dt);
    let steady =
        app.view.active_modal != Some(modal) && app.view.motion.modal.rendered != Some(modal);
    if steady {
        app.view.motion.modal_subflow.sync(key, order);
    } else if app.view.motion.modal_subflow.advance(key, order, dt) {
        ui.ctx().request_repaint();
    }

    let opacity = app.view.motion.modal_subflow.opacity();
    let offset_x = app.view.motion.modal_subflow.offset_x();
    crate::motion::show_flow_content(ui, opacity, offset_x, |ui| add_contents(app, ui));
}

fn draw_dialog_header(
    ui: &mut egui::Ui,
    spec: &DialogSpec,
    inner_w: f32,
    show_close: bool,
    t: &Tokens,
) -> (bool, egui::Rect) {
    let mut close_clicked = false;
    let mut close_rect = egui::Rect::NOTHING;
    let badge_w = 32.0;
    let close_btn_w = 28.0;
    let gap = 10.0;
    let text_w = (inner_w - badge_w - close_btn_w - gap * 2.0)
        .max(120.0)
        .floor_ui();

    ui.allocate_ui_with_layout(
        egui::vec2(inner_w, 0.0),
        egui::Layout::left_to_right(egui::Align::Min),
        |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(0.0, 0.0);
            let (badge_rect, _) =
                ui.allocate_exact_size(egui::vec2(badge_w, badge_w), egui::Sense::hover());
            ui.painter().rect_filled(badge_rect, 8.0, t.accent_soft);
            paint_icon(ui.painter(), badge_rect.shrink(6.0), spec.icon, t.accent);
            ui.add_space(gap);

            ui.allocate_ui_with_layout(
                egui::vec2(text_w, 0.0),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    ui.set_min_width(text_w);
                    ui.set_max_width(text_w);
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(
                        egui::RichText::new(spec.title)
                            .size(15.5)
                            .strong()
                            .color(t.text),
                    );
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(spec.subtitle)
                                .size(11.5)
                                .color(t.text_muted),
                        )
                        .wrap(),
                    );
                },
            );
            ui.add_space(gap);

            if show_close {
                let resp = icon_button(ui, Icon::Close, None, false, true, t)
                    .on_hover_text("Close dialog (Esc)");
                close_rect = resp.rect;
                if resp.clicked() {
                    close_clicked = true;
                }
            } else {
                let (_, spacer) =
                    ui.allocate_exact_size(egui::vec2(close_btn_w, 28.0), egui::Sense::hover());
                close_rect = spacer.rect;
            }
        },
    );

    (close_clicked, close_rect)
}

fn add_modal_action_button(
    ui: &mut egui::Ui,
    btn: egui::Button<'_>,
    enabled: bool,
) -> egui::Response {
    let resp = ui.add_enabled(enabled, btn);
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// Renders a prominent blue primary dialog CTA button.
pub(crate) fn primary_action_button(
    ui: &mut egui::Ui,
    label: &str,
    enabled: bool,
    t: &Tokens,
) -> egui::Response {
    let fill = if enabled { t.accent } else { t.border };
    let text_color = if enabled { t.on_accent } else { t.text_muted };
    let btn = egui::Button::new(
        egui::RichText::new(label)
            .size(12.0)
            .strong()
            .color(text_color),
    )
    .fill(fill)
    .stroke(egui::Stroke::NONE)
    .corner_radius(7.0)
    .min_size(egui::vec2(0.0, 30.0));
    add_modal_action_button(ui, btn, enabled)
}

/// Renders a clean secondary/outlined dialog action button.
pub(crate) fn secondary_action_button(
    ui: &mut egui::Ui,
    label: &str,
    enabled: bool,
    t: &Tokens,
) -> egui::Response {
    let text_color = if enabled { t.text } else { t.text_muted };
    let btn = egui::Button::new(egui::RichText::new(label).size(12.0).color(text_color))
        .fill(t.toolbar_bg)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(7.0)
        .min_size(egui::vec2(0.0, 30.0));
    add_modal_action_button(ui, btn, enabled)
}

/// Renders a subtle danger-accented secondary action button (e.g., closing a notebook).
pub(crate) fn danger_action_button(
    ui: &mut egui::Ui,
    label: &str,
    enabled: bool,
    t: &Tokens,
) -> egui::Response {
    let text_color = if enabled { t.danger_fg } else { t.text_muted };
    let stroke_color = if enabled { t.danger_border } else { t.border };
    let fill = if enabled { t.danger_bg } else { t.toolbar_bg };
    let btn = egui::Button::new(egui::RichText::new(label).size(12.0).color(text_color))
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, stroke_color))
        .corner_radius(7.0)
        .min_size(egui::vec2(0.0, 30.0));
    add_modal_action_button(ui, btn, enabled)
}

fn draw_dialog_tabs(
    ui: &mut egui::Ui,
    modal_seq: u64,
    tag: u8,
    tabs: &[ModalStepSpec],
    current_tab: usize,
    t: &Tokens,
) -> Option<usize> {
    if tabs.is_empty() {
        return None;
    }
    let options: Vec<crate::icons::SwitchOption<'_>> = tabs
        .iter()
        .map(|tab| {
            crate::icons::SwitchOption::new(Some(tab.icon), Some(tab.label), tab.label, true)
        })
        .collect();
    let (picked, _) = crate::icons::segmented_switch(
        ui,
        ("modal_section_tabs", tag, modal_seq),
        &options,
        Some(current_tab.min(options.len().saturating_sub(1))),
        crate::icons::SwitchLayout::EqualWidth,
        t,
    );
    picked
}

fn draw_dialog_stepper(
    ui: &mut egui::Ui,
    tag: u8,
    steps: &[ModalStepSpec],
    current_step: usize,
    inner_w: f32,
    t: &Tokens,
) -> Option<usize> {
    if steps.is_empty() {
        return None;
    }
    let mut picked = None;
    egui::Frame::NONE
        .fill(t.toolbar_bg)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(9.0)
        .inner_margin(egui::Margin::symmetric(5, 4))
        .show(ui, |ui| {
            let count = steps.len().max(1);
            let gap = 6.0_f32;
            let track_w = (inner_w - 10.0).max(160.0);
            let pill_w = ((track_w - gap * ((count - 1) as f32)) / (count as f32)).max(80.0);
            let pill_h = 30.0_f32;
            let (track_rect, _) =
                ui.allocate_exact_size(egui::vec2(track_w, pill_h), egui::Sense::hover());

            let mut step_rects = Vec::with_capacity(count);
            let mut responses = Vec::with_capacity(count);
            for (idx, step) in steps.iter().enumerate() {
                let x = track_rect.left() + (pill_w + gap) * idx as f32;
                let rect = egui::Rect::from_min_size(
                    egui::pos2(x, track_rect.top()),
                    egui::vec2(pill_w, pill_h),
                );
                let resp = ui
                    .interact(
                        rect,
                        ui.make_persistent_id(("modal_step_pill", tag, idx)),
                        egui::Sense::click(),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand)
                    .on_hover_text(format!("Step {} of {}: {}", idx + 1, count, step.label));
                if resp.clicked() {
                    picked = Some(idx);
                }
                step_rects.push(rect);
                responses.push(resp);
            }

            // Paint completed & hover pill backgrounds first.
            for (idx, rect) in step_rects.iter().enumerate() {
                let is_active = idx == current_step;
                let is_completed = idx < current_step;
                let hover_t = crate::motion::hover_factor(
                    ui.ctx(),
                    ui.make_persistent_id(("modal_step_hov", tag, idx)),
                    responses[idx].hovered() && !is_active,
                );
                let base_fill = if is_completed {
                    t.accent_soft.gamma_multiply(0.42)
                } else {
                    Color32::TRANSPARENT
                };
                let fill = crate::motion::lerp_color(base_fill, t.field_bg, hover_t * 0.72);
                let border_col = if is_completed {
                    t.accent.gamma_multiply(0.45)
                } else if hover_t > 0.05 {
                    t.border.gamma_multiply(hover_t)
                } else {
                    Color32::TRANSPARENT
                };
                if fill != Color32::TRANSPARENT || border_col != Color32::TRANSPARENT {
                    ui.painter().rect(
                        *rect,
                        egui::CornerRadius::same(7),
                        fill,
                        egui::Stroke::new(1.0, border_col),
                        egui::StrokeKind::Inside,
                    );
                }
            }

            // Smoothly sliding active step pill across the stepper track.
            if let Some(&target_r) = step_rects.get(current_step) {
                let pill_track_id = ui.make_persistent_id(("modal_stepper_track", tag));
                let rel_x0 = target_r.left() - track_rect.left();
                let rel_x1 = target_r.right() - track_rect.left();
                let anim_x0 = track_rect.left()
                    + ui.ctx().animate_value_with_time(
                        pill_track_id.with("x0"),
                        rel_x0,
                        crate::motion::MICRO_DURATION_S,
                    );
                let anim_x1 = track_rect.left()
                    + ui.ctx().animate_value_with_time(
                        pill_track_id.with("x1"),
                        rel_x1,
                        crate::motion::MICRO_DURATION_S,
                    );
                let sliding_rect = egui::Rect::from_min_max(
                    egui::pos2(anim_x0, target_r.top()),
                    egui::pos2(anim_x1, target_r.bottom()),
                );
                ui.painter().rect(
                    sliding_rect,
                    egui::CornerRadius::same(7),
                    t.field_bg,
                    egui::Stroke::new(1.2, t.accent),
                    egui::StrokeKind::Inside,
                );
            }

            // Paint circular step badges, icons, and labels crisp on top.
            for (idx, step) in steps.iter().enumerate() {
                let rect = step_rects[idx];
                let is_active = idx == current_step;
                let is_completed = idx < current_step;

                let badge_center = egui::pos2(rect.left() + 18.0, rect.center().y);
                let badge_r = 9.0_f32;
                let badge_rect = egui::Rect::from_center_size(
                    badge_center,
                    egui::vec2(badge_r * 2.0, badge_r * 2.0),
                );
                if is_active {
                    ui.painter().circle_filled(badge_center, badge_r, t.accent);
                    ui.painter().text(
                        badge_center,
                        egui::Align2::CENTER_CENTER,
                        (idx + 1).to_string(),
                        egui::FontId::proportional(10.5),
                        t.on_accent,
                    );
                } else if is_completed {
                    ui.painter()
                        .circle_filled(badge_center, badge_r, t.accent_soft);
                    paint_icon(ui.painter(), badge_rect.shrink(3.5), Icon::Check, t.accent);
                } else {
                    ui.painter().circle(
                        badge_center,
                        badge_r,
                        t.field_bg,
                        egui::Stroke::new(1.0, t.border),
                    );
                    ui.painter().text(
                        badge_center,
                        egui::Align2::CENTER_CENTER,
                        (idx + 1).to_string(),
                        egui::FontId::proportional(10.5),
                        t.text_muted,
                    );
                }

                let icon_rect = egui::Rect::from_center_size(
                    egui::pos2(rect.left() + 39.0, rect.center().y),
                    egui::vec2(14.0, 14.0),
                );
                let fg = if is_active {
                    t.text
                } else if is_completed {
                    t.accent
                } else {
                    t.text_muted
                };
                paint_icon(ui.painter(), icon_rect, step.icon, fg);
                ui.painter().text(
                    egui::pos2(icon_rect.right() + 6.0, rect.center().y),
                    egui::Align2::LEFT_CENTER,
                    step.label,
                    egui::FontId::proportional(12.0),
                    if is_active { t.text } else { fg },
                );
            }
        });
    picked
}

/// Renders an outlined or primary dialog action button with a small Lucide icon + text label.
pub(crate) fn step_action_button(
    ui: &mut egui::Ui,
    label: &str,
    icon: Icon,
    icon_after: bool,
    primary: bool,
    enabled: bool,
    t: &Tokens,
) -> egui::Response {
    let text_color = if !enabled {
        t.text_muted
    } else if primary {
        t.on_accent
    } else {
        t.text
    };
    let fill = if !enabled {
        if primary { t.border } else { t.toolbar_bg }
    } else if primary {
        t.accent
    } else {
        t.toolbar_bg
    };
    let stroke = if primary {
        egui::Stroke::NONE
    } else {
        egui::Stroke::new(1.0, t.border)
    };
    let pad_x = 12.0_f32;
    let icon_sz = 13.0_f32;
    let icon_gap = 5.0_f32;
    let galley = ui.painter().layout_no_wrap(
        label.to_string(),
        egui::FontId::proportional(12.0),
        text_color,
    );
    let width = ((pad_x * 2.0 + icon_sz + icon_gap + galley.size().x) / egui::emath::GUI_ROUNDING)
        .ceil()
        * egui::emath::GUI_ROUNDING;
    let (rect, resp) = ui.allocate_exact_size(
        egui::vec2(width.max(76.0), 30.0),
        if enabled {
            egui::Sense::click()
        } else {
            egui::Sense::hover()
        },
    );
    let resp = if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    };
    let hovered = enabled && resp.hovered();
    let final_fill = if hovered {
        if primary { t.accent_hover } else { t.field_bg }
    } else {
        fill
    };
    ui.painter().rect(
        rect,
        egui::CornerRadius::same(7),
        final_fill,
        stroke,
        egui::StrokeKind::Inside,
    );

    let total_inner_w = icon_sz + icon_gap + galley.size().x;
    let left_x = rect.center().x - total_inner_w * 0.5;
    let (icon_x, text_x) = if icon_after {
        (left_x + galley.size().x + icon_gap, left_x)
    } else {
        (left_x, left_x + icon_sz + icon_gap)
    };
    let icon_rect = egui::Rect::from_min_size(
        egui::pos2(icon_x, rect.center().y - icon_sz * 0.5),
        egui::vec2(icon_sz, icon_sz),
    );
    paint_icon(ui.painter(), icon_rect, icon, text_color);
    ui.painter().galley(
        egui::pos2(text_x, rect.center().y - galley.size().y * 0.5),
        galley,
        text_color,
    );
    resp
}

/// Outcome returned by [`render_stepped_footer`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct SteppedFooterOutcome {
    pub close_modal: bool,
    pub finish_clicked: bool,
}

/// Standardized stepped modal footer bar: `[left_slot] ... [Cancel] [Back] [Next | Finish]`.
pub(crate) fn render_stepped_footer(
    ui: &mut egui::Ui,
    step: &mut usize,
    total_steps: usize,
    finish_label: &str,
    finish_enabled: bool,
    t: &Tokens,
    left_slot: impl FnOnce(&mut egui::Ui, &Tokens) -> bool,
) -> SteppedFooterOutcome {
    let mut outcome = SteppedFooterOutcome::default();
    let last_step = total_steps.saturating_sub(1);
    *step = (*step).min(last_step);

    ui.horizontal(|ui| {
        if left_slot(ui, t) {
            outcome.close_modal = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if *step < last_step {
                if step_action_button(ui, "Next", Icon::NextPage, true, true, true, t).clicked() {
                    *step += 1;
                }
            } else if step_action_button(
                ui,
                finish_label,
                Icon::Check,
                false,
                true,
                finish_enabled,
                t,
            )
            .clicked()
            {
                outcome.finish_clicked = true;
            }

            if *step > 0
                && step_action_button(ui, "Back", Icon::PrevPage, false, false, true, t).clicked()
            {
                *step = step.saturating_sub(1);
            }

            if secondary_action_button(ui, "Cancel", true, t).clicked() {
                outcome.close_modal = true;
            }
        });
    });

    outcome
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn all_modals_render_centered_with_aligned_close_icon_and_pinned_footer() {
        let modals = [
            ActiveModal::Export,
            ActiveModal::NewNotebook,
            ActiveModal::NotebookSettings,
            ActiveModal::AccountSync,
            ActiveModal::AddAccount,
            ActiveModal::SelectLocalAccount,
            ActiveModal::ConvertLocalAccount,
            ActiveModal::TransferPages,
        ];

        for modal in modals {
            let ctx = egui::Context::default();
            let mut app = PaperCraftApp::default();
            open(&mut app, modal);

            let mut metrics = ModalMetrics {
                modal_rect: egui::Rect::NOTHING,
                close_btn_rect: egui::Rect::NOTHING,
                body_rect: egui::Rect::NOTHING,
                footer_rect: egui::Rect::NOTHING,
                inner_margin: 0.0,
            };
            for _ in 0..5 {
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    ..Default::default()
                };
                let out = ctx.run_ui(raw, |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    metrics = show_inner(&mut app, ui, modal);
                });
                out.drop_without_applying_deltas();
            }

            let expected_w = modal.spec().width;
            assert!(
                (metrics.modal_rect.width() - expected_w).abs() <= 4.0,
                "{modal:?} width {} differed from expected {expected_w}",
                metrics.modal_rect.width()
            );
            assert!(
                metrics.modal_rect.height() >= 300.0 && metrics.modal_rect.height() <= 740.0,
                "{modal:?} height {} was outside expected unclipped bounds",
                metrics.modal_rect.height()
            );
            assert!(
                (metrics.modal_rect.center().x - 640.0).abs() <= 2.0
                    && (metrics.modal_rect.center().y - 400.0).abs() <= 2.0,
                "{modal:?} center {:?} was not centered cleanly",
                metrics.modal_rect.center()
            );

            // Top-right X close icon sits flush at top-right inner corner.
            let inner_top = metrics.modal_rect.top() + metrics.inner_margin;
            let inner_right = metrics.modal_rect.right() - metrics.inner_margin;
            assert!(
                (metrics.close_btn_rect.top() - inner_top).abs() <= 2.5,
                "{modal:?} close icon top {} not aligned with header top {inner_top}",
                metrics.close_btn_rect.top()
            );
            assert!(
                (metrics.close_btn_rect.right() - inner_right).abs() <= 2.5,
                "{modal:?} close icon right {} not aligned with header right {inner_right}",
                metrics.close_btn_rect.right()
            );

            // Footer is strictly separated below body and stays inside the modal card.
            assert!(
                metrics.footer_rect.top() >= metrics.body_rect.bottom() + 20.0,
                "{modal:?} footer {:?} was not cleanly separated below body {:?}",
                metrics.footer_rect,
                metrics.body_rect
            );
            assert!(
                metrics.footer_rect.right() <= inner_right + 1.0,
                "{modal:?} footer overflowed right margin ({} > {inner_right})",
                metrics.footer_rect.right()
            );
        }

        // Expanded custom cover color studio inside NewNotebook also stays unclipped and centered.
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        open(&mut app, ActiveModal::NewNotebook);
        app.view.new_notebook_cover = papercraft_doc::NotebookCover::custom_rgb(0x2B, 0x7A, 0x78);
        app.view
            .cover_color_draft
            .sync_from_cover(app.view.new_notebook_cover);
        let mut custom_metrics = ModalMetrics {
            modal_rect: egui::Rect::NOTHING,
            close_btn_rect: egui::Rect::NOTHING,
            body_rect: egui::Rect::NOTHING,
            footer_rect: egui::Rect::NOTHING,
            inner_margin: 0.0,
        };
        for _ in 0..5 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                custom_metrics = show_inner(&mut app, ui, ActiveModal::NewNotebook);
            });
            out.drop_without_applying_deltas();
        }
        assert!(
            custom_metrics.modal_rect.height() <= 740.0,
            "NewNotebook with expanded custom cover studio height {} exceeded bounds",
            custom_metrics.modal_rect.height()
        );
        assert!(
            custom_metrics.footer_rect.top() >= custom_metrics.body_rect.bottom() + 20.0,
            "NewNotebook custom cover footer was not cleanly separated below body"
        );

        // Verify every step across NewNotebook (0..=2) and NotebookSettings (0..=2)
        // stays unclipped, centered, and fits cleanly inside a 1280×800 viewport.
        let stepped_cases = [
            (ActiveModal::NewNotebook, 3usize),
            (ActiveModal::NotebookSettings, 3usize),
        ];
        for (modal, total_steps) in stepped_cases {
            let mut step_heights = Vec::with_capacity(total_steps);
            for step in 0..total_steps {
                let ctx = egui::Context::default();
                let mut app = PaperCraftApp::default();
                let _ = app.session.execute("page.add", &serde_json::json!({}));
                open(&mut app, modal);
                set_active_step_index(&mut app, modal, step);
                app.view
                    .page_setup_format
                    .select_preset(papercraft_doc::PageResolutionPreset::Custom);
                app.view
                    .new_notebook_format
                    .select_preset(papercraft_doc::PageResolutionPreset::Custom);

                let mut step_metrics = ModalMetrics {
                    modal_rect: egui::Rect::NOTHING,
                    close_btn_rect: egui::Rect::NOTHING,
                    body_rect: egui::Rect::NOTHING,
                    footer_rect: egui::Rect::NOTHING,
                    inner_margin: 0.0,
                };
                for _ in 0..5 {
                    let raw = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1280.0, 800.0),
                        )),
                        ..Default::default()
                    };
                    let out = ctx.run_ui(raw, |ui| {
                        crate::theme::apply_light_visuals(ui.ctx());
                        step_metrics = show_inner(&mut app, ui, modal);
                    });
                    out.drop_without_applying_deltas();
                }
                assert!(
                    step_metrics.modal_rect.height() >= 300.0
                        && step_metrics.modal_rect.height() <= 740.0,
                    "{modal:?} step {step} height {} out of bounds",
                    step_metrics.modal_rect.height()
                );
                assert!(
                    step_metrics.footer_rect.top() >= step_metrics.body_rect.bottom() + 20.0,
                    "{modal:?} step {step} footer separated check failed"
                );
                step_heights.push(step_metrics.modal_rect.height());
            }
            assert!(
                step_heights.windows(2).all(|w| (w[0] - w[1]).abs() <= 1.0),
                "{modal:?} modal height changed across steps: {step_heights:?}"
            );
        }
    }

    #[test]
    fn stepped_modals_keep_identical_height_when_navigating_across_steps() {
        for (modal, total_steps) in [
            (ActiveModal::NewNotebook, 3usize),
            (ActiveModal::NotebookSettings, 3usize),
        ] {
            let ctx = egui::Context::default();
            let mut app = PaperCraftApp::default();
            let _ = app.session.execute("page.add", &serde_json::json!({}));
            open(&mut app, modal);

            let sequence: Vec<usize> = (0..total_steps)
                .chain((0..total_steps.saturating_sub(1)).rev())
                .collect();
            let mut observed_heights = Vec::with_capacity(sequence.len());
            for step in sequence {
                set_active_step_index(&mut app, modal, step);
                let mut metrics = ModalMetrics {
                    modal_rect: egui::Rect::NOTHING,
                    close_btn_rect: egui::Rect::NOTHING,
                    body_rect: egui::Rect::NOTHING,
                    footer_rect: egui::Rect::NOTHING,
                    inner_margin: 0.0,
                };
                for _ in 0..3 {
                    let raw = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(1280.0, 800.0),
                        )),
                        ..Default::default()
                    };
                    let out = ctx.run_ui(raw, |ui| {
                        crate::theme::apply_light_visuals(ui.ctx());
                        metrics = show_inner(&mut app, ui, modal);
                    });
                    out.drop_without_applying_deltas();
                }
                observed_heights.push(metrics.modal_rect.height());
            }
            assert!(
                observed_heights
                    .windows(2)
                    .all(|w| (w[0] - w[1]).abs() <= 1.0),
                "{modal:?} modal resized during step navigation: {observed_heights:?}"
            );
        }
    }

    #[test]
    fn modal_opens_with_coordinated_fade_and_closes_non_interactively() {
        let mut app = PaperCraftApp::default();
        let ctx = egui::Context::default();
        let viewport = egui::vec2(1280.0, 800.0);
        let run_modal = |ctx: &egui::Context, app: &mut PaperCraftApp, t: f64| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport)),
                time: Some(t),
                predicted_dt: 0.04,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                show(app, ui);
            });
            out.drop_without_applying_deltas();
        };

        open(&mut app, ActiveModal::NewNotebook);
        // First open performs the 1-frame sizing pass at progress 0.0 so modal and backdrop enter together.
        run_modal(&ctx, &mut app, 0.0);
        assert_eq!(
            app.view.motion.modal.rendered,
            Some(ActiveModal::NewNotebook)
        );
        assert_eq!(app.view.motion.modal.progress, 0.0);

        // Intermediate frame advances fade-in smoothly.
        run_modal(&ctx, &mut app, 0.04);
        assert!(app.view.motion.modal.progress > 0.0 && app.view.motion.modal.progress < 1.0);

        let mut t = 0.04;
        for _ in 0..6 {
            t += 0.04;
            run_modal(&ctx, &mut app, t);
        }
        assert_eq!(app.view.motion.modal.progress, 1.0);

        // Closing clears active_modal immediately while rendered dialog fades out softly.
        close(&mut app);
        assert_eq!(app.view.active_modal, None);
        t += 0.04;
        run_modal(&ctx, &mut app, t);
        assert_eq!(
            app.view.motion.modal.rendered,
            Some(ActiveModal::NewNotebook)
        );
        assert!(app.view.motion.modal.progress > 0.0 && app.view.motion.modal.progress < 1.0);

        for _ in 0..5 {
            t += 0.04;
            run_modal(&ctx, &mut app, t);
        }
        assert_eq!(app.view.motion.modal.rendered, None);
        assert_eq!(app.view.motion.modal.progress, 0.0);
    }

    #[test]
    fn modal_swap_preserves_backdrop_and_glides_between_steps() {
        let mut app = PaperCraftApp::default();
        let ctx = egui::Context::default();
        let viewport = egui::vec2(1280.0, 800.0);
        let run_modal = |ctx: &egui::Context, app: &mut PaperCraftApp, t: f64| {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport)),
                time: Some(t),
                predicted_dt: 0.04,
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                show(app, ui);
            });
            out.drop_without_applying_deltas();
        };

        open(&mut app, ActiveModal::NotebookSettings);
        run_modal(&ctx, &mut app, 0.0);
        let mut t = 0.0;
        for _ in 0..6 {
            t += 0.04;
            run_modal(&ctx, &mut app, t);
        }
        assert_eq!(app.view.motion.modal.progress, 1.0);
        assert_eq!(app.view.motion.modal_backdrop.progress, 1.0);
        assert_eq!(app.view.motion.modal_flow.offset_x(), 0.0);

        // Switching forward 0 -> 1 initiates a right-to-left horizontal glide that settles to 0.
        app.view.notebook_settings_step = 1;
        t += 0.04;
        run_modal(&ctx, &mut app, t);
        assert!(app.view.motion.modal_flow.offset_x() > 0.0);
        for _ in 0..5 {
            t += 0.04;
            run_modal(&ctx, &mut app, t);
        }
        assert_eq!(app.view.motion.modal_flow.offset_x(), 0.0);

        // Switching backward 1 -> 0 glides left-to-right smoothly.
        app.view.notebook_settings_step = 0;
        t += 0.04;
        run_modal(&ctx, &mut app, t);
        assert!(app.view.motion.modal_flow.offset_x() < 0.0);
        for _ in 0..5 {
            t += 0.04;
            run_modal(&ctx, &mut app, t);
        }
        assert_eq!(app.view.motion.modal_flow.offset_x(), 0.0);

        // Swapping directly to ConfirmDeleteNotebook keeps the backdrop steadily dimmed at 1.0.
        app.view.pending_delete_notebook = Some(std::sync::Arc::clone(&app.session.doc));
        open(&mut app, ActiveModal::ConfirmDeleteNotebook);
        for _ in 0..10 {
            t += 0.04;
            run_modal(&ctx, &mut app, t);
            assert_eq!(app.view.motion.modal_backdrop.progress, 1.0);
        }
        assert_eq!(
            app.view.motion.modal.rendered,
            Some(ActiveModal::ConfirmDeleteNotebook)
        );
        assert_eq!(app.view.motion.modal.progress, 1.0);
    }

    #[test]
    fn folder_modals_render_centered_with_pinned_footer() {
        let mut app = PaperCraftApp::default();
        app.session
            .execute(
                "notebook.create",
                &serde_json::json!({ "title": "Calc", "folder_path": "School/Math", "tags": ["exam"] }),
            )
            .unwrap();

        let cases = [
            ActiveModal::NewFolder,
            ActiveModal::RenameFolder,
            ActiveModal::DeleteFolder,
            ActiveModal::FolderColor,
            ActiveModal::TagColor,
        ];
        for target in cases {
            match target {
                ActiveModal::NewFolder => folder_modals::open_new_folder(&mut app, "School", None),
                ActiveModal::RenameFolder => {
                    folder_modals::open_rename_folder(&mut app, "School/Math")
                }
                ActiveModal::DeleteFolder => folder_modals::open_delete_folder(&mut app, "School"),
                ActiveModal::FolderColor => {
                    folder_modals::open_folder_color(&mut app, "School/Math")
                }
                ActiveModal::TagColor => folder_modals::open_tag_color(&mut app, "exam"),
                _ => {}
            }
            let ctx = egui::Context::default();
            let mut metrics = ModalMetrics {
                modal_rect: egui::Rect::NOTHING,
                close_btn_rect: egui::Rect::NOTHING,
                body_rect: egui::Rect::NOTHING,
                footer_rect: egui::Rect::NOTHING,
                inner_margin: 0.0,
            };
            for _ in 0..4 {
                let raw = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1280.0, 800.0),
                    )),
                    ..Default::default()
                };
                let out = ctx.run_ui(raw, |ui| {
                    crate::theme::apply_light_visuals(ui.ctx());
                    metrics = show_inner(&mut app, ui, target);
                });
                out.drop_without_applying_deltas();
            }
            assert!(
                (metrics.modal_rect.width() - target.spec().width).abs() <= 4.0,
                "{target:?} width mismatch"
            );
            assert!(
                (metrics.modal_rect.center().x - 640.0).abs() <= 2.0
                    && (metrics.modal_rect.center().y - 400.0).abs() <= 2.0,
                "{target:?} was not centered cleanly: {:?}",
                metrics.modal_rect.center()
            );
            assert!(
                metrics.footer_rect.top() >= metrics.body_rect.bottom() + 20.0,
                "{target:?} footer was not cleanly separated below body"
            );
        }
    }

    fn find_unclipped_text_rects(
        shapes: &[egui::epaint::ClippedShape],
        label: &str,
    ) -> Vec<(egui::Rect, egui::Rect)> {
        fn visit(
            shape: &egui::epaint::Shape,
            clip: egui::Rect,
            label: &str,
            out: &mut Vec<(egui::Rect, egui::Rect)>,
        ) {
            match shape {
                egui::epaint::Shape::Text(text) if text.galley.text() == label => {
                    out.push((text.visual_bounding_rect(), clip));
                }
                egui::epaint::Shape::Vec(items) => {
                    for item in items {
                        visit(item, clip, label, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in shapes {
            visit(&clipped.shape, clipped.clip_rect, label, &mut out);
        }
        out
    }

    #[test]
    fn folder_location_list_expands_unclipped_instead_of_shrinking() {
        let mut app = PaperCraftApp::default();
        app.session
            .execute(
                "library.folder.create",
                &serde_json::json!({ "path": "test" }),
            )
            .unwrap();
        app.session
            .execute(
                "library.folder.create",
                &serde_json::json!({ "path": "test/mk" }),
            )
            .unwrap();

        folder_modals::open_new_folder(&mut app, "test", None);
        let ctx = egui::Context::default();
        let mut small_metrics = ModalMetrics {
            modal_rect: egui::Rect::NOTHING,
            close_btn_rect: egui::Rect::NOTHING,
            body_rect: egui::Rect::NOTHING,
            footer_rect: egui::Rect::NOTHING,
            inner_margin: 0.0,
        };
        let mut last_shapes = Vec::new();
        for _ in 0..4 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                small_metrics = show_inner(&mut app, ui, ActiveModal::NewFolder);
            });
            last_shapes = out.shapes.clone();
            out.drop_without_applying_deltas();
        }

        // All 3 rows ("Library (top level)", "test", "mk") must fit without vertical clipping.
        for label in ["Library (top level)", "test", "mk"] {
            let rects = find_unclipped_text_rects(&last_shapes, label);
            assert!(
                !rects.is_empty(),
                "expected location row '{label}' to be rendered"
            );
            for (text_rect, clip_rect) in rects {
                assert!(
                    text_rect.top() >= clip_rect.top() - 0.5
                        && text_rect.bottom() <= clip_rect.bottom() + 0.5,
                    "location row '{label}' text {text_rect:?} was clipped by {clip_rect:?}"
                );
            }
        }

        // Adding many more folders expands the Location list up to 200px even on the same Context
        // (whose AreaState cached the smaller height).
        for idx in 1..=10 {
            app.session
                .execute(
                    "library.folder.create",
                    &serde_json::json!({ "path": format!("Extra{idx:02}") }),
                )
                .unwrap();
        }
        close(&mut app);
        folder_modals::open_new_folder(&mut app, "test", None);

        let mut large_metrics = small_metrics;
        for _ in 0..3 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                large_metrics = show_inner(&mut app, ui, ActiveModal::NewFolder);
            });
            out.drop_without_applying_deltas();
        }
        assert!(
            large_metrics.body_rect.height() >= small_metrics.body_rect.height() + 90.0,
            "location list stayed shrunk ({} -> {}) instead of expanding toward 200px max_height",
            small_metrics.body_rect.height(),
            large_metrics.body_rect.height()
        );

        // Compact viewport activates outer modal scroll cleanly without height oscillation.
        let mut compact_heights = Vec::new();
        for _ in 0..6 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 380.0),
                )),
                ..Default::default()
            };
            let mut m = large_metrics;
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                m = show_inner(&mut app, ui, ActiveModal::NewFolder);
            });
            compact_heights.push(m.modal_rect.height());
            assert!(
                m.footer_rect.bottom() <= 380.0,
                "compact modal footer {:?} overflowed viewport",
                m.footer_rect
            );
            out.drop_without_applying_deltas();
        }
        let settled = compact_heights[2..].to_vec();
        assert!(
            settled.windows(2).all(|w| (w[0] - w[1]).abs() <= 1.0),
            "compact modal height oscillated across frames: {compact_heights:?}"
        );
    }

    #[test]
    fn confirm_reorder_page_modal_renders_centered_with_pinned_footer() {
        let ctx = egui::Context::default();
        let mut app = PaperCraftApp::default();
        let _ = app.session.execute("page.add", &serde_json::json!({}));
        let _ = app.session.execute("page.add", &serde_json::json!({}));
        app.view.pending_page_reorder = Some(crate::PendingPageReorder {
            notebook_index: app.session.active_notebook,
            from: 2,
            to: 0,
        });
        open(&mut app, ActiveModal::ConfirmReorderPage);

        let mut metrics = ModalMetrics {
            modal_rect: egui::Rect::NOTHING,
            close_btn_rect: egui::Rect::NOTHING,
            body_rect: egui::Rect::NOTHING,
            footer_rect: egui::Rect::NOTHING,
            inner_margin: 0.0,
        };
        let mut saw_prompt = false;
        for _ in 0..4 {
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1280.0, 800.0),
                )),
                ..Default::default()
            };
            let out = ctx.run_ui(raw, |ui| {
                crate::theme::apply_light_visuals(ui.ctx());
                metrics = show_inner(&mut app, ui, ActiveModal::ConfirmReorderPage);
            });
            saw_prompt |=
                !find_unclipped_text_rects(&out.shapes, "Move Page 3 to position 1?").is_empty();
            out.drop_without_applying_deltas();
        }
        assert!(saw_prompt);
        assert!(
            (metrics.modal_rect.width() - ActiveModal::ConfirmReorderPage.spec().width).abs()
                <= 4.0
        );
        assert!((metrics.modal_rect.center().x - 640.0).abs() <= 2.0);
        assert!((metrics.modal_rect.center().y - 400.0).abs() <= 2.0);
        assert!(metrics.footer_rect.top() >= metrics.body_rect.bottom() + 20.0);
    }

    fn find_unaligned_warnings(shapes: &[egui::epaint::ClippedShape]) -> Vec<egui::Rect> {
        find_unclipped_text_rects(shapes, "Unaligned")
            .into_iter()
            .map(|(r, _)| r)
            .collect()
    }

    #[test]
    fn modals_and_transitions_never_paint_egui_unaligned_warnings() {
        let mut failures = Vec::new();
        let mut check_case =
            |label: &str,
             viewport: egui::Vec2,
             pixels_per_point: f32,
             setup: &dyn Fn(&mut PaperCraftApp),
             step_fn: &dyn Fn(usize, &mut PaperCraftApp)| {
                let ctx = egui::Context::default();
                ctx.set_pixels_per_point(pixels_per_point);
                let mut app = PaperCraftApp::default();
                app.session.prefs.theme = papercraft_doc::UiTheme::Dark;
                setup(&mut app);
                for frame in 0..45 {
                    step_fn(frame, &mut app);
                    let raw = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, viewport)),
                        time: Some(frame as f64 * 0.016),
                        predicted_dt: 0.016,
                        ..Default::default()
                    };
                    let out = ctx.run_ui(raw, |ui| {
                        crate::theme::apply_visuals(ui.ctx(), papercraft_doc::UiTheme::Dark);
                        show(&mut app, ui);
                    });
                    let warnings = find_unaligned_warnings(&out.shapes);
                    out.drop_without_applying_deltas();
                    if !warnings.is_empty() {
                        failures.push(format!(
                            "{label} frame={frame}: {} Unaligned at {:?}",
                            warnings.len(),
                            warnings
                        ));
                    }
                }
            };

        // 1. AddAccount opening in Server mode (unconfigured and configured), switching to Local and back, then closing.
        for configured in [false, true] {
            let label = format!("AddAccount configured={configured} Server<->Local");
            check_case(
                &label,
                egui::vec2(1280.0, 800.0),
                1.0,
                &move |app| {
                    if configured {
                        let _ = app
                            .session
                            .set_server_url_preference("https://sync.example.com");
                    }
                    open(app, ActiveModal::AddAccount);
                    app.view.account.add_account_mode =
                        crate::account_modal::AddAccountMode::Server;
                },
                &|frame, app| {
                    if frame == 10 {
                        app.view.account.tab = crate::account_modal::AuthTab::CreateAccount;
                    } else if frame == 18 {
                        app.view.account.add_account_mode =
                            crate::account_modal::AddAccountMode::Local;
                    } else if frame == 28 {
                        app.view.account.add_account_mode =
                            crate::account_modal::AddAccountMode::Server;
                    } else if frame == 36 {
                        close(app);
                    }
                },
            );
        }

        // 2. Stepped & collapsible modals across subflows, collapses, and close fade-outs.
        check_case(
            "NewNotebook steps+collapses",
            egui::vec2(1280.0, 800.0),
            1.25,
            &|app| {
                open(app, ActiveModal::NewNotebook);
                app.session
                    .execute(
                        "profile.dismissFirstNotebookDataNotice",
                        &serde_json::json!({}),
                    )
                    .ok();
                open(app, ActiveModal::NewNotebook);
            },
            &|frame, app| {
                if frame == 6 {
                    app.view.new_notebook_cover =
                        papercraft_doc::NotebookCover::custom_rgb(0x2B, 0x7A, 0x78);
                    app.view
                        .cover_color_draft
                        .sync_from_cover(app.view.new_notebook_cover);
                } else if frame == 14 {
                    app.view.new_notebook_step = 1;
                    app.view
                        .new_notebook_format
                        .select_preset(papercraft_doc::PageResolutionPreset::Custom);
                } else if frame == 24 {
                    app.view.new_notebook_step = 2;
                } else if frame == 34 {
                    close(app);
                }
            },
        );

        check_case(
            "SelectLocalAccount subflows+drawer",
            egui::vec2(1280.0, 800.0),
            1.5,
            &|app| {
                let _ = app.session.execute(
                    "profile.completeFirstAccess",
                    &serde_json::json!({ "nickname": "Personal" }),
                );
                open(app, ActiveModal::SelectLocalAccount);
            },
            &|frame, app| {
                if frame == 8 {
                    app.view.account.selector_other_server_open = true;
                } else if frame == 18 {
                    app.view.account.selector_other_server_open = false;
                } else if frame == 28 {
                    close(app);
                }
            },
        );

        check_case(
            "TransferPages subflow",
            egui::vec2(1280.0, 800.0),
            1.0,
            &|app| {
                let _ = app
                    .session
                    .execute("notebook.create", &serde_json::json!({ "title": "Second" }));
                open(app, ActiveModal::TransferPages);
            },
            &|frame, app| {
                if frame == 8 {
                    app.view.transfer_create_new_notebook = true;
                } else if frame == 18 {
                    app.view.transfer_create_new_notebook = false;
                }
            },
        );

        check_case(
            "AccountSync collapses + AddAccount error",
            egui::vec2(1280.0, 800.0),
            1.25,
            &|app| {
                open(app, ActiveModal::AccountSync);
            },
            &|frame, app| {
                if frame == 8 {
                    app.view.account.profiles_panel_open = true;
                } else if frame == 18 {
                    app.view.account.profiles_panel_open = false;
                } else if frame == 26 {
                    open(app, ActiveModal::AddAccount);
                    app.view.account.add_account_mode = crate::account_modal::AddAccountMode::Local;
                    app.view.account.select_local_error =
                        Some("Please enter a nickname for the local account.".to_string());
                } else if frame == 34 {
                    app.view.account.select_local_error = None;
                }
            },
        );

        // 3. All modals at standard, fractional (150% DPI), and compact viewport sizes.
        let viewports = [
            ("1280x800@1.0", egui::vec2(1280.0, 800.0), 1.0_f32),
            ("853.33x533.33@1.5", egui::vec2(853.3333, 533.3333), 1.5),
            ("1024x576@1.25", egui::vec2(1024.0, 576.0), 1.25),
            ("compact@1.0", egui::vec2(640.0, 460.0), 1.0),
        ];
        let modals = [
            ActiveModal::Export,
            ActiveModal::NewNotebook,
            ActiveModal::NotebookSettings,
            ActiveModal::ConfirmDeleteNotebook,
            ActiveModal::ConfirmReorderPage,
            ActiveModal::AccountSync,
            ActiveModal::AddAccount,
            ActiveModal::FirstAccessNickname,
            ActiveModal::SelectLocalAccount,
            ActiveModal::ConvertLocalAccount,
            ActiveModal::TransferPages,
            ActiveModal::NewFolder,
            ActiveModal::RenameFolder,
            ActiveModal::DeleteFolder,
            ActiveModal::FolderColor,
            ActiveModal::TagColor,
            ActiveModal::AppUpdate,
            ActiveModal::KeyboardShortcuts,
            ActiveModal::WelcomeBanner,
            ActiveModal::FirstNotebookDataNotice,
            ActiveModal::InfiniteTheme,
        ];
        for (vp_name, vp, ppp) in viewports {
            for modal in modals {
                let tag = format!("{modal:?}@{vp_name}");
                check_case(
                    &tag,
                    vp,
                    ppp,
                    &move |app| {
                        open(app, modal);
                    },
                    &|frame, app| {
                        if modal == ActiveModal::NewNotebook && frame == 10 {
                            app.view.new_notebook_step = 1;
                        } else if modal == ActiveModal::NotebookSettings && frame == 10 {
                            app.view.notebook_settings_step = 2;
                        } else if modal == ActiveModal::AccountSync && frame == 10 {
                            app.view.account.tab = crate::account_modal::AuthTab::CreateAccount;
                        } else if frame == 36 {
                            close(app);
                        }
                    },
                );
            }
        }

        assert!(
            failures.is_empty(),
            "Found Unaligned warnings:\n{}",
            failures.join("\n")
        );
    }
}
