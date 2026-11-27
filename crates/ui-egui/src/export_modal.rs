//! Clean, objective export format selector rendered inside the standard modal shell.

use egui::{CornerRadius, Margin, RichText, Sense, Stroke, StrokeKind, Vec2, pos2};
use serde_json::json;

use crate::icons::{Icon, paint_icon, toggle_switch_row_enabled};
use crate::inspector::{export_target_with_ext, title_to_default_filename};
use crate::modal::{ActiveModal, primary_action_button, secondary_action_button};
use crate::motion::{hover_factor, lerp_color};
use crate::theme::Tokens;
use crate::{ExportPageScope, FileDialogKind, PaperCraftApp, PdfExportProgressView};

#[derive(Clone, Copy)]
struct ExportFormatSpec {
    kind: FileDialogKind,
    label: &'static str,
    extension_tag: &'static str,
    tooltip: &'static str,
}

const EXPORT_FORMATS: [ExportFormatSpec; 5] = [
    ExportFormatSpec {
        kind: FileDialogKind::Pdf,
        label: "PDF document",
        extension_tag: ".pdf",
        tooltip: "Export all pages with ink, text, shapes, and images as a multi-page PDF.",
    },
    ExportFormatSpec {
        kind: FileDialogKind::Png,
        label: "PNG image",
        extension_tag: ".png",
        tooltip: "Export the active page as a high-resolution PNG image.",
    },
    ExportFormatSpec {
        kind: FileDialogKind::Svg,
        label: "SVG vector",
        extension_tag: ".svg",
        tooltip: "Export the active page as scalable vector artwork.",
    },
    ExportFormatSpec {
        kind: FileDialogKind::Pcraft,
        label: "Notebook file",
        extension_tag: ".pcraft",
        tooltip: "Save a portable PaperCraft notebook archive.",
    },
    ExportFormatSpec {
        kind: FileDialogKind::Package,
        label: "Complete package",
        extension_tag: ".pcraftpkg",
        tooltip: "Export all active notebooks, trashed items, folders, templates, and studio preferences into one backup file.",
    },
];

/// Opens the standard Export modal pre-scoped to the ordered 0-based `pages` selection.
pub(crate) fn open_for_pages(app: &mut PaperCraftApp, pages: Vec<usize>) {
    let page_count = app.session.doc.page_count().max(1);
    let clean = sanitize_export_pages(&pages, page_count);
    let has_selection = !clean.is_empty();
    app.view.export_target_pages = if has_selection { Some(clean) } else { None };
    crate::modal::open(app, ActiveModal::Export);
    app.view.export_page_scope = if has_selection {
        ExportPageScope::SidebarSelection
    } else {
        ExportPageScope::AllPages
    };
}

fn sanitize_export_pages(raw: &[usize], page_count: usize) -> Vec<usize> {
    let mut out = Vec::with_capacity(raw.len().min(page_count));
    for &idx in raw {
        if idx < page_count && !out.contains(&idx) {
            out.push(idx);
        }
    }
    out
}

/// Parses a 1-based comma/semicolon-separated page selection string (e.g. `"1-10, 15"`)
/// into deduplicated 0-based page indices bounded by `page_count`.
pub(crate) fn parse_page_range_spec(raw: &str, page_count: usize) -> Result<Vec<usize>, String> {
    if page_count == 0 {
        return Err("Notebook contains no pages.".to_string());
    }
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Enter page numbers or ranges, e.g. 1-5, 12.".to_string());
    }
    let mut out = Vec::new();
    for raw_part in trimmed.split([',', ';']) {
        let part = raw_part.trim();
        if part.is_empty() {
            return Err("Remove extra commas or enter valid page numbers.".to_string());
        }
        if let Some((start_raw, end_raw)) = part.split_once('-') {
            if end_raw.contains('-') {
                return Err(format!("Invalid range '{part}'. Use a-b format."));
            }
            let start = parse_one_based_page(start_raw, page_count)?;
            let end = parse_one_based_page(end_raw, page_count)?;
            if start > end {
                return Err(format!(
                    "Invalid range '{start}-{end}': start page must not exceed end page."
                ));
            }
            for page_num in start..=end {
                let idx = page_num - 1;
                if !out.contains(&idx) {
                    out.push(idx);
                }
            }
        } else {
            let page_num = parse_one_based_page(part, page_count)?;
            let idx = page_num - 1;
            if !out.contains(&idx) {
                out.push(idx);
            }
        }
    }
    if out.is_empty() {
        return Err("Enter at least one valid page number.".to_string());
    }
    Ok(out)
}

fn parse_one_based_page(token: &str, page_count: usize) -> Result<usize, String> {
    let clean = token.trim();
    if clean.is_empty() || !clean.bytes().all(|b| b.is_ascii_digit()) {
        return Err(format!("Invalid page number '{clean}'."));
    }
    let num: usize = clean
        .parse()
        .map_err(|_| format!("Page '{clean}' is too large."))?;
    if num == 0 || num > page_count {
        return Err(format!("Page {num} is out of range (1–{page_count})."));
    }
    Ok(num)
}

struct ResolvedExportScope {
    pages: Option<Vec<usize>>,
    annotated_count: usize,
    error: Option<String>,
}

fn resolve_scope_for_kind(
    app: &PaperCraftApp,
    kind: FileDialogKind,
    page_count: usize,
    sidebar_target: Option<&[usize]>,
) -> ResolvedExportScope {
    let annotated = app.session.annotated_page_indices();
    let annotated_count = annotated.len();
    match kind {
        FileDialogKind::Package => ResolvedExportScope {
            pages: None,
            annotated_count,
            error: None,
        },
        FileDialogKind::Png | FileDialogKind::Svg => ResolvedExportScope {
            pages: sidebar_target.map(|v| v.to_vec()),
            annotated_count,
            error: None,
        },
        FileDialogKind::Pdf | FileDialogKind::Pcraft => {
            if page_count <= 1 {
                return ResolvedExportScope {
                    pages: None,
                    annotated_count,
                    error: None,
                };
            }
            match app.view.export_page_scope {
                ExportPageScope::AllPages => ResolvedExportScope {
                    pages: None,
                    annotated_count,
                    error: None,
                },
                ExportPageScope::CurrentPage => ResolvedExportScope {
                    pages: Some(vec![
                        app.session.doc.active_page_index().min(page_count - 1),
                    ]),
                    annotated_count,
                    error: None,
                },
                ExportPageScope::AnnotatedOnly => {
                    if annotated.is_empty() {
                        ResolvedExportScope {
                            pages: Some(Vec::new()),
                            annotated_count,
                            error: Some("No pages have annotations yet.".to_string()),
                        }
                    } else {
                        ResolvedExportScope {
                            pages: Some(annotated),
                            annotated_count,
                            error: None,
                        }
                    }
                }
                ExportPageScope::PageRange => {
                    match parse_page_range_spec(&app.view.export_page_range_draft, page_count) {
                        Ok(list) => ResolvedExportScope {
                            pages: Some(list),
                            annotated_count,
                            error: None,
                        },
                        Err(err) => ResolvedExportScope {
                            pages: None,
                            annotated_count,
                            error: Some(err),
                        },
                    }
                }
                ExportPageScope::SidebarSelection => ResolvedExportScope {
                    pages: sidebar_target.map(|v| v.to_vec()),
                    annotated_count,
                    error: None,
                },
            }
        }
    }
}

