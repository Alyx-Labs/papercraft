#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

#[cfg(not(target_os = "windows"))]
mod unix_platform;

use std::io::Write as _;
use std::path::{Path, PathBuf};
#[cfg(target_os = "windows")]
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

const DIAGNOSTIC_LOG_FILE: &str = "papercraft.log";
const MAX_DIAGNOSTIC_LOG_BYTES: u64 = 256 * 1024;
const MAX_LOG_LINE_CHARS: usize = 2048;

fn sanitize_log_line(raw: &str) -> String {
    let mut out = String::new();
    for ch in raw.chars() {
        if out.chars().count() >= MAX_LOG_LINE_CHARS {
            break;
        }
        if matches!(ch, '\r' | '\n' | '\t') || ch.is_control() {
            if !out.ends_with(' ') {
                out.push(' ');
            }
        } else {
            out.push(ch);
        }
    }
    out.trim().to_string()
}

fn append_diagnostic_log(dir: Option<&Path>, kind: &str, details: &str) {
    let Some(dir) = dir else {
        return;
    };
    let _ = std::fs::create_dir_all(dir);
    let log_path = dir.join(DIAGNOSTIC_LOG_FILE);
    let too_large =
        std::fs::metadata(&log_path).is_ok_and(|meta| meta.len() >= MAX_DIAGNOSTIC_LOG_BYTES);
    let mut opts = std::fs::OpenOptions::new();
    opts.create(true).write(true);
    if too_large {
        opts.truncate(true);
    } else {
        opts.append(true);
    }
    let Ok(mut file) = opts.open(&log_path) else {
        return;
    };
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis());
    let clean = sanitize_log_line(details);
    let _ = writeln!(file, "[{now_ms}] [{kind}] {clean}");
}

fn panic_payload_text(info: &std::panic::PanicHookInfo<'_>) -> String {
    let payload = if let Some(s) = info.payload().downcast_ref::<&str>() {
        (*s).to_string()
    } else if let Some(s) = info.payload().downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic payload".to_string()
    };
    match info.location() {
        Some(loc) => format!(
            "{payload} at {}:{}:{}",
            loc.file(),
            loc.line(),
            loc.column()
        ),
        None => payload,
    }
}

fn install_panic_logger(library_dir: Option<PathBuf>) {
    let Some(dir) = library_dir else {
        return;
    };
    let prev_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let msg = panic_payload_text(info);
        append_diagnostic_log(Some(&dir), "panic", &msg);
        prev_hook(info);
    }));
}

use papercraft_engine::Session;
#[cfg(target_os = "windows")]
use papercraft_ui_egui::ClipboardImagePayload;
#[cfg(target_os = "windows")]
use papercraft_ui_egui::stylus::PenSample;
#[cfg(target_os = "windows")]
use papercraft_ui_egui::{FileDialogKind, PickedOpenFile};
use papercraft_ui_egui::{PaperCraftApp, PlatformHooks};

pub(crate) fn remember_parent_dir(slot: &Mutex<Option<PathBuf>>, path: &Path) {
    if let Some(parent) = path.parent()
        && let Ok(mut guard) = slot.lock()
    {
        *guard = Some(parent.to_path_buf());
    }
}

pub(crate) fn cached_dir(slot: &Mutex<Option<PathBuf>>) -> Option<PathBuf> {
    slot.lock().ok().and_then(|guard| guard.clone())
}

pub(crate) fn is_safe_external_url(url: &str) -> bool {
    let trimmed = url.trim();
    !trimmed.is_empty()
        && trimmed.len() <= 2048
        && !trimmed.chars().any(|c| c.is_control() || c.is_whitespace())
        && (trimmed.starts_with("https://")
            || trimmed.starts_with("http://")
            || trimmed.starts_with("mailto:"))
}

