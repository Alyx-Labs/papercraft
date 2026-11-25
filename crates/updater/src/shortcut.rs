//! Cross-platform Desktop shortcut creation, update-time refresh, and removal.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::install::macos_app_bundle_root;
use crate::manifest::AppId;

#[cfg(target_os = "linux")]
const MAX_USER_DIRS_BYTES: u64 = 64 * 1024;
const RESERVED_EXEC_CHARS: &[char] = &[
    ' ', '\t', '\n', '"', '\'', '\\', '>', '<', '~', '|', '&', ';', '$', '*', '?', '#', '(', ')',
    '`',
];

#[cfg(any(target_os = "linux", test))]
const APP_ICON_PNG: &[u8] =
    include_bytes!("../../../apps/papercraft/assets/branding/papercraft-256.png");

/// Returns a concise, plain-English description of how PaperCraft creates Desktop shortcuts on the current OS.
#[must_use]
pub const fn shortcut_architecture_hint() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "Creates a Windows shortcut (.lnk) on your Desktop pointing to this app's current folder."
    }
    #[cfg(target_os = "macos")]
    {
        "Creates a Desktop link to this app bundle so you can open it directly from your Desktop."
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        "Creates a desktop launcher (.desktop) on your Desktop pointing to this AppImage or binary."
    }
}

/// Result of synchronizing the OS Desktop shortcut for a PaperCraft executable or bundle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShortcutOutcome {
    CreatedOrUpdated(PathBuf),
    Removed(PathBuf),
    AlreadyAbsent(PathBuf),
    AlreadyOnDesktop(PathBuf),
}

impl ShortcutOutcome {
    /// Returns the OS Desktop shortcut (or on-Desktop app) path evaluated by the sync operation.
    #[must_use]
    pub fn path(&self) -> &Path {
        match self {
            Self::CreatedOrUpdated(p)
            | Self::Removed(p)
            | Self::AlreadyAbsent(p)
            | Self::AlreadyOnDesktop(p) => p,
        }
    }

    /// Returns a concise, plain-English status string suitable for Settings feedback labels.
    #[must_use]
    pub fn status_message(&self) -> String {
        match self {
            Self::CreatedOrUpdated(p) => {
                format!("Desktop shortcut updated at {}.", p.display())
            }
            Self::Removed(p) => {
                format!("Desktop shortcut removed from {}.", p.display())
            }
            Self::AlreadyAbsent(_) => "No Desktop shortcut exists.".to_string(),
            Self::AlreadyOnDesktop(p) => {
                format!(
                    "App already lives directly on Desktop ({}); left untouched.",
                    p.display()
                )
            }
        }
    }
}

/// Resolves the target executable or `.app` bundle path that a Desktop shortcut should launch.
#[must_use]
pub fn resolve_launch_target_for(
    current_exe: &Path,
    appimage_env: Option<&Path>,
    target_os: &str,
) -> PathBuf {
    match target_os {
        "linux" => appimage_env
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| current_exe.to_path_buf(), Path::to_path_buf),
        "macos" => macos_app_bundle_root(current_exe).unwrap_or_else(|| current_exe.to_path_buf()),
        _ => current_exe.to_path_buf(),
    }
}

/// Computes the idiomatic OS Desktop shortcut path inside `desktop_dir` for `app` and `target_os`.
#[must_use]
pub fn shortcut_path_in_dir(
    app: AppId,
    target_path: &Path,
    desktop_dir: &Path,
    target_os: &str,
) -> PathBuf {
    let file_name = match target_os {
        "windows" => format!("{}.lnk", app.display_name()),
        "linux" => format!("{}.desktop", app.manifest_key()),
        "macos" => {
            let resolved =
                macos_app_bundle_root(target_path).unwrap_or_else(|| target_path.to_path_buf());
            let name = resolved
                .file_name()
                .and_then(|n| n.to_str())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| app.macos_app_bundle_name());
            name.to_string()
        }
        _ => format!("{}.lnk", app.display_name()),
    };
    desktop_dir.join(file_name)
}