/// Renders the Export Notebook modal body. Returns `true` if the modal should close.
pub(crate) fn render_body(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) -> bool {
    if !std::mem::take(&mut app.pdf_export_stepped_in_show_ui) {
        app.poll_async_pdf_export(ui.ctx());
    }
    if app.view.active_modal != Some(ActiveModal::Export) {
        return true;
    }

    let is_exporting = app.is_pdf_export_active();
    if !is_exporting && app.view.active_modal == Some(ActiveModal::Export) {
        let (down, up) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowUp),
            )
        });
        if down || up {
            let cur_idx = EXPORT_FORMATS
                .iter()
                .position(|spec| spec.kind == app.view.export_format)
                .unwrap_or(0);
            let next_idx = if down {
                (cur_idx + 1).min(EXPORT_FORMATS.len() - 1)
            } else {
                cur_idx.saturating_sub(1)
            };
            if let Some(next_spec) = EXPORT_FORMATS.get(next_idx)
                && next_spec.kind != app.view.export_format
            {
                app.view.export_format = next_spec.kind;
                app.view.export_feedback = None;
            }
        }
    }

    let page_count = app.session.doc.page_count().max(1);
    let active_num = app.session.doc.active_page_index() + 1;
    let sidebar_target = app
        .view
        .export_target_pages
        .as_deref()
        .map(|raw| sanitize_export_pages(raw, page_count))
        .filter(|v| !v.is_empty());

    let mut chosen_format = None;
    ui.add_enabled_ui(!is_exporting, |ui| {
        let annotated_count = app.session.annotated_page_indices().len();
        egui::Frame::NONE
            .fill(t.panel_bg)
            .stroke(Stroke::new(1.0, t.border))
            .corner_radius(10.0)
            .inner_margin(Margin::same(6))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.spacing_mut().item_spacing.y = 4.0;
                for spec in EXPORT_FORMATS {
                    let row_resolved = resolve_scope_for_kind(
                        app,
                        spec.kind,
                        page_count,
                        sidebar_target.as_deref(),
                    );
                    let scope = format_scope_label(
                        spec.kind,
                        page_count,
                        active_num,
                        app.view.export_page_scope,
                        &row_resolved,
                        annotated_count,
                        app.view.export_separate_files,
                    );
                    let selected = app.view.export_format == spec.kind;
                    if draw_format_radio_row(ui, spec, &scope, selected, t).clicked() {
                        chosen_format = Some(spec.kind);
                    }
                }
            });

        ui.add_space(8.0);
        if app.view.export_format == FileDialogKind::Package {
            egui::Frame::NONE
                .fill(t.panel_bg)
                .stroke(Stroke::new(1.0, t.border))
                .corner_radius(10.0)
                .inner_margin(Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.add(
                        egui::Label::new(
                            RichText::new(
                                "Complete Backup Package: exports all active notebooks, Trash items, folders, custom templates, and studio settings together.",
                            )
                            .size(11.0)
                            .color(t.text_muted),
                        )
                        .wrap(),
                    );
                });
        } else {
            draw_page_scope_options(app, ui, page_count, sidebar_target.as_deref(), t);
        }
    });

    if let Some(kind) = chosen_format {
        app.view.export_format = kind;
        app.view.export_feedback = None;
    }

    if let Some(progress) = app.view.pdf_export_progress.as_ref() {
        ui.add_space(8.0);
        draw_pdf_export_progress_card(ui, progress, t);
    } else if let Some((is_error, message)) = app.view.export_feedback.as_ref() {
        ui.add_space(8.0);
        draw_feedback_banner(ui, *is_error, message, t);
    }

    false
}

/// Renders the Export Notebook modal footer (`Cancel` and `Export`). Returns `true` when closed.
pub(crate) fn render_footer(app: &mut PaperCraftApp, ui: &mut egui::Ui, t: &Tokens) -> bool {
    let is_exporting = app.is_pdf_export_active();
    let cancelling = app
        .view
        .pdf_export_progress
        .as_ref()
        .is_some_and(|p| p.cancelling);

    let page_count = app.session.doc.page_count().max(1);
    let sidebar_target = app
        .view
        .export_target_pages
        .as_deref()
        .map(|raw| sanitize_export_pages(raw, page_count))
        .filter(|v| !v.is_empty());
    let resolved = resolve_scope_for_kind(
        app,
        app.view.export_format,
        page_count,
        sidebar_target.as_deref(),
    );
    let can_export = !is_exporting && resolved.error.is_none();

    let enter_pressed = can_export
        && app.view.active_modal == Some(ActiveModal::Export)
        && ui.input(|i| i.key_pressed(egui::Key::Enter));
    let mut should_close = false;
    let mut export_requested = enter_pressed;

    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            if primary_action_button(ui, "Export", can_export, t).clicked() && can_export {
                export_requested = true;
            }
            if secondary_action_button(ui, "Cancel", !cancelling, t).clicked() {
                if is_exporting {
                    app.cancel_active_pdf_export();
                } else {
                    should_close = true;
                }
            }
        });
    });

    if !should_close && export_requested && can_export {
        if trigger_export_or_save(app, app.view.export_format, ui.ctx()) {
            should_close = true;
        } else {
            ui.ctx().request_repaint();
        }
    }

    should_close
}

/// Exports a selected subset of 0-based `page_indices` to a multi-page PDF file via native/web hooks.
#[cfg(test)]
pub(crate) fn trigger_export_selected_pages_pdf(app: &mut PaperCraftApp, page_indices: &[usize]) {
    if page_indices.is_empty() {
        return;
    }
    let stem = title_to_default_filename(&app.session.doc.title, "");
    let clean_stem = stem.trim_end_matches('.');
    let default_name = format!("{clean_stem}_selected.pdf");
    if let Some(pick_save) = app.hooks.pick_save_path.as_mut() {
        let Some(path) = pick_save(FileDialogKind::Pdf, &default_name) else {
            app.session.last_export_status = Some("Export cancelled.".to_string());
            return;
        };
        if let Err(err) = app.session.execute(
            "export.pdf",
            &json!({ "path": path.to_string_lossy(), "pages": page_indices }),
        ) {
            app.session.last_export_status = Some(err.to_string());
        }
        return;
    }
    if let Some(save_bytes) = app.hooks.save_bytes_dialog.as_mut() {
        match app.session.export_pdf_bytes_for_pages(Some(page_indices)) {
            Ok(bytes) => save_bytes(FileDialogKind::Pdf, &default_name, bytes),
            Err(err) => app.session.last_export_status = Some(err.to_string()),
        }
        return;
    }
    let base_stem = export_target_with_ext(&app.view.export_path_stem, "");
    let path = format!("{}_selected.pdf", base_stem.trim_end_matches('.'));
    if let Err(err) = app.session.execute(
        "export.pdf",
        &json!({ "path": path, "pages": page_indices }),
    ) {
        app.session.last_export_status = Some(err.to_string());
    }
}