#[cfg(target_os = "windows")]
fn install_platform_hooks() -> (Option<papercraft_tablet::windows::Monitor>, PlatformHooks) {
    let monitor = match papercraft_tablet::windows::Monitor::install() {
        Ok(m) => {
            log::info!(
                "Installed Windows Ink pointer monitor (tracks_leave={})",
                m.tracks_leave()
            );
            Some(m)
        }
        Err(err) => {
            log::warn!("Windows Ink monitor unavailable, falling back to pointer input: {err}");
            None
        }
    };

    let last_dir: Arc<Mutex<Option<PathBuf>>> = Arc::new(Mutex::new(None));
    let save_dir = Arc::clone(&last_dir);
    let open_dir = Arc::clone(&last_dir);
    let image_dir = Arc::clone(&last_dir);

    let hooks = PlatformHooks {
        tablet_poll: Some(Box::new(|| {
            papercraft_tablet::windows::drain_samples()
                .into_iter()
                .map(|s| PenSample {
                    pressure: s.pressure,
                    tilt_x: s.tilt_x,
                    tilt_y: s.tilt_y,
                    rotation: s.rotation,
                    eraser: s.eraser,
                    barrel: s.barrel,
                    barrel_secondary: s.barrel_secondary,
                    time_ms: s.time_ms,
                })
                .collect()
        })),
        pen_pointer_id: Some(Box::new(papercraft_tablet::windows::pen_pointer_id)),
        pen_end: Some(Box::new(papercraft_tablet::windows::pen_end)),
        barrel_down: Some(Box::new(papercraft_tablet::windows::is_barrel_button_down)),
        secondary_barrel_down: Some(Box::new(
            papercraft_tablet::windows::is_secondary_barrel_button_down,
        )),
        barrel_press_count: Some(Box::new(
            papercraft_tablet::windows::take_barrel_press_count,
        )),
        secondary_barrel_press_count: Some(Box::new(
            papercraft_tablet::windows::take_secondary_barrel_press_count,
        )),
        pen_devices_dirty: Some(Box::new(papercraft_tablet::windows::take_pen_devices_dirty)),
        connected_pen_devices: Some(Box::new(|| {
            papercraft_tablet::windows::connected_pen_devices()
                .into_iter()
                .map(|d| papercraft_ui_egui::stylus::ConnectedStylusInfo {
                    name: d.name,
                    is_external: d.kind == papercraft_tablet::PenDeviceKind::External,
                })
                .collect()
        })),
        pick_save_path: Some(Box::new(move |kind: FileDialogKind, default_name: &str| {
            let init = cached_dir(&save_dir);
            let title = format!("Save {}", kind.filter_label());
            let chosen = papercraft_tablet::windows::pick_save_file(
                &title,
                kind.filter_label(),
                kind.extension(),
                default_name,
                init.as_deref(),
            );
            if let Some(ref path) = chosen {
                remember_parent_dir(&save_dir, path);
            }
            chosen
        })),
        pick_open_file: Some(Box::new(move |kind: FileDialogKind| {
            let init = cached_dir(&open_dir);
            let title = format!("Open {}", kind.filter_label());
            let chosen = papercraft_tablet::windows::pick_open_file_exts(
                &title,
                kind.filter_label(),
                kind.open_extensions(),
                init.as_deref(),
            )?;
            remember_parent_dir(&open_dir, &chosen);
            Some(PickedOpenFile::Path(chosen))
        })),
        pick_image_file: Some(Box::new(move || {
            let init = cached_dir(&image_dir);
            let chosen =
                papercraft_tablet::windows::pick_open_image_file("Insert Image", init.as_deref())?;
            remember_parent_dir(&image_dir, &chosen);
            Some(PickedOpenFile::Path(chosen))
        })),
        clipboard_image: Some(Box::new(|| {
            papercraft_tablet::windows::read_clipboard_image().map(|img| match img {
                papercraft_tablet::ClipboardImage::EncodedBytes(bytes) => {
                    ClipboardImagePayload::EncodedBytes(bytes)
                }
                papercraft_tablet::ClipboardImage::FilePath(path) => {
                    ClipboardImagePayload::FilePath(path)
                }
            })
        })),
        clipboard_image_candidates: Some(Box::new(|| {
            papercraft_tablet::windows::read_clipboard_image_candidates()
                .into_iter()
                .map(|img| match img {
                    papercraft_tablet::ClipboardImage::EncodedBytes(bytes) => {
                        ClipboardImagePayload::EncodedBytes(bytes)
                    }
                    papercraft_tablet::ClipboardImage::FilePath(path) => {
                        ClipboardImagePayload::FilePath(path)
                    }
                })
                .collect()
        })),
        clipboard_text: Some(Box::new(papercraft_tablet::windows::read_clipboard_text)),
        set_clipboard_text: Some(Box::new(|text| {
            let _ = papercraft_tablet::windows::write_clipboard_text(text);
        })),
        open_url: Some(Box::new(|url| {
            let trimmed = url.trim();
            if is_safe_external_url(trimmed) {
                let _ = std::process::Command::new("rundll32.exe")
                    .args(["url.dll,FileProtocolHandler", trimmed])
                    .spawn();
            }
        })),
        paste_poll: Some(Box::new(papercraft_tablet::windows::take_paste_requested)),
        sync_desktop_shortcut: Some(Box::new(|enabled| {
            papercraft_updater::sync_desktop_shortcut(
                papercraft_updater::AppId::PaperCraft,
                enabled,
            )
            .map(|outcome| outcome.status_message())
        })),
        enable_auto_update_check: true,
        ..PlatformHooks::default()
    };

    (monitor, hooks)
}