/// Parses `XDG_DESKTOP_DIR` from an `user-dirs.dirs` configuration file.
#[must_use]
pub fn parse_xdg_user_dirs_desktop(content: &str, home_dir: &Path) -> Option<PathBuf> {
    for raw_line in content.lines() {
        let trimmed = raw_line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let line = trimmed
            .strip_prefix("export ")
            .map_or(trimmed, str::trim_start);
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "XDG_DESKTOP_DIR" {
            continue;
        }
        let unquoted = unquote_xdg_value(value.trim());
        if let Some(resolved) = expand_xdg_desktop_value(&unquoted, home_dir) {
            return Some(resolved);
        }
    }
    None
}

/// Renders a Freedesktop `.desktop` entry pointing directly to `exec_path`.
#[must_use]
pub fn render_linux_desktop_entry(app: AppId, exec_path: &Path, icon_ref: &str) -> String {
    let (comment, categories) = match app {
        AppId::PaperCraft => (
            "PaperCraft notebook and handwriting studio",
            "Office;Graphics;",
        ),
        AppId::PaperCraftServer => (
            "PaperCraft self-hosted sync server control",
            "Utility;Network;",
        ),
    };
    let exec_str = format_desktop_exec_value(&exec_path.to_string_lossy());
    let path_str = exec_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| escape_desktop_string(&p.to_string_lossy()))
        .unwrap_or_default();
    let icon_escaped = escape_desktop_string(icon_ref);

    let mut out = format!(
        "[Desktop Entry]\n\
         Version=1.0\n\
         Type=Application\n\
         Name={name}\n\
         Comment={comment}\n\
         Exec={exec_str}\n\
         Icon={icon_escaped}\n\
         Terminal=false\n\
         Categories={categories}\n\
         StartupNotify=true\n\
         StartupWMClass={wm_class}\n",
        name = app.display_name(),
        wm_class = app.manifest_key(),
    );
    if !path_str.is_empty() {
        out.push_str(&format!("Path={path_str}\n"));
    }
    out
}

/// Synchronizes the current OS Desktop shortcut for the currently running PaperCraft executable.
pub fn sync_desktop_shortcut(app: AppId, enabled: bool) -> Result<ShortcutOutcome, String> {
    let exe = std::env::current_exe()
        .map_err(|err| format!("Could not locate the running executable: {err}"))?;
    let appimage_env = std::env::var_os("APPIMAGE").map(PathBuf::from);
    let target = resolve_launch_target_for(&exe, appimage_env.as_deref(), std::env::consts::OS);
    sync_desktop_shortcut_for_target(app, &target, enabled)
}

/// Synchronizes the current OS Desktop shortcut to point at `target_path`.
pub fn sync_desktop_shortcut_for_target(
    app: AppId,
    target_path: &Path,
    enabled: bool,
) -> Result<ShortcutOutcome, String> {
    let absolute_target = absolutize_path(target_path);
    let resolved_target = if cfg!(target_os = "macos") {
        macos_app_bundle_root(&absolute_target).unwrap_or(absolute_target)
    } else {
        absolute_target
    };
    let desktop_dir = resolve_desktop_dir()?;
    let icons_dir = resolve_linux_icons_dir();
    let outcome = sync_desktop_shortcut_in_dir(
        app,
        &resolved_target,
        &desktop_dir,
        icons_dir.as_deref(),
        enabled,
        std::env::consts::OS,
    )?;

    #[cfg(target_os = "linux")]
    if let ShortcutOutcome::CreatedOrUpdated(ref desktop_file) = outcome {
        mark_linux_desktop_trusted(desktop_file);
    }

    Ok(outcome)
}

/// Synchronizes a Desktop shortcut for `app` -> `target_path` inside an explicit `desktop_dir`.
///
/// Kept pure with respect to user directories so tests never mutate the real user Desktop.
pub fn sync_desktop_shortcut_in_dir(
    app: AppId,
    target_path: &Path,
    desktop_dir: &Path,
    icons_dir: Option<&Path>,
    enabled: bool,
    target_os: &str,
) -> Result<ShortcutOutcome, String> {
    if target_path.as_os_str().is_empty() {
        return Err("Executable path is empty.".to_string());
    }

    let shortcut_path = shortcut_path_in_dir(app, target_path, desktop_dir, target_os);
    if paths_refer_to_same_location(&shortcut_path, target_path) {
        return Ok(ShortcutOutcome::AlreadyOnDesktop(shortcut_path));
    }

    if !enabled {
        return remove_shortcut_at(&shortcut_path, target_os);
    }

    fs::create_dir_all(desktop_dir).map_err(|err| {
        format!(
            "Could not create Desktop directory `{}`: {err}",
            desktop_dir.display()
        )
    })?;

    match target_os {
        "windows" => create_windows_shortcut_at(app, target_path, &shortcut_path),
        "linux" => create_linux_shortcut_at(app, target_path, &shortcut_path, icons_dir),
        "macos" => create_macos_shortcut_at(target_path, &shortcut_path),
        other => Err(format!(
            "Desktop shortcut creation is not supported on platform `{other}`."
        )),
    }
}