pub(crate) fn numbered_export_filename(
    stem: &str,
    order_idx: usize,
    page_idx: usize,
    ext: &str,
) -> String {
    let clean_stem = stem.trim_end_matches('.');
    let clean_ext = ext.trim_start_matches('.');
    format!(
        "{clean_stem}_{:02}_page{}.{}",
        order_idx + 1,
        page_idx + 1,
        clean_ext
    )
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub(crate) fn numbered_export_path(
    base_path: &std::path::Path,
    order_idx: usize,
    page_idx: usize,
    ext: &str,
) -> std::path::PathBuf {
    let stem = base_path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("notebook");
    let file_name = numbered_export_filename(stem, order_idx, page_idx, ext);
    match base_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(file_name),
        _ => std::path::PathBuf::from(file_name),
    }
}

fn command_and_params_for_pages(
    kind: FileDialogKind,
    path: &str,
    pages: Option<&[usize]>,
) -> (&'static str, serde_json::Value) {
    let cmd = command_for_kind(kind);
    let params = match (kind, pages) {
        (FileDialogKind::Pdf | FileDialogKind::Pcraft, Some(list)) if !list.is_empty() => {
            json!({ "path": path, "pages": list })
        }
        (FileDialogKind::Png | FileDialogKind::Svg, Some([page_idx, ..])) => {
            json!({ "path": path, "page": page_idx })
        }
        _ => json!({ "path": path }),
    };
    (cmd, params)
}

fn export_bytes_for_pages(
    session: &mut papercraft_engine::Session,
    kind: FileDialogKind,
    pages: Option<&[usize]>,
) -> Result<Vec<u8>, papercraft_engine::EngineError> {
    match kind {
        FileDialogKind::Pdf => match pages {
            Some(list) if !list.is_empty() => session.export_pdf_bytes_for_pages(Some(list)),
            _ => session.export_pdf_bytes(),
        },
        FileDialogKind::Svg => {
            session.export_svg_bytes(pages.and_then(|list| list.first().copied()))
        }
        FileDialogKind::Png => {
            session.export_png_bytes(pages.and_then(|list| list.first().copied()))
        }
        FileDialogKind::Pcraft => match pages {
            Some(list) if !list.is_empty() => session.save_notebook_bytes_for_pages(Some(list)),
            _ => session.save_notebook_bytes(),
        },
        FileDialogKind::Package => session.export_package_bytes(),
    }
}

fn set_export_error(app: &mut PaperCraftApp, err: impl std::fmt::Display) -> bool {
    let msg = err.to_string();
    app.session.last_export_status = Some(msg.clone());
    app.view.export_feedback = Some((true, msg));
    false
}

fn export_pages_separately(app: &mut PaperCraftApp, kind: FileDialogKind, pages: &[usize]) -> bool {
    let title_stem = title_to_default_filename(&app.session.doc.title, "");
    let default_name = title_to_default_filename(&app.session.doc.title, kind.extension());

    if let Some(pick_choice) = app
        .hooks
        .pick_save_path
        .as_mut()
        .map(|pick_save| pick_save(kind, &default_name))
    {
        let Some(base_path) = pick_choice else {
            let msg = "Export cancelled.".to_string();
            app.session.last_export_status = Some(msg.clone());
            app.view.export_feedback = Some((false, msg));
            return false;
        };
        for (order_idx, &page_idx) in pages.iter().enumerate() {
            let path = numbered_export_path(&base_path, order_idx, page_idx, kind.extension());
            let (cmd, params) =
                command_and_params_for_pages(kind, &path.to_string_lossy(), Some(&[page_idx]));
            if let Err(err) = app.session.execute(cmd, &params) {
                return set_export_error(app, err);
            }
        }
        app.session.last_export_status = Some(format!(
            "Exported {} separate {} files in selection order.",
            pages.len(),
            kind.extension()
        ));
        app.view.export_feedback = None;
        return true;
    }

    if app.hooks.save_bytes_dialog.is_some() {
        let mut prepared = Vec::with_capacity(pages.len());
        for (order_idx, &page_idx) in pages.iter().enumerate() {
            let file_name =
                numbered_export_filename(&title_stem, order_idx, page_idx, kind.extension());
            match export_bytes_for_pages(&mut app.session, kind, Some(&[page_idx])) {
                Ok(bytes) => prepared.push((file_name, bytes)),
                Err(err) => return set_export_error(app, err),
            }
        }
        if let Some(save_bytes) = app.hooks.save_bytes_dialog.as_mut() {
            for (file_name, bytes) in prepared {
                save_bytes(kind, &file_name, bytes);
            }
        }
        app.session.last_export_status = Some(format!(
            "Prepared {} separate {} files in selection order.",
            pages.len(),
            kind.extension()
        ));
        app.view.export_feedback = None;
        return true;
    }

    let base_stem = export_target_with_ext(&app.view.export_path_stem, "");
    for (order_idx, &page_idx) in pages.iter().enumerate() {
        let path = numbered_export_filename(&base_stem, order_idx, page_idx, kind.extension());
        let (cmd, params) = command_and_params_for_pages(kind, &path, Some(&[page_idx]));
        if let Err(err) = app.session.execute(cmd, &params) {
            return set_export_error(app, err);
        }
    }
    app.session.last_export_status = Some(format!(
        "Exported {} separate {} files in selection order.",
        pages.len(),
        kind.extension()
    ));
    app.view.export_feedback = None;
    true
}

fn trigger_export_or_save(
    app: &mut PaperCraftApp,
    kind: FileDialogKind,
    ctx: &egui::Context,
) -> bool {
    let page_count = app.session.doc.page_count().max(1);
    let sidebar_target = app
        .view
        .export_target_pages
        .as_deref()
        .map(|raw| sanitize_export_pages(raw, page_count))
        .filter(|v| !v.is_empty());
    let resolved = resolve_scope_for_kind(app, kind, page_count, sidebar_target.as_deref());
    if let Some(err) = resolved.error {
        return set_export_error(app, err);
    }
    let target_pages = resolved.pages.filter(|v| !v.is_empty());

    let separate_pages: Option<Vec<usize>> = if app.view.export_separate_files {
        Some(
            target_pages
                .clone()
                .unwrap_or_else(|| (0..page_count).collect()),
        )
    } else if matches!(kind, FileDialogKind::Png | FileDialogKind::Svg)
        && target_pages.as_ref().is_some_and(|p| p.len() > 1)
    {
        target_pages.clone()
    } else {
        None
    };

    if kind == FileDialogKind::Pdf {
        return trigger_async_or_headless_pdf_export(app, target_pages, separate_pages, ctx);
    }

    if kind != FileDialogKind::Package
        && let Some(pages) = separate_pages
        && pages.len() > 1
    {
        return export_pages_separately(app, kind, &pages);
    }

    let default_name = if kind == FileDialogKind::Package {
        "PaperCraft_Complete_Package.pcraftpkg".to_string()
    } else {
        title_to_default_filename(&app.session.doc.title, kind.extension())
    };
    if let Some(pick_choice) = app
        .hooks
        .pick_save_path
        .as_mut()
        .map(|pick_save| pick_save(kind, &default_name))
    {
        let Some(path) = pick_choice else {
            let msg = "Export cancelled.".to_string();
            app.session.last_export_status = Some(msg.clone());
            app.view.export_feedback = Some((false, msg));
            return false;
        };
        let (cmd, params) =
            command_and_params_for_pages(kind, &path.to_string_lossy(), target_pages.as_deref());
        if let Err(err) = app.session.execute(cmd, &params) {
            return set_export_error(app, err);
        }
        app.view.export_feedback = None;
        return true;
    }

    if app.hooks.save_bytes_dialog.is_some() {
        let bytes_res = export_bytes_for_pages(&mut app.session, kind, target_pages.as_deref());
        return match bytes_res {
            Ok(bytes) => {
                if let Some(save_bytes) = app.hooks.save_bytes_dialog.as_mut() {
                    save_bytes(kind, &default_name, bytes);
                }
                app.view.export_feedback = None;
                true
            }
            Err(err) => set_export_error(app, err),
        };
    }

    let path = export_target_with_ext(&app.view.export_path_stem, kind.extension());
    let (cmd, params) = command_and_params_for_pages(kind, &path, target_pages.as_deref());
    if let Err(err) = app.session.execute(cmd, &params) {
        return set_export_error(app, err);
    }
    app.view.export_feedback = None;
    true
}