#[cfg(not(target_os = "windows"))]
fn install_platform_hooks() -> (Option<()>, PlatformHooks) {
    unix_platform::install_platform_hooks()
}

fn main() -> eframe::Result {
    papercraft_updater::cleanup_stale_files(papercraft_updater::AppId::PaperCraft);
    let library_dir = papercraft_engine::default_library_dir();
    install_panic_logger(library_dir.clone());

    let (_monitor_guard, hooks) = install_platform_hooks();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("PaperCraft")
            .with_app_id("papercraft")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([840.0, 560.0])
            .with_icon(egui::IconData {
                rgba: include_bytes!("../assets/branding/papercraft-256.rgba").to_vec(),
                width: 256,
                height: 256,
            }),
        ..Default::default()
    };

    let session = match library_dir.clone() {
        Some(dir) => Session::open_desktop_library(dir),
        None => Session::default(),
    };

    let result = eframe::run_native(
        "PaperCraft",
        native_options,
        Box::new(|cc| {
            let mut app = PaperCraftApp::new(session, hooks);
            app.init_wgpu(cc.wgpu_render_state.as_ref());
            Ok(Box::new(app))
        }),
    );
    if let Err(ref err) = result {
        append_diagnostic_log(library_dir.as_deref(), "eframe_error", &err.to_string());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn append_diagnostic_log_writes_and_caps_growth() {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!("papercraft_diag_log_{nanos}"));
        let _ = std::fs::remove_dir_all(&dir);
        struct Guard(PathBuf);
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
        let _guard = Guard(dir.clone());

        append_diagnostic_log(Some(&dir), "startup", "line one\nwith newline");
        let log_path = dir.join(DIAGNOSTIC_LOG_FILE);
        let content = std::fs::read_to_string(&log_path).expect("log file should exist");
        assert!(content.contains("[startup] line one with newline"));

        let oversized = vec![b'x'; (MAX_DIAGNOSTIC_LOG_BYTES as usize) + 64];
        std::fs::write(&log_path, &oversized).expect("seed oversized log");
        append_diagnostic_log(Some(&dir), "panic", "rotated entry");
        let reset_content = std::fs::read_to_string(&log_path).expect("rotated log should read");
        assert!(reset_content.len() < 512);
        assert!(reset_content.contains("[panic] rotated entry"));
    }
}