fn absolutize_path(path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    match std::env::current_dir() {
        Ok(cwd) => cwd.join(path),
        Err(_) => path.to_path_buf(),
    }
}

fn unquote_xdg_value(raw: &str) -> String {
    let inner = if (raw.starts_with('"') && raw.ends_with('"') && raw.len() >= 2)
        || (raw.starts_with('\'') && raw.ends_with('\'') && raw.len() >= 2)
    {
        &raw[1..raw.len() - 1]
    } else {
        raw
    };

    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\'
            && let Some(next) = chars.peek().copied()
            && matches!(next, '\\' | '"' | '\'')
        {
            out.push(next);
            chars.next();
            continue;
        }
        out.push(ch);
    }
    out
}

fn expand_xdg_desktop_value(value: &str, home_dir: &Path) -> Option<PathBuf> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed
        .strip_prefix("$HOME/")
        .or_else(|| trimmed.strip_prefix("${HOME}/"))
    {
        let rel = rest.trim_start_matches('/');
        if rel.is_empty() {
            return None;
        }
        return Some(home_dir.join(rel));
    }
    if trimmed == "$HOME" || trimmed == "${HOME}" {
        return None;
    }
    let candidate = PathBuf::from(trimmed);
    let is_abs = trimmed.starts_with('/') || candidate.is_absolute();
    if is_abs && candidate != home_dir {
        Some(candidate)
    } else {
        None
    }
}

fn resolve_desktop_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        papercraft_tablet::windows::desktop_dir()
            .ok_or_else(|| "Could not resolve the Windows Desktop folder.".to_string())
    }

    #[cfg(target_os = "linux")]
    {
        if let Some(env_desktop) = std::env::var_os("XDG_DESKTOP_DIR") {
            let p = PathBuf::from(env_desktop);
            if p.is_absolute() {
                return Ok(p);
            }
        }
        let home = std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| "Could not resolve HOME to locate Desktop folder.".to_string())?;
        let config_home = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".config"));
        let user_dirs_file = config_home.join("user-dirs.dirs");
        if let Ok(meta) = fs::metadata(&user_dirs_file)
            && meta.len() <= MAX_USER_DIRS_BYTES
            && let Ok(content) = fs::read_to_string(&user_dirs_file)
            && let Some(dir) = parse_xdg_user_dirs_desktop(&content, &home)
        {
            return Ok(dir);
        }
        Ok(home.join("Desktop"))
    }

    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .ok_or_else(|| "Could not resolve HOME to locate Desktop folder.".to_string())?;
        Ok(home.join("Desktop"))
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
    {
        Err("Desktop shortcut resolution is not implemented for this operating system.".to_string())
    }
}

fn resolve_linux_icons_dir() -> Option<PathBuf> {
    if !cfg!(target_os = "linux") {
        return None;
    }
    let home = std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)?;
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .unwrap_or_else(|| home.join(".local").join("share"));
    Some(
        data_home
            .join("icons")
            .join("hicolor")
            .join("256x256")
            .join("apps"),
    )
}

fn paths_refer_to_same_location(a: &Path, b: &Path) -> bool {
    if a == b {
        return true;
    }
    if let (Ok(ca), Ok(cb)) = (fs::canonicalize(a), fs::canonicalize(b))
        && ca == cb
    {
        return true;
    }
    matches!(
        (a.parent(), b.parent(), a.file_name(), b.file_name()),
        (Some(pa), Some(pb), Some(fa), Some(fb)) if pa == pb && fa == fb
    )
}