fn trigger_async_or_headless_pdf_export(
    app: &mut PaperCraftApp,
    target_pages: Option<Vec<usize>>,
    separate_pages: Option<Vec<usize>>,
    ctx: &egui::Context,
) -> bool {
    let default_name =
        title_to_default_filename(&app.session.doc.title, FileDialogKind::Pdf.extension());
    let separate_list = separate_pages.filter(|pages| pages.len() > 1);

    if let Some(pick_choice) = app
        .hooks
        .pick_save_path
        .as_mut()
        .map(|pick_save| pick_save(FileDialogKind::Pdf, &default_name))
    {
        let Some(path) = pick_choice else {
            let msg = "Export cancelled.".to_string();
            app.session.last_export_status = Some(msg.clone());
            app.view.export_feedback = Some((false, msg));
            return false;
        };
        #[cfg(not(target_arch = "wasm32"))]
        {
            let res = if let Some(pages) = separate_list {
                app.start_desktop_pdf_export(path, Some(pages), true, ctx.clone())
            } else {
                app.start_desktop_pdf_export(path, target_pages, false, ctx.clone())
            };
            if let Err(err) = res {
                return set_export_error(app, err);
            }
            return false;
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = (path, ctx);
        }
    }

    if app.hooks.save_bytes_dialog.is_some() {
        let res = if let Some(pages) = separate_list {
            let title_stem = title_to_default_filename(&app.session.doc.title, "");
            app.start_chunked_pdf_export_separate(title_stem, pages, ctx)
        } else {
            app.start_chunked_pdf_export_single(default_name, target_pages.as_deref(), ctx)
        };
        if let Err(err) = res {
            return set_export_error(app, err);
        }
        return false;
    }

    if let Some(pages) = separate_list {
        return export_pages_separately(app, FileDialogKind::Pdf, &pages);
    }
    let path = export_target_with_ext(&app.view.export_path_stem, FileDialogKind::Pdf.extension());
    let (cmd, params) =
        command_and_params_for_pages(FileDialogKind::Pdf, &path, target_pages.as_deref());
    if let Err(err) = app.session.execute(cmd, &params) {
        return set_export_error(app, err);
    }
    app.view.export_feedback = None;
    true
}

const fn command_for_kind(kind: FileDialogKind) -> &'static str {
    match kind {
        FileDialogKind::Pdf => "export.pdf",
        FileDialogKind::Svg => "export.svg",
        FileDialogKind::Png => "export.png",
        FileDialogKind::Pcraft => "notebook.save",
        FileDialogKind::Package => "package.export",
    }
}

fn format_scope_label(
    kind: FileDialogKind,
    page_count: usize,
    active_num: usize,
    page_scope: ExportPageScope,
    resolved: &ResolvedExportScope,
    annotated_count: usize,
    separate_files: bool,
) -> String {
    if kind == FileDialogKind::Package {
        return "All saves & notebooks".to_string();
    }
    if matches!(kind, FileDialogKind::Png | FileDialogKind::Svg) {
        if let Some(pages) = resolved.pages.as_deref() {
            if pages.len() == 1 {
                return format!("Page {}", pages[0] + 1);
            }
            return format!("{} separate files", pages.len());
        }
        return format!("Page {active_num}");
    }

    if page_count > 1 {
        match page_scope {
            ExportPageScope::SidebarSelection => {
                if let Some(pages) = resolved.pages.as_deref() {
                    if pages.len() == 1 {
                        return format!("Page {}", pages[0] + 1);
                    }
                    if separate_files {
                        return format!("{} separate files", pages.len());
                    }
                    return format!("{} selected pages", pages.len());
                }
            }
            ExportPageScope::CurrentPage => return format!("Page {active_num}"),
            ExportPageScope::AnnotatedOnly => {
                if annotated_count == 0 {
                    return "No annotated pages".to_string();
                }
                if separate_files && annotated_count > 1 {
                    return format!("{annotated_count} separate files");
                }
                return if annotated_count == 1 {
                    "1 annotated page".to_string()
                } else {
                    format!("{annotated_count} annotated pages")
                };
            }
            ExportPageScope::PageRange => {
                if let Some(pages) = resolved.pages.as_deref() {
                    if pages.len() == 1 {
                        return format!("Page {}", pages[0] + 1);
                    }
                    if separate_files {
                        return format!("{} separate files", pages.len());
                    }
                    return format!("{} pages in range", pages.len());
                }
                return "Invalid range".to_string();
            }
            ExportPageScope::AllPages => {}
        }
    }

    if separate_files && page_count > 1 {
        return format!("{page_count} separate files");
    }
    match kind {
        FileDialogKind::Pdf => {
            if page_count == 1 {
                "1 page".to_string()
            } else {
                format!("All {page_count} pages")
            }
        }
        FileDialogKind::Png | FileDialogKind::Svg => format!("Page {active_num}"),
        FileDialogKind::Pcraft => "Full notebook".to_string(),
        FileDialogKind::Package => "All saves & notebooks".to_string(),
    }
}

fn draw_scope_pill(
    ui: &mut egui::Ui,
    label: &str,
    selected: bool,
    enabled: bool,
    t: &Tokens,
) -> egui::Response {
    let fill = if selected { t.accent_soft } else { t.field_bg };
    let text_color = if !enabled {
        t.text_muted.gamma_multiply(0.55)
    } else if selected {
        t.accent
    } else {
        t.text
    };
    let stroke = if selected {
        Stroke::new(1.2, t.accent)
    } else {
        Stroke::new(1.0, t.border)
    };
    let btn = egui::Button::new(RichText::new(label).size(11.5).color(text_color))
        .fill(fill)
        .stroke(stroke)
        .corner_radius(CornerRadius::same(6));
    ui.add_enabled(enabled, btn).on_hover_cursor(if enabled {
        egui::CursorIcon::PointingHand
    } else {
        egui::CursorIcon::Default
    })
}

