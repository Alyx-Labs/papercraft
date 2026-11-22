//! Cross-platform desktop helpers for folder picking, file manager opening, and clipboard copy.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::mpsc;

/// Opens a native folder selection dialog on a background thread and notifies `ctx` when done.
pub fn pick_folder_async(
    initial_dir: Option<PathBuf>,
    ctx: egui::Context,
) -> mpsc::Receiver<Option<PathBuf>> {
    let (tx, rx) = mpsc::sync_channel(1);
    let tx_thread = tx.clone();
    let spawn_res = std::thread::Builder::new()
        .name("papercraft-folder-picker".to_string())
        .spawn(move || {
            let chosen = pick_folder_blocking(initial_dir.as_deref());
            let _ = tx_thread.send(chosen);
            ctx.request_repaint();
        });
    if spawn_res.is_err() {
        let _ = tx.send(None);
    }
    rx
}

fn pick_folder_blocking(initial_dir: Option<&Path>) -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        papercraft_tablet::windows::pick_folder("Choose PaperCraft Server Data Folder", initial_dir)
    }
    #[cfg(target_os = "macos")]
    {
        let script = match initial_dir.filter(|p| p.exists()) {
            Some(dir) => {
                let escaped = dir.display().to_string().replace('"', "");
                format!(
                    "POSIX path of (choose folder with prompt \"Choose PaperCraft Server Data Folder\" default location POSIX file \"{escaped}\")"
                )
            }
            None => {
                "POSIX path of (choose folder with prompt \"Choose PaperCraft Server Data Folder\")"
                    .to_string()
            }
        };
        run_stdout_path("osascript", &["-e", &script])
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        let dir_arg = initial_dir
            .filter(|p| p.exists())
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".to_string());
        run_stdout_path(
            "zenity",
            &[
                "--file-selection",
                "--directory",
                "--title",
                "Choose PaperCraft Server Data Folder",
                "--filename",
                &dir_arg,
            ],
        )
        .or_else(|| {
            run_stdout_path(
                "kdialog",
                &[
                    "--getexistingdirectory",
                    &dir_arg,
                    "--title",
                    "Choose PaperCraft Server Data Folder",
                ],
            )
        })
    }
}

#[cfg(not(target_os = "windows"))]
fn run_stdout_path(program: &str, args: &[&str]) -> Option<PathBuf> {
    let out = std::process::Command::new(program)
        .args(args)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!text.is_empty()).then(|| PathBuf::from(text))
}

/// Ensures `path` exists and opens it in the system file manager.
pub fn open_folder_in_file_manager(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|err| {
        format!(
            "Could not open or create folder `{}`: {err}",
            path.display()
        )
    })?;

    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer.exe")
            .arg(path)
            .spawn()
            .map_err(|err| format!("Could not open Windows Explorer: {err}"))?;
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg(path)
            .spawn()
            .map_err(|err| format!("Could not open Finder: {err}"))?;
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        std::process::Command::new("xdg-open")
            .arg(path)
            .spawn()
            .map_err(|err| format!("Could not open file manager: {err}"))?;
    }
    Ok(())
}

/// Copies `text` to the OS clipboard cleanly even if the UI window is hidden.
pub fn copy_text_to_clipboard(text: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        papercraft_tablet::windows::write_clipboard_text(text)
    }
    #[cfg(target_os = "macos")]
    {
        pipe_to_clipboard_command("pbcopy", &[], text)
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        pipe_to_clipboard_command("wl-copy", &[], text)
            || pipe_to_clipboard_command("xclip", &["-selection", "clipboard"], text)
            || pipe_to_clipboard_command("xsel", &["--clipboard", "--input"], text)
    }
}

#[cfg(not(target_os = "windows"))]
fn pipe_to_clipboard_command(program: &str, args: &[&str], text: &str) -> bool {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let Ok(mut child) = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(text.as_bytes());
    }
    child.wait().is_ok_and(|s| s.success())
}