fn remove_shortcut_at(shortcut_path: &Path, target_os: &str) -> Result<ShortcutOutcome, String> {
    let meta = match fs::symlink_metadata(shortcut_path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            return Ok(ShortcutOutcome::AlreadyAbsent(shortcut_path.to_path_buf()));
        }
        Err(err) => {
            return Err(format!(
                "Could not inspect `{}`: {err}",
                shortcut_path.display()
            ));
        }
    };

    if target_os == "macos" && !meta.file_type().is_symlink() {
        if meta.is_dir() {
            return Ok(ShortcutOutcome::AlreadyOnDesktop(
                shortcut_path.to_path_buf(),
            ));
        }
        return Err(format!(
            "Refusing to remove `{}` because it is not a symlink created by PaperCraft.",
            shortcut_path.display()
        ));
    }

    if meta.is_dir() && !meta.file_type().is_symlink() {
        return Err(format!(
            "Refusing to remove directory `{}`.",
            shortcut_path.display()
        ));
    }

    fs::remove_file(shortcut_path).map_err(|err| {
        format!(
            "Could not remove Desktop shortcut `{}`: {err}",
            shortcut_path.display()
        )
    })?;
    Ok(ShortcutOutcome::Removed(shortcut_path.to_path_buf()))
}

fn create_windows_shortcut_at(
    app: AppId,
    target_path: &Path,
    shortcut_path: &Path,
) -> Result<ShortcutOutcome, String> {
    #[cfg(target_os = "windows")]
    {
        use papercraft_tablet::windows::{WindowsShortcutSpec, create_desktop_shortcut};

        let description = match app {
            AppId::PaperCraft => "PaperCraft notebook and handwriting studio",
            AppId::PaperCraftServer => "PaperCraft self-hosted sync server control",
        };
        create_desktop_shortcut(WindowsShortcutSpec {
            shortcut_path,
            target_path,
            working_dir: target_path.parent().filter(|p| !p.as_os_str().is_empty()),
            description,
            icon_path: Some(target_path),
        })?;
        Ok(ShortcutOutcome::CreatedOrUpdated(
            shortcut_path.to_path_buf(),
        ))
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (app, target_path, shortcut_path);
        Err("Windows .lnk shortcuts can only be created when running on Windows.".to_string())
    }
}

fn create_linux_shortcut_at(
    app: AppId,
    target_path: &Path,
    shortcut_path: &Path,
    icons_dir: Option<&Path>,
) -> Result<ShortcutOutcome, String> {
    #[cfg(any(target_os = "linux", test))]
    {
        let icon_ref = match icons_dir {
            Some(dir) => match install_linux_icon(app, dir) {
                Ok(icon_path) => icon_path.to_string_lossy().into_owned(),
                Err(err) => {
                    log::warn!("Could not write Desktop icon PNG cleanly: {err}");
                    app.manifest_key().to_string()
                }
            },
            None => app.manifest_key().to_string(),
        };
        let contents = render_linux_desktop_entry(app, target_path, &icon_ref);
        write_atomic_executable_file(shortcut_path, contents.as_bytes())?;
        Ok(ShortcutOutcome::CreatedOrUpdated(
            shortcut_path.to_path_buf(),
        ))
    }

    #[cfg(not(any(target_os = "linux", test)))]
    {
        let _ = (app, target_path, shortcut_path, icons_dir);
        Err("Linux .desktop shortcuts can only be created when running on Linux.".to_string())
    }
}

#[cfg(any(target_os = "linux", test))]
fn install_linux_icon(app: AppId, icons_dir: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(icons_dir).map_err(|err| {
        format!(
            "Could not create icon folder `{}`: {err}",
            icons_dir.display()
        )
    })?;
    let dest = icons_dir.join(format!("{}.png", app.manifest_key()));
    write_atomic_bytes(&dest, APP_ICON_PNG, false)?;
    Ok(dest)
}

#[cfg(any(target_os = "linux", test))]
fn write_atomic_executable_file(dest: &Path, bytes: &[u8]) -> Result<(), String> {
    write_atomic_bytes(dest, bytes, true)
}