fn draw_page_scope_pills(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    annotated_count: usize,
    sidebar_target: Option<&[usize]>,
    t: &Tokens,
) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 4.0);
        if draw_scope_pill(
            ui,
            "All pages",
            app.view.export_page_scope == ExportPageScope::AllPages,
            true,
            t,
        )
        .clicked()
        {
            app.view.export_page_scope = ExportPageScope::AllPages;
            app.view.export_feedback = None;
        }
        if draw_scope_pill(
            ui,
            "Current page",
            app.view.export_page_scope == ExportPageScope::CurrentPage,
            true,
            t,
        )
        .clicked()
        {
            app.view.export_page_scope = ExportPageScope::CurrentPage;
            app.view.export_feedback = None;
        }
        let ann_label = format!("Annotated ({annotated_count})");
        let ann_resp = draw_scope_pill(
            ui,
            &ann_label,
            app.view.export_page_scope == ExportPageScope::AnnotatedOnly,
            annotated_count > 0,
            t,
        )
        .on_disabled_hover_text("No pages have annotations yet.");
        if ann_resp.clicked() && annotated_count > 0 {
            app.view.export_page_scope = ExportPageScope::AnnotatedOnly;
            app.view.export_feedback = None;
        }
        if draw_scope_pill(
            ui,
            "Range",
            app.view.export_page_scope == ExportPageScope::PageRange,
            true,
            t,
        )
        .clicked()
        {
            app.view.export_page_scope = ExportPageScope::PageRange;
            app.view.export_feedback = None;
        }
        if let Some(pages) = sidebar_target {
            let sel_label = format!("Selected ({})", pages.len());
            if draw_scope_pill(
                ui,
                &sel_label,
                app.view.export_page_scope == ExportPageScope::SidebarSelection,
                true,
                t,
            )
            .clicked()
            {
                app.view.export_page_scope = ExportPageScope::SidebarSelection;
                app.view.export_feedback = None;
            }
        }
    });
}

fn draw_page_scope_options(
    app: &mut PaperCraftApp,
    ui: &mut egui::Ui,
    page_count: usize,
    sidebar_target: Option<&[usize]>,
    t: &Tokens,
) {
    let resolved = resolve_scope_for_kind(app, app.view.export_format, page_count, sidebar_target);
    let split_count = resolved.pages.as_deref().map_or(page_count, <[usize]>::len);
    let can_split = split_count > 1;
    let force_separate_image = matches!(
        app.view.export_format,
        FileDialogKind::Png | FileDialogKind::Svg
    ) && can_split
        && sidebar_target.is_some();

    egui::Frame::NONE
        .fill(t.panel_bg)
        .stroke(Stroke::new(1.0, t.border))
        .corner_radius(10.0)
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;

            if matches!(
                app.view.export_format,
                FileDialogKind::Pdf | FileDialogKind::Pcraft
            ) && page_count > 1
            {
                draw_page_scope_pills(app, ui, resolved.annotated_count, sidebar_target, t);

                if app.view.export_page_scope == ExportPageScope::PageRange {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Pages:").size(12.0).color(t.text));
                        let edit = egui::TextEdit::singleline(&mut app.view.export_page_range_draft)
                            .hint_text("Enter page numbers or ranges...")
                            .desired_width((ui.available_width() - 8.0).max(120.0));
                        ui.add(edit);
                    });
                    if let Some(ref err) = resolved.error {
                        ui.add(
                            egui::Label::new(RichText::new(err).size(11.0).color(t.danger_fg))
                                .wrap(),
                        );
                    } else if let Some(ref pages) = resolved.pages {
                        let msg = if pages.len() == 1 {
                            format!("1 page selected (Page {}).", pages[0] + 1)
                        } else {
                            format!("{} pages selected (out of {page_count}).", pages.len())
                        };
                        ui.add(
                            egui::Label::new(RichText::new(msg).size(11.0).color(t.text_muted))
                                .wrap(),
                        );
                    }
                }
            }

            let show_sidebar_order = matches!(
                app.view.export_format,
                FileDialogKind::Png | FileDialogKind::Svg
            ) || app.view.export_page_scope == ExportPageScope::SidebarSelection;
            if show_sidebar_order && let Some(pages) = sidebar_target {
                let summary = if pages.len() == 1 {
                    format!("Selected page: Page {}", pages[0] + 1)
                } else {
                    let order = pages
                        .iter()
                        .map(|idx| format!("Page {}", idx + 1))
                        .collect::<Vec<_>>()
                        .join(" → ");
                    format!("Selection order ({}): {order}", pages.len())
                };
                ui.add(
                    egui::Label::new(RichText::new(summary).size(11.5).strong().color(t.accent))
                        .wrap(),
                );
            }

            let mut checked = if force_separate_image {
                true
            } else {
                app.view.export_separate_files
            };
            let can_toggle_separate = can_split && !force_separate_image;
            let switch_resp = toggle_switch_row_enabled(
                ui,
                "export_separate_files_switch",
                "Export pages as separate files",
                "Export each page as its own numbered file or combine them together in selection order",
                &mut checked,
                can_toggle_separate,
                t,
            );
            if can_toggle_separate && switch_resp.changed() {
                app.view.export_separate_files = checked;
            }

            let hint = if !can_split {
                "Exports the selected single page."
            } else if checked {
                "Generate one numbered file per page preserving selection order."
            } else {
                "Combine exported pages into a single file preserving selection order."
            };
            ui.add(egui::Label::new(RichText::new(hint).size(11.0).color(t.text_muted)).wrap());
        });
}

fn draw_pdf_export_progress_card(ui: &mut egui::Ui, progress: &PdfExportProgressView, t: &Tokens) {
    let title = if progress.cancelling {
        "Cancelling export…"
    } else {
        "Exporting PDF…"
    };
    let subtitle = if progress.separate_files {
        let total = progress.file_total.max(1);
        let cur = (progress.file_index + 1).min(total);
        format!("Saving file {cur} of {total}…")
    } else if progress.pages_done >= progress.pages_total && progress.pages_total > 0 {
        "Finalizing PDF…".to_string()
    } else {
        let total = progress.pages_total.max(1);
        let cur = (progress.pages_done + 1).min(total);
        format!("Processing page {cur} of {total}…")
    };
    let fraction = if progress.pages_total == 0 {
        1.0
    } else {
        (progress.pages_done as f32 / progress.pages_total as f32).clamp(0.0, 1.0)
    };

    egui::Frame::NONE
        .fill(t.panel_bg)
        .stroke(Stroke::new(1.0, t.accent.gamma_multiply(0.5)))
        .corner_radius(10.0)
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            ui.horizontal(|ui| {
                ui.label(RichText::new(title).size(12.5).strong().color(t.text));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format!(
                            "{} / {}",
                            progress.pages_done.min(progress.pages_total),
                            progress.pages_total.max(1)
                        ))
                        .size(11.5)
                        .color(t.text_muted),
                    );
                });
            });
            ui.add(
                egui::ProgressBar::new(fraction)
                    .desired_width(ui.available_width())
                    .desired_height(6.0)
                    .fill(t.accent),
            );
            ui.label(RichText::new(subtitle).size(11.0).color(t.text_muted));
        });
}

fn draw_format_radio_row(
    ui: &mut egui::Ui,
    spec: ExportFormatSpec,
    scope: &str,
    selected: bool,
    t: &Tokens,
) -> egui::Response {
    let width = ui.available_width().max(160.0);
    let row_h = 44.0_f32;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, row_h), Sense::click());
    if ui.is_rect_visible(rect) {
        let ctx = ui.ctx();
        let hover_t = hover_factor(ctx, resp.id.with("hov"), resp.hovered() && !selected);
        let sel_t = hover_factor(ctx, resp.id.with("sel"), selected);
        let idle_fill = lerp_color(t.panel_bg, t.toolbar_bg, hover_t);
        let fill = lerp_color(idle_fill, t.accent_soft, sel_t);
        let stroke = if sel_t > 0.01 {
            Stroke::new(1.0, t.accent.gamma_multiply(0.55 * sel_t))
        } else {
            Stroke::NONE
        };
        let p = ui.painter();
        p.rect(
            rect,
            CornerRadius::same(7),
            fill,
            stroke,
            StrokeKind::Inside,
        );

        let radio_center = pos2(rect.left() + 20.0, rect.center().y);
        let outer_r = 7.5_f32;
        if selected {
            p.circle(
                radio_center,
                outer_r,
                t.field_bg,
                Stroke::new(1.8, t.accent),
            );
            p.circle_filled(radio_center, 3.8, t.accent);
        } else {
            let ring_c = lerp_color(t.text_muted.gamma_multiply(0.55), t.text_muted, hover_t);
            p.circle(radio_center, outer_r, t.field_bg, Stroke::new(1.4, ring_c));
        }

        let label_x = radio_center.x + outer_r + 12.0;
        let label_galley = p.layout_no_wrap(
            spec.label.to_string(),
            egui::FontId::proportional(13.0),
            t.text,
        );
        let ext_galley = p.layout_no_wrap(
            spec.extension_tag.to_string(),
            egui::FontId::monospace(11.5),
            t.text_muted,
        );
        let scope_galley = p.layout_no_wrap(
            scope.to_string(),
            egui::FontId::proportional(11.5),
            t.text_muted,
        );

        let ext_x = label_x + label_galley.size().x + 8.0;
        let scope_x = (rect.right() - 14.0 - scope_galley.size().x).max(ext_x + 12.0);
        p.galley(
            pos2(label_x, rect.center().y - label_galley.size().y * 0.5),
            label_galley,
            t.text,
        );
        p.galley(
            pos2(ext_x, rect.center().y - ext_galley.size().y * 0.5 + 0.5),
            ext_galley,
            t.text_muted,
        );
        p.galley(
            pos2(scope_x, rect.center().y - scope_galley.size().y * 0.5),
            scope_galley,
            t.text_muted,
        );
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(spec.tooltip)
}