#[cfg(any(target_os = "linux", test))]
fn write_atomic_bytes(dest: &Path, bytes: &[u8], executable: bool) -> Result<(), String> {
    let parent = dest
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| format!("Invalid destination path `{}`.", dest.display()))?;
    let file_name = dest
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("shortcut");
    let pid = std::process::id();
    let tmp = parent.join(format!(".{file_name}.tmp.{pid}"));

    if let Err(err) = fs::write(&tmp, bytes) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Could not write `{}`: {err}", dest.display()));
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if executable { 0o755 } else { 0o644 };
        let perms = fs::Permissions::from_mode(mode);
        if let Err(err) = fs::set_permissions(&tmp, perms) {
            let _ = fs::remove_file(&tmp);
            return Err(format!(
                "Could not set permissions on `{}`: {err}",
                dest.display()
            ));
        }
    }
    #[cfg(not(unix))]
    let _ = executable;

    if let Err(err) = fs::rename(&tmp, dest) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("Could not finalize `{}`: {err}", dest.display()));
    }
    Ok(())
}

fn create_macos_shortcut_at(
    target_path: &Path,
    shortcut_path: &Path,
) -> Result<ShortcutOutcome, String> {
    #[cfg(unix)]
    {
        match fs::symlink_metadata(shortcut_path) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    fs::remove_file(shortcut_path).map_err(|err| {
                        format!(
                            "Could not replace existing Desktop symlink `{}`: {err}",
                            shortcut_path.display()
                        )
                    })?;
                } else if meta.is_dir() {
                    return Ok(ShortcutOutcome::AlreadyOnDesktop(
                        shortcut_path.to_path_buf(),
                    ));
                } else {
                    return Err(format!(
                        "A file already exists at `{}`; move it aside to create the Desktop alias.",
                        shortcut_path.display()
                    ));
                }
            }
            Err(err) if err.kind() == ErrorKind::NotFound => {}
            Err(err) => {
                return Err(format!(
                    "Could not inspect `{}`: {err}",
                    shortcut_path.display()
                ));
            }
        }

        std::os::unix::fs::symlink(target_path, shortcut_path).map_err(|err| {
            format!(
                "Could not create Desktop symlink `{}` -> `{}`: {err}",
                shortcut_path.display(),
                target_path.display()
            )
        })?;
        Ok(ShortcutOutcome::CreatedOrUpdated(
            shortcut_path.to_path_buf(),
        ))
    }

    #[cfg(not(unix))]
    {
        let _ = (target_path, shortcut_path);
        Err("macOS Desktop symlinks can only be created on Unix hosts.".to_string())
    }
}

#[cfg(target_os = "linux")]
fn mark_linux_desktop_trusted(desktop_file: &Path) {
    use std::process::{Command, Stdio};
    let _ = Command::new("gio")
        .arg("set")
        .arg(desktop_file)
        .arg("metadata::trusted")
        .arg("true")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
}