fn draw_feedback_banner(ui: &mut egui::Ui, is_error: bool, message: &str, t: &Tokens) {
    let (fill, border, fg, icon) = if is_error {
        (t.danger_bg, t.danger_border, t.danger_fg, Icon::AlertCircle)
    } else {
        (t.toolbar_bg, t.border, t.text_muted, Icon::Info)
    };
    egui::Frame::NONE
        .fill(fill)
        .stroke(Stroke::new(1.0, border))
        .corner_radius(8.0)
        .inner_margin(Margin::symmetric(10, 7))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 7.0;
                let (icon_rect, _) = ui.allocate_exact_size(Vec2::splat(14.0), Sense::hover());
                paint_icon(ui.painter(), icon_rect, icon, fg);
                ui.add(egui::Label::new(RichText::new(message).size(11.5).color(fg)).wrap());
            });
        });
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::{PlatformHooks, modal};
    use papercraft_doc::UiTheme;
    use papercraft_engine::Session;
    use std::cell::RefCell;
    use std::rc::Rc;

    fn run_modal_frame(
        ctx: &egui::Context,
        app: &mut PaperCraftApp,
        events: Vec<egui::Event>,
    ) -> Vec<egui::epaint::ClippedShape> {
        let raw = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1280.0, 800.0),
            )),
            events,
            ..Default::default()
        };
        let out = ctx.run_ui(raw, |ui| {
            crate::theme::apply_visuals(ui.ctx(), app.session.prefs.theme);
            modal::show(app, ui);
        });
        let shapes = out.shapes.clone();
        out.drop_without_applying_deltas();
        shapes
    }

    fn find_text_rects(
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

    fn click_events(pos: egui::Pos2) -> Vec<egui::Event> {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ]
    }

    #[test]
    fn export_modal_renders_clean_format_options_and_scope_in_light_and_dark() {
        let mut app = PaperCraftApp::default();
        let _ = app.session.execute("page.add", &json!({}));
        let _ = app.session.execute("page.add", &json!({}));
        let _ = app.session.execute("page.select", &json!({ "index": 1 }));
        assert_eq!(app.view.export_format, FileDialogKind::Pdf);

        for theme in [UiTheme::Light, UiTheme::Dark] {
            let theme_name = match theme {
                UiTheme::Light => "Light",
                UiTheme::Dark => "Dark",
            };
            app.session
                .execute("theme.set", &json!({ "theme": theme_name }))
                .unwrap();
            let ctx = egui::Context::default();
            modal::open(&mut app, ActiveModal::Export);
            let mut last_shapes = Vec::new();
            for _ in 0..5 {
                last_shapes = run_modal_frame(&ctx, &mut app, Vec::new());
            }

            for label in [
                "Export Notebook",
                "PDF document",
                ".pdf",
                "All 3 pages",
                "PNG image",
                ".png",
                "Page 2",
                "SVG vector",
                ".svg",
                "Notebook file",
                ".pcraft",
                "Full notebook",
                "Cancel",
                "Export",
            ] {
                let rects = find_text_rects(&last_shapes, label);
                assert!(
                    !rects.is_empty(),
                    "{theme:?} missing expected export text '{label}'"
                );
                for (r, clip) in rects {
                    assert!(
                        r.top() >= clip.top() - 0.5 && r.bottom() <= clip.bottom() + 0.5,
                        "{theme:?} clipped '{label}': {r:?} vs {clip:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn selecting_format_and_clicking_export_saves_and_closes_modal() {
        let captured: Rc<RefCell<Vec<(FileDialogKind, String, usize)>>> =
            Rc::new(RefCell::new(Vec::new()));
        let captured_hook = Rc::clone(&captured);
        let hooks = PlatformHooks {
            save_bytes_dialog: Some(Box::new(move |kind, name, bytes| {
                captured_hook
                    .borrow_mut()
                    .push((kind, name.to_string(), bytes.len()));
            })),
            ..Default::default()
        };
        let mut app = PaperCraftApp::new(Session::default(), hooks);
        let ctx = egui::Context::default();
        modal::open(&mut app, ActiveModal::Export);

        let mut last_shapes = Vec::new();
        for _ in 0..5 {
            last_shapes = run_modal_frame(&ctx, &mut app, Vec::new());
        }

        // Select PNG image row.
        let png_rect = find_text_rects(&last_shapes, "PNG image")[0].0;
        let shapes_after = run_modal_frame(&ctx, &mut app, click_events(png_rect.center()));
        assert_eq!(app.view.export_format, FileDialogKind::Png);

        // Click Export primary button.
        let export_rect = find_text_rects(&shapes_after, "Export")[0].0;
        let _ = run_modal_frame(&ctx, &mut app, click_events(export_rect.center()));
        assert_eq!(app.view.active_modal, None);

        let calls = captured.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, FileDialogKind::Png);
        assert!(calls[0].1.ends_with(".png"));
        assert!(calls[0].2 > 0);
    }

    #[test]
    fn cancelled_save_dialog_keeps_modal_open_with_notice_until_format_changes() {
        let hooks = PlatformHooks {
            pick_save_path: Some(Box::new(|_, _| None)),
            ..Default::default()
        };
        let mut app = PaperCraftApp::new(Session::default(), hooks);
        let ctx = egui::Context::default();
        modal::open(&mut app, ActiveModal::Export);

        let mut last_shapes = Vec::new();
        for _ in 0..5 {
            last_shapes = run_modal_frame(&ctx, &mut app, Vec::new());
        }

        let export_rect = find_text_rects(&last_shapes, "Export")[0].0;
        let _ = run_modal_frame(&ctx, &mut app, click_events(export_rect.center()));
        let shapes = run_modal_frame(&ctx, &mut app, Vec::new());
        assert_eq!(app.view.active_modal, Some(ActiveModal::Export));
        assert!(
            !find_text_rects(&shapes, "Export cancelled.").is_empty(),
            "expected 'Export cancelled.' feedback inside modal"
        );

        // Selecting another format clears the stale cancel notice.
        let svg_rect = find_text_rects(&shapes, "SVG vector")[0].0;
        let shapes_after = run_modal_frame(&ctx, &mut app, click_events(svg_rect.center()));
        assert_eq!(app.view.export_format, FileDialogKind::Svg);
        assert!(find_text_rects(&shapes_after, "Export cancelled.").is_empty());

        // Clicking Cancel closes without triggering export.
        let cancel_rect = find_text_rects(&shapes_after, "Cancel")[0].0;
        let _ = run_modal_frame(&ctx, &mut app, click_events(cancel_rect.center()));
        assert_eq!(app.view.active_modal, None);
    }

    #[test]
    fn trigger_export_selected_pages_pdf_exports_only_requested_pages() {
        let captured: Rc<RefCell<Vec<(FileDialogKind, String, usize)>>> =
            Rc::new(RefCell::new(Vec::new()));
        let captured_hook = Rc::clone(&captured);
        let hooks = PlatformHooks {
            save_bytes_dialog: Some(Box::new(move |kind, name, bytes| {
                captured_hook
                    .borrow_mut()
                    .push((kind, name.to_string(), bytes.len()));
            })),
            ..Default::default()
        };
        let mut app = PaperCraftApp::new(Session::default(), hooks);
        let _ = app.session.execute("page.add", &json!({}));
        let _ = app.session.execute("page.add", &json!({}));

        trigger_export_selected_pages_pdf(&mut app, &[0, 2]);
        let calls = captured.borrow();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].0, FileDialogKind::Pdf);
        assert!(calls[0].1.ends_with("_selected.pdf"));
        assert!(calls[0].2 > 0);
        assert_eq!(
            app.session.last_export_status.as_deref(),
            Some(format!("Prepared 2-page PDF ({} bytes).", calls[0].2).as_str())
        );
    }

    #[test]
    fn export_modal_supports_single_page_combined_ordered_and_separate_files_exports() {
        type CapturedBytes = Rc<RefCell<Vec<(FileDialogKind, String, Vec<u8>)>>>;
        let captured: CapturedBytes = Rc::new(RefCell::new(Vec::new()));
        let captured_hook = Rc::clone(&captured);
        let hooks = PlatformHooks {
            save_bytes_dialog: Some(Box::new(move |kind, name, bytes| {
                captured_hook
                    .borrow_mut()
                    .push((kind, name.to_string(), bytes));
            })),
            ..Default::default()
        };
        let mut app = PaperCraftApp::new(Session::default(), hooks);
        // Create 3 distinct paper colors across pages 1..3 to verify exact page selection & ordering.
        let _ = app.session.execute("page.add", &json!({}));
        let _ = app.session.execute("page.add", &json!({}));
        let _ = app.session.execute("page.select", &json!({ "index": 0 }));
        let _ = app
            .session
            .execute("page.setTemplate", &json!({ "color": "WarmCream" }));
        let _ = app.session.execute("page.select", &json!({ "index": 2 }));
        let _ = app
            .session
            .execute("page.setTemplate", &json!({ "color": "DarkSlate" }));

        let ctx = egui::Context::default();

        // 1. Single page export from right-click ("Page 3").
        open_for_pages(&mut app, vec![2]);
        let mut shapes = Vec::new();
        for _ in 0..5 {
            shapes = run_modal_frame(&ctx, &mut app, Vec::new());
        }
        assert!(!find_text_rects(&shapes, "Selected page: Page 3").is_empty());
        let export_rect = find_text_rects(&shapes, "Export")[0].0;
        let _ = run_modal_frame(&ctx, &mut app, click_events(export_rect.center()));
        for _ in 0..8 {
            if app.view.active_modal.is_none() {
                break;
            }
            let _ = run_modal_frame(&ctx, &mut app, Vec::new());
        }
        assert_eq!(app.view.active_modal, None);
        assert!(
            app.session
                .last_export_status
                .as_deref()
                .is_some_and(|s| s.starts_with("Prepared 1-page PDF")),
            "status={:?}",
            app.session.last_export_status
        );
        assert_eq!(captured.borrow().len(), 1);
        captured.borrow_mut().clear();

        // 2. Multi-page selection [2, 0] combined together into .pcraft preserving order Page 3 -> Page 1.
        open_for_pages(&mut app, vec![2, 0]);
        for _ in 0..5 {
            shapes = run_modal_frame(&ctx, &mut app, Vec::new());
        }
        assert!(!find_text_rects(&shapes, "Selection order (2): Page 3 → Page 1").is_empty());
        assert!(!find_text_rects(&shapes, "2 selected pages").is_empty());
        let ncraft_rect = find_text_rects(&shapes, "Notebook file")[0].0;
        let shapes_ncraft = run_modal_frame(&ctx, &mut app, click_events(ncraft_rect.center()));
        let export_rect = find_text_rects(&shapes_ncraft, "Export")[0].0;
        let _ = run_modal_frame(&ctx, &mut app, click_events(export_rect.center()));
        {
            let calls = captured.borrow();
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0].0, FileDialogKind::Pcraft);
            let mut probe = Session::default();
            probe.load_notebook_bytes(&calls[0].2, None, true).unwrap();
            assert_eq!(probe.doc.page_count(), 2);
            assert_eq!(
                probe.doc.pages[0].paper_color,
                papercraft_doc::PaperColor::DarkSlate
            );
            assert_eq!(
                probe.doc.pages[1].paper_color,
                papercraft_doc::PaperColor::WarmCream
            );
        }
        captured.borrow_mut().clear();

        // 3. Toggle "Export pages as separate files" switch -> saves 2 files in order Page 3 then Page 1.
        app.view.export_format = FileDialogKind::Pdf;
        open_for_pages(&mut app, vec![2, 0]);
        for _ in 0..5 {
            shapes = run_modal_frame(&ctx, &mut app, Vec::new());
        }
        let check_rect = find_text_rects(&shapes, "Export pages as separate files")[0].0;
        let shapes_checked = run_modal_frame(&ctx, &mut app, click_events(check_rect.center()));
        assert!(app.view.export_separate_files);
        assert!(!find_text_rects(&shapes_checked, "2 separate files").is_empty());
        let export_rect = find_text_rects(&shapes_checked, "Export")[0].0;
        let _ = run_modal_frame(&ctx, &mut app, click_events(export_rect.center()));
        for _ in 0..16 {
            if app.view.active_modal.is_none() {
                break;
            }
            let _ = run_modal_frame(&ctx, &mut app, Vec::new());
        }
        assert_eq!(app.view.active_modal, None);
        let calls = captured.borrow();
        assert_eq!(calls.len(), 2);
        assert!(
            calls[0].1.ends_with("_01_page3.pdf") && !calls[0].2.is_empty(),
            "first separate file name={}",
            calls[0].1
        );
        assert!(
            calls[1].1.ends_with("_02_page1.pdf") && !calls[1].2.is_empty(),
            "second separate file name={}",
            calls[1].1
        );
    }

    #[test]
    fn parse_page_range_spec_validates_ranges_bounds_and_hostile_input() {
        assert_eq!(
            parse_page_range_spec("1-3, 5; 2", 10).unwrap(),
            vec![0, 1, 2, 4]
        );
        assert_eq!(parse_page_range_spec(" 10 - 10 ", 10).unwrap(), vec![9]);
        assert!(parse_page_range_spec("", 10).is_err());
        assert!(parse_page_range_spec("0", 10).is_err());
        assert!(parse_page_range_spec("11", 10).is_err());
        assert!(parse_page_range_spec("5-2", 10).is_err());
        assert!(parse_page_range_spec("1-2-3", 10).is_err());
        assert!(parse_page_range_spec("abc", 10).is_err());
        assert!(parse_page_range_spec("1,,2", 10).is_err());
    }

    #[test]
    fn export_modal_quick_scope_pills_update_pdf_target_pages_cleanly() {
        let mut app = PaperCraftApp::default();
        let _ = app.session.execute("page.add", &json!({}));
        let _ = app.session.execute("page.add", &json!({}));
        let _ = app.session.execute("page.select", &json!({ "index": 1 }));
        let _ = app.session.execute(
            "stroke.draw",
            &json!({
                "kind": "BallPen",
                "points": [
                    { "x": 10.0, "y": 10.0, "pressure": 0.5 },
                    { "x": 60.0, "y": 10.0, "pressure": 0.5 }
                ]
            }),
        );

        let ctx = egui::Context::default();
        for theme in [UiTheme::Light, UiTheme::Dark] {
            let theme_name = match theme {
                UiTheme::Light => "Light",
                UiTheme::Dark => "Dark",
            };
            app.session
                .execute("theme.set", &json!({ "theme": theme_name }))
                .unwrap();
            modal::open(&mut app, ActiveModal::Export);
            let mut shapes = Vec::new();
            for _ in 0..5 {
                shapes = run_modal_frame(&ctx, &mut app, Vec::new());
            }

            for label in ["All pages", "Current page", "Annotated (1)", "Range"] {
                let rects = find_text_rects(&shapes, label);
                assert!(
                    !rects.is_empty(),
                    "{theme:?} missing quick scope pill '{label}'"
                );
                for (r, clip) in rects {
                    assert!(
                        r.top() >= clip.top() - 0.5 && r.bottom() <= clip.bottom() + 0.5,
                        "{theme:?} clipped '{label}': {r:?} vs {clip:?}"
                    );
                }
            }

            let ann_rect = find_text_rects(&shapes, "Annotated (1)")[0].0;
            let _ = run_modal_frame(&ctx, &mut app, click_events(ann_rect.center()));
            let shapes_ann = run_modal_frame(&ctx, &mut app, Vec::new());
            assert_eq!(app.view.export_page_scope, ExportPageScope::AnnotatedOnly);
            assert!(!find_text_rects(&shapes_ann, "1 annotated page").is_empty());

            let cur_rect = find_text_rects(&shapes_ann, "Current page")[0].0;
            let _ = run_modal_frame(&ctx, &mut app, click_events(cur_rect.center()));
            assert_eq!(app.view.export_page_scope, ExportPageScope::CurrentPage);
        }
    }

    #[test]
    fn async_pdf_export_progress_and_cancel_stops_cleanly() {
        type CapturedBytes = Rc<RefCell<Vec<(FileDialogKind, String, Vec<u8>)>>>;
        let captured: CapturedBytes = Rc::new(RefCell::new(Vec::new()));
        let captured_hook = Rc::clone(&captured);
        let hooks = PlatformHooks {
            save_bytes_dialog: Some(Box::new(move |kind, name, bytes| {
                captured_hook
                    .borrow_mut()
                    .push((kind, name.to_string(), bytes));
            })),
            ..Default::default()
        };
        let mut app = PaperCraftApp::new(Session::default(), hooks);
        for _ in 0..3 {
            let _ = app.session.execute("page.add", &json!({}));
        }

        let ctx = egui::Context::default();
        modal::open(&mut app, ActiveModal::Export);
        let mut shapes = Vec::new();
        for _ in 0..5 {
            shapes = run_modal_frame(&ctx, &mut app, Vec::new());
        }

        // Start a chunked separate-files export of 4 pages, let 1 file finish, then cancel mid-way.
        let check_rect = find_text_rects(&shapes, "Export pages as separate files")[0].0;
        let shapes_checked = run_modal_frame(&ctx, &mut app, click_events(check_rect.center()));
        let export_rect = find_text_rects(&shapes_checked, "Export")[0].0;
        let _ = run_modal_frame(&ctx, &mut app, click_events(export_rect.center()));
        assert!(app.is_pdf_export_active());
        let shapes_prog = run_modal_frame(&ctx, &mut app, Vec::new());
        assert!(
            !find_text_rects(&shapes_prog, "Exporting PDF…").is_empty(),
            "expected in-progress PDF export banner"
        );

        let cancel_rect = find_text_rects(&shapes_prog, "Cancel")[0].0;
        let _ = run_modal_frame(&ctx, &mut app, click_events(cancel_rect.center()));
        assert!(!app.is_pdf_export_active());
        assert_eq!(app.view.pdf_export_progress, None);
        assert_eq!(app.view.active_modal, Some(ActiveModal::Export));
        let shapes_cancelled = run_modal_frame(&ctx, &mut app, Vec::new());
        assert!(
            !find_text_rects(&shapes_cancelled, "Export cancelled.").is_empty(),
            "expected cancellation notice inside modal"
        );
        let saved_before_cancel = captured.borrow().len();
        assert!(saved_before_cancel < 4);

        // Also test combined 40-page PDF progress + Cancel (exceeds two frames at MAX_STEPS_PER_FRAME = 16).
        captured.borrow_mut().clear();
        for _ in 0..36 {
            let _ = app.session.execute("page.add", &json!({}));
        }
        app.view.export_separate_files = false;
        modal::open(&mut app, ActiveModal::Export);
        let mut combined_shapes = Vec::new();
        for _ in 0..5 {
            combined_shapes = run_modal_frame(&ctx, &mut app, Vec::new());
        }
        let export_rect = find_text_rects(&combined_shapes, "Export")[0].0;
        let _ = run_modal_frame(&ctx, &mut app, click_events(export_rect.center()));
        assert!(app.is_pdf_export_active());
        let prog_shapes = run_modal_frame(&ctx, &mut app, Vec::new());
        assert!(!find_text_rects(&prog_shapes, "Exporting PDF…").is_empty());
        assert!(captured.borrow().is_empty());

        let cancel_rect = find_text_rects(&prog_shapes, "Cancel")[0].0;
        let _ = run_modal_frame(&ctx, &mut app, click_events(cancel_rect.center()));
        assert!(!app.is_pdf_export_active());
        assert!(captured.borrow().is_empty());
        assert_eq!(
            app.session.last_export_status.as_deref(),
            Some("Export cancelled.")
        );
    }
}