fn escape_desktop_string(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

fn format_desktop_exec_value(raw_exec: &str) -> String {
    let needs_quotes =
        raw_exec.is_empty() || raw_exec.chars().any(|c| RESERVED_EXEC_CHARS.contains(&c));
    let mut exec_arg = String::with_capacity(raw_exec.len() + 8);
    if needs_quotes {
        exec_arg.push('"');
        for ch in raw_exec.chars() {
            match ch {
                '"' | '`' | '$' | '\\' => {
                    exec_arg.push('\\');
                    exec_arg.push(ch);
                }
                '%' => exec_arg.push_str("%%"),
                _ => exec_arg.push(ch),
            }
        }
        exec_arg.push('"');
    } else {
        for ch in raw_exec.chars() {
            if ch == '%' {
                exec_arg.push_str("%%");
            } else {
                exec_arg.push(ch);
            }
        }
    }
    escape_desktop_string(&exec_arg)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    struct TempDirGuard(PathBuf);
    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir(label: &str) -> TempDirGuard {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let p = std::env::temp_dir().join(format!(
            "pc_upd_shortcut_{label}_{}_{nanos}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        TempDirGuard(p)
    }

    #[test]
    fn resolves_platform_launch_targets_and_shortcut_names() {
        let linux_exe = Path::new("/tmp/.mount_PC123/usr/bin/papercraft");
        let appimage = Path::new("/home/alex/Apps/PaperCraft.AppImage");
        assert_eq!(
            resolve_launch_target_for(linux_exe, Some(appimage), "linux"),
            appimage
        );
        assert_eq!(
            resolve_launch_target_for(linux_exe, Some(Path::new("")), "linux"),
            linux_exe
        );

        let mac_exe = Path::new("/Applications/PaperCraft.app/Contents/MacOS/papercraft");
        assert_eq!(
            resolve_launch_target_for(mac_exe, None, "macos"),
            Path::new("/Applications/PaperCraft.app")
        );

        let win_exe = Path::new("D:\\Apps\\PaperCraft\\papercraft.exe");
        assert_eq!(resolve_launch_target_for(win_exe, None, "windows"), win_exe);

        let desktop = Path::new("/tmp/Desktop");
        assert_eq!(
            shortcut_path_in_dir(AppId::PaperCraft, win_exe, desktop, "windows"),
            desktop.join("PaperCraft.lnk")
        );
        assert_eq!(
            shortcut_path_in_dir(AppId::PaperCraftServer, win_exe, desktop, "windows"),
            desktop.join("PaperCraft Server.lnk")
        );
        assert_eq!(
            shortcut_path_in_dir(AppId::PaperCraft, appimage, desktop, "linux"),
            desktop.join("papercraft.desktop")
        );
        assert_eq!(
            shortcut_path_in_dir(AppId::PaperCraftServer, appimage, desktop, "linux"),
            desktop.join("papercraft-server.desktop")
        );
        assert_eq!(
            shortcut_path_in_dir(AppId::PaperCraft, mac_exe, desktop, "macos"),
            desktop.join("PaperCraft.app")
        );
        assert_eq!(
            shortcut_path_in_dir(
                AppId::PaperCraftServer,
                Path::new(
                    "/Users/alex/Apps/PaperCraft Server.app/Contents/MacOS/papercraft-server"
                ),
                desktop,
                "macos",
            ),
            desktop.join("PaperCraft Server.app")
        );
    }

    #[test]
    fn parses_xdg_user_dirs_desktop_cleanly() {
        let home = Path::new("/home/alex");
        let cfg = r#"
# Written by xdg-user-dirs-update
XDG_DOWNLOAD_DIR="$HOME/Downloads"
XDG_DESKTOP_DIR="$HOME/Área de Trabalho"
"#;
        assert_eq!(
            parse_xdg_user_dirs_desktop(cfg, home),
            Some(home.join("Área de Trabalho"))
        );

        let custom = "export XDG_DESKTOP_DIR='/mnt/fast/Desktop'\n";
        assert_eq!(
            parse_xdg_user_dirs_desktop(custom, home),
            Some(PathBuf::from("/mnt/fast/Desktop"))
        );

        let disabled = "XDG_DESKTOP_DIR=\"$HOME/\"\n";
        assert_eq!(parse_xdg_user_dirs_desktop(disabled, home), None);
    }

    #[test]
    fn renders_linux_desktop_entry_for_unix_paths_with_spaces() {
        let entry = render_linux_desktop_entry(
            AppId::PaperCraft,
            Path::new("/home/alex/My Apps/PaperCraft.AppImage"),
            "/home/alex/.local/share/icons/hicolor/256x256/apps/papercraft.png",
        );
        assert!(entry.contains("Exec=\"/home/alex/My Apps/PaperCraft.AppImage\"\n"));
        assert!(entry.contains("Path=/home/alex/My Apps\n"));
        assert!(
            entry.contains(
                "Icon=/home/alex/.local/share/icons/hicolor/256x256/apps/papercraft.png\n"
            )
        );
    }

    #[test]
    fn creates_updates_and_removes_linux_desktop_entry_and_icon_in_temp_dir() {
        let tmp = temp_dir("linux_desktop");
        let desktop = tmp.0.join("Desktop");
        let icons = tmp.0.join("icons");
        let appimage = tmp.0.join("My Apps").join("PaperCraft-0.2.4.AppImage");
        fs::create_dir_all(appimage.parent().unwrap()).unwrap();
        fs::write(&appimage, b"stub").unwrap();

        let created = sync_desktop_shortcut_in_dir(
            AppId::PaperCraft,
            &appimage,
            &desktop,
            Some(&icons),
            true,
            "linux",
        )
        .unwrap();
        let desktop_file = desktop.join("papercraft.desktop");
        assert_eq!(
            created,
            ShortcutOutcome::CreatedOrUpdated(desktop_file.clone())
        );

        let icon_file = icons.join("papercraft.png");
        assert!(icon_file.is_file());
        let text = fs::read_to_string(&desktop_file).unwrap();
        assert!(text.contains("Name=PaperCraft\n"));
        assert!(text.contains("StartupWMClass=papercraft\n"));
        assert!(text.contains(&format!(
            "Exec={}\n",
            format_desktop_exec_value(&appimage.to_string_lossy())
        )));
        assert!(text.contains(&format!(
            "Icon={}\n",
            escape_desktop_string(&icon_file.to_string_lossy())
        )));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&desktop_file).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o755);
        }

        let removed = sync_desktop_shortcut_in_dir(
            AppId::PaperCraft,
            &appimage,
            &desktop,
            Some(&icons),
            false,
            "linux",
        )
        .unwrap();
        assert_eq!(removed, ShortcutOutcome::Removed(desktop_file.clone()));
        assert!(!desktop_file.exists());

        let absent = sync_desktop_shortcut_in_dir(
            AppId::PaperCraft,
            &appimage,
            &desktop,
            Some(&icons),
            false,
            "linux",
        )
        .unwrap();
        assert_eq!(absent, ShortcutOutcome::AlreadyAbsent(desktop_file));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn creates_updates_and_removes_windows_lnk_in_temp_dir() {
        let tmp = temp_dir("win_lnk");
        let desktop = tmp.0.join("Desktop");
        let app_dir = tmp.0.join("Portable");
        fs::create_dir_all(&app_dir).unwrap();
        let exe = app_dir.join("papercraft-server.exe");
        fs::write(&exe, b"stub").unwrap();

        let created = sync_desktop_shortcut_in_dir(
            AppId::PaperCraftServer,
            &exe,
            &desktop,
            None,
            true,
            "windows",
        )
        .unwrap();
        let lnk = desktop.join("PaperCraft Server.lnk");
        assert_eq!(created, ShortcutOutcome::CreatedOrUpdated(lnk.clone()));
        assert!(lnk.is_file());

        let removed = sync_desktop_shortcut_in_dir(
            AppId::PaperCraftServer,
            &exe,
            &desktop,
            None,
            false,
            "windows",
        )
        .unwrap();
        assert_eq!(removed, ShortcutOutcome::Removed(lnk.clone()));
        assert!(!lnk.exists());
    }

    #[cfg(unix)]
    #[test]
    fn macos_symlink_sync_protects_real_app_bundle_on_desktop() {
        let tmp = temp_dir("macos_symlink");
        let desktop = tmp.0.join("Desktop");
        let apps = tmp.0.join("Apps");
        let bundle = apps.join("PaperCraft.app");
        let exe = bundle.join("Contents").join("MacOS").join("papercraft");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::write(&exe, b"stub").unwrap();

        let created =
            sync_desktop_shortcut_in_dir(AppId::PaperCraft, &bundle, &desktop, None, true, "macos")
                .unwrap();
        let link_path = desktop.join("PaperCraft.app");
        assert_eq!(
            created,
            ShortcutOutcome::CreatedOrUpdated(link_path.clone())
        );
        assert!(
            fs::symlink_metadata(&link_path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(fs::read_link(&link_path).unwrap(), bundle);

        let removed = sync_desktop_shortcut_in_dir(
            AppId::PaperCraft,
            &bundle,
            &desktop,
            None,
            false,
            "macos",
        )
        .unwrap();
        assert_eq!(removed, ShortcutOutcome::Removed(link_path.clone()));
        assert!(fs::symlink_metadata(&link_path).is_err());

        // Real bundle placed directly on Desktop must never be overwritten or deleted.
        let desktop_bundle = desktop.join("PaperCraft.app");
        fs::create_dir_all(desktop_bundle.join("Contents").join("MacOS")).unwrap();
        let on_desktop = sync_desktop_shortcut_in_dir(
            AppId::PaperCraft,
            &desktop_bundle,
            &desktop,
            None,
            true,
            "macos",
        )
        .unwrap();
        assert_eq!(
            on_desktop,
            ShortcutOutcome::AlreadyOnDesktop(desktop_bundle.clone())
        );

        let disable_real = sync_desktop_shortcut_in_dir(
            AppId::PaperCraft,
            &desktop_bundle,
            &desktop,
            None,
            false,
            "macos",
        )
        .unwrap();
        assert_eq!(
            disable_real,
            ShortcutOutcome::AlreadyOnDesktop(desktop_bundle.clone())
        );
        assert!(desktop_bundle.is_dir());
    }
}
