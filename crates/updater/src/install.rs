//! Platform-aware binary staging verification, atomic self-replacement, and relaunch.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::UpdateError;
use crate::manifest::{AppId, AssetFormat};
use crate::version::SemVer;

/// Whether the current executable location supports in-place self-replacement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallCapability {
    Supported {
        target_path: PathBuf,
        staging_dir: PathBuf,
    },
    ReadOnly {
        target_path: PathBuf,
        reason: String,
    },
    UnsupportedLayout {
        reason: String,
    },
}

/// Verified downloaded update staged on disk and ready to swap on restart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StagedUpdate {
    pub app: AppId,
    pub version: SemVer,
    pub staged_path: PathBuf,
    pub target_path: PathBuf,
    pub format: AssetFormat,
    pub sha256: String,
}

/// Probes whether this running binary can replace itself in place for `format`.
#[must_use]
pub fn probe_install_capability(app: AppId, format: AssetFormat) -> InstallCapability {
    let Ok(exe) = std::env::current_exe() else {
        return InstallCapability::UnsupportedLayout {
            reason: "Could not locate the running executable path.".to_string(),
        };
    };
    let appimage_env = std::env::var_os("APPIMAGE").map(PathBuf::from);
    probe_install_capability_for(
        app,
        format,
        &exe,
        appimage_env.as_deref(),
        std::env::consts::OS,
    )
}

/// Pure helper allowing deterministic cross-platform layout tests.
#[must_use]
pub fn probe_install_capability_for(
    app: AppId,
    format: AssetFormat,
    current_exe: &Path,
    appimage_env: Option<&Path>,
    target_os: &str,
) -> InstallCapability {
    let Some(target_path) = resolve_target_path(app, format, current_exe, appimage_env, target_os)
    else {
        return unsupported_layout_for(format, target_os);
    };

    let Some(staging_dir) = target_path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
    else {
        return InstallCapability::UnsupportedLayout {
            reason: format!(
                "Could not determine a parent folder for `{}`.",
                target_path.display()
            ),
        };
    };

    match check_dir_writable(&staging_dir) {
        Ok(()) => InstallCapability::Supported {
            target_path,
            staging_dir,
        },
        Err(err) => InstallCapability::ReadOnly {
            target_path,
            reason: format!(
                "Folder `{}` is not writable ({err}). Install from the release page or move the app to a writable folder.",
                staging_dir.display()
            ),
        },
    }
}

fn unsupported_layout_for(format: AssetFormat, target_os: &str) -> InstallCapability {
    let reason = match (target_os, format) {
        ("macos", AssetFormat::MacOsAppTarGz) => {
            "In-app updates on macOS require running from the .app bundle. Download the latest release from GitHub."
                .to_string()
        }
        _ => format!(
            "Automatic replacement is not supported for {format:?} on {target_os}. Download the update from GitHub."
        ),
    };
    InstallCapability::UnsupportedLayout { reason }
}

fn resolve_target_path(
    _app: AppId,
    format: AssetFormat,
    current_exe: &Path,
    appimage_env: Option<&Path>,
    target_os: &str,
) -> Option<PathBuf> {
    match (target_os, format) {
        ("windows", AssetFormat::Exe | AssetFormat::Binary) => Some(current_exe.to_path_buf()),
        ("linux", AssetFormat::AppImage | AssetFormat::Binary) => {
            if let Some(img) = appimage_env.filter(|p| !p.as_os_str().is_empty()) {
                Some(img.to_path_buf())
            } else {
                Some(current_exe.to_path_buf())
            }
        }
        ("macos", AssetFormat::MacOsAppTarGz) => macos_app_bundle_root(current_exe),
        ("macos", AssetFormat::Binary) => Some(current_exe.to_path_buf()),
        _ => None,
    }
}

/// Returns `<Name>.app` when `exe_path` points to `<Name>.app/Contents/MacOS/<bin>`.
#[must_use]
pub fn macos_app_bundle_root(exe_path: &Path) -> Option<PathBuf> {
    let macos_dir = exe_path.parent()?;
    let contents_dir = macos_dir.parent()?;
    let bundle_dir = contents_dir.parent()?;
    let is_bundle = macos_dir.file_name().and_then(|n| n.to_str()) == Some("MacOS")
        && contents_dir.file_name().and_then(|n| n.to_str()) == Some("Contents")
        && bundle_dir
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("app"));
    is_bundle.then(|| bundle_dir.to_path_buf())
}

fn check_dir_writable(dir: &Path) -> Result<(), String> {
    let pid = std::process::id();
    let probe_path = dir.join(format!(".papercraft-update-probe-{pid}"));
    let _ = fs::remove_file(&probe_path);
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe_path)
    {
        Ok(file) => {
            drop(file);
            let _ = fs::remove_file(&probe_path);
            Ok(())
        }
        Err(err) => Err(err.to_string()),
    }
}

/// Best-effort startup cleanup for backup `.old` and interrupted `.update.part` / `.update.stage` files.
pub fn cleanup_stale_files(app: AppId) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let appimage_env = std::env::var_os("APPIMAGE").map(PathBuf::from);
    let target = if let Some(img) = appimage_env.filter(|p| !p.as_os_str().is_empty()) {
        img
    } else if let Some(bundle) = macos_app_bundle_root(&exe) {
        bundle
    } else {
        exe
    };
    cleanup_stale_files_near_target(&target, app);
}

/// Removes stale `.old`, `.update.part`, and `.update.stage` artifacts next to `target_path`.
pub fn cleanup_stale_files_near_target(target_path: &Path, app: AppId) {
    let Some(dir) = target_path.parent().filter(|p| !p.as_os_str().is_empty()) else {
        return;
    };
    if let Some(target_name) = target_path.file_name().and_then(|n| n.to_str()) {
        let old_primary = dir.join(format!("{target_name}.old"));
        remove_path_best_effort(&old_primary);
        for idx in 1..=5 {
            let indexed = dir.join(format!("{target_name}.old.{idx}"));
            remove_path_best_effort(&indexed);
        }
    }

    let prefix = app.manifest_key();
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let is_app_staging = name.starts_with(prefix)
            && (name.ends_with(".update.part") || name.ends_with(".update.stage"));
        let is_temp_dir_or_probe = name.starts_with(".papercraft-update-stage-")
            || name.starts_with(".papercraft-update-probe-");
        if is_app_staging || is_temp_dir_or_probe {
            remove_path_best_effort(&path);
        }
    }
}

fn remove_path_best_effort(path: &Path) {
    let Ok(meta) = fs::symlink_metadata(path) else {
        return;
    };
    if meta.is_dir() {
        let _ = fs::remove_dir_all(path);
    } else {
        let _ = fs::remove_file(path);
    }
}

/// Swaps the staged update onto `target_path` using the OS-safe replacement strategy.
pub fn replace_binary_on_disk(
    staged_path: &Path,
    target_path: &Path,
    format: AssetFormat,
    app: AppId,
) -> Result<PathBuf, UpdateError> {
    if !staged_path.is_file() {
        return Err(UpdateError::Io(format!(
            "staged update file `{}` is missing",
            staged_path.display()
        )));
    }

    if format == AssetFormat::MacOsAppTarGz {
        return replace_macos_app_bundle(staged_path, target_path, app);
    }

    #[cfg(target_os = "windows")]
    {
        let _ = app;
        replace_windows_executable(staged_path, target_path)?;
        Ok(target_path.to_path_buf())
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = app;
        set_unix_executable_permissions(staged_path)?;
        fs::rename(staged_path, target_path).map_err(|err| {
            UpdateError::Io(format!(
                "could not replace `{}` with `{}`: {err}",
                target_path.display(),
                staged_path.display()
            ))
        })?;
        Ok(target_path.to_path_buf())
    }
}

#[cfg(any(target_os = "windows", test))]
fn choose_free_backup_path(target_path: &Path) -> Result<PathBuf, UpdateError> {
    let target_name = target_path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| {
            UpdateError::Io(format!(
                "invalid target executable path `{}`",
                target_path.display()
            ))
        })?;
    let primary = target_path.with_file_name(format!("{target_name}.old"));
    if !primary.exists() || remove_if_not_locked(&primary) {
        return Ok(primary);
    }
    for idx in 1..=32 {
        let candidate = target_path.with_file_name(format!("{target_name}.old.{idx}"));
        if !candidate.exists() || remove_if_not_locked(&candidate) {
            return Ok(candidate);
        }
    }
    Err(UpdateError::Io(
        "could not allocate a backup filename next to the executable".to_string(),
    ))
}

#[cfg(any(target_os = "windows", test))]
fn remove_if_not_locked(path: &Path) -> bool {
    if let Ok(meta) = fs::symlink_metadata(path) {
        if meta.is_dir() {
            fs::remove_dir_all(path).is_ok()
        } else {
            fs::remove_file(path).is_ok()
        }
    } else {
        true
    }
}

#[cfg(any(target_os = "windows", test))]
fn replace_windows_executable(staged_path: &Path, target_path: &Path) -> Result<(), UpdateError> {
    let backup_path = choose_free_backup_path(target_path)?;
    if target_path.exists() {
        fs::rename(target_path, &backup_path).map_err(|err| {
            UpdateError::Io(format!(
                "could not move current executable `{}` aside: {err}",
                target_path.display()
            ))
        })?;
    }

    if let Err(err) = fs::rename(staged_path, target_path) {
        if backup_path.exists() {
            let _ = fs::rename(&backup_path, target_path);
        }
        return Err(UpdateError::Io(format!(
            "could not install new executable `{}`: {err}",
            target_path.display()
        )));
    }
    Ok(())
}

#[cfg(unix)]
fn set_unix_executable_permissions(path: &Path) -> Result<(), UpdateError> {
    use std::os::unix::fs::PermissionsExt;

    let perms = fs::Permissions::from_mode(0o755);
    fs::set_permissions(path, perms).map_err(|err| {
        UpdateError::Io(format!(
            "could not mark `{}` executable: {err}",
            path.display()
        ))
    })
}

fn replace_macos_app_bundle(
    archive_path: &Path,
    target_bundle: &Path,
    app: AppId,
) -> Result<PathBuf, UpdateError> {
    let parent_dir = target_bundle
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| {
            UpdateError::Io(format!(
                "could not resolve parent directory for `{}`",
                target_bundle.display()
            ))
        })?;
    let stage_dir = parent_dir.join(format!(".papercraft-update-stage-{}", std::process::id()));
    let _ = fs::remove_dir_all(&stage_dir);
    fs::create_dir_all(&stage_dir).map_err(|err| {
        UpdateError::Io(format!(
            "could not create staging directory `{}`: {err}",
            stage_dir.display()
        ))
    })?;

    let res = extract_and_swap_bundle(archive_path, &stage_dir, target_bundle, app);
    let _ = fs::remove_dir_all(&stage_dir);
    if res.is_ok() {
        let _ = fs::remove_file(archive_path);
    }
    res
}

fn extract_and_swap_bundle(
    archive_path: &Path,
    stage_dir: &Path,
    target_bundle: &Path,
    app: AppId,
) -> Result<PathBuf, UpdateError> {
    let status = Command::new("/usr/bin/tar")
        .arg("-xzf")
        .arg(archive_path)
        .arg("-C")
        .arg(stage_dir)
        .status()
        .map_err(|err| UpdateError::Io(format!("failed to invoke tar for extraction: {err}")))?;
    if !status.success() {
        return Err(UpdateError::Io(format!(
            "failed to extract update archive (tar exited with {status})"
        )));
    }

    let extracted_bundle = stage_dir.join(app.macos_app_bundle_name());
    let meta = fs::symlink_metadata(&extracted_bundle).map_err(|_| {
        UpdateError::InvalidManifest(format!(
            "archive did not contain `{}`",
            app.macos_app_bundle_name()
        ))
    })?;
    if meta.file_type().is_symlink() || !meta.is_dir() {
        return Err(UpdateError::InvalidManifest(format!(
            "extracted `{}` is not a valid application bundle",
            app.macos_app_bundle_name()
        )));
    }

    let expected_exe = extracted_bundle
        .join("Contents")
        .join("MacOS")
        .join(app.executable_name());
    if !expected_exe.is_file() {
        return Err(UpdateError::InvalidManifest(format!(
            "extracted bundle is missing `{}`",
            app.executable_name()
        )));
    }

    let backup_path = target_bundle.with_file_name(format!(
        "{}.old",
        target_bundle
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(app.macos_app_bundle_name())
    ));
    let _ = fs::remove_dir_all(&backup_path);
    if target_bundle.exists() {
        fs::rename(target_bundle, &backup_path).map_err(|err| {
            UpdateError::Io(format!(
                "could not move existing bundle `{}` aside: {err}",
                target_bundle.display()
            ))
        })?;
    }

    if let Err(err) = fs::rename(&extracted_bundle, target_bundle) {
        if backup_path.exists() {
            let _ = fs::rename(&backup_path, target_bundle);
        }
        return Err(UpdateError::Io(format!(
            "could not move new bundle into place at `{}`: {err}",
            target_bundle.display()
        )));
    }

    let _ = fs::remove_dir_all(&backup_path);
    Ok(target_bundle.to_path_buf())
}

/// Launches the newly replaced binary or bundle as a detached child process.
pub fn spawn_relaunched_app(launch_path: &Path, format: AssetFormat) -> Result<(), UpdateError> {
    #[cfg(target_os = "macos")]
    {
        if format == AssetFormat::MacOsAppTarGz {
            Command::new("open")
                .arg("-n")
                .arg(launch_path)
                .spawn()
                .map_err(|err| UpdateError::RelaunchFailed(err.to_string()))?;
            return Ok(());
        }
    }

    let mut cmd = Command::new(launch_path);
    #[cfg(target_os = "linux")]
    {
        let _ = format;
        // Clear AppImage mount environment variables so the newly launched AppImage mounts itself fresh.
        cmd.env_remove("APPIMAGE")
            .env_remove("APPDIR")
            .env_remove("ARGV0")
            .env_remove("OWD");
    }
    #[cfg(not(target_os = "linux"))]
    let _ = format;

    cmd.spawn()
        .map_err(|err| UpdateError::RelaunchFailed(err.to_string()))?;
    Ok(())
}

/// Installs `staged` in place and launches the updated process.
pub fn apply_staged_update_and_relaunch(staged: &StagedUpdate) -> Result<(), UpdateError> {
    let launch_path = replace_binary_on_disk(
        &staged.staged_path,
        &staged.target_path,
        staged.format,
        staged.app,
    )?;
    spawn_relaunched_app(&launch_path, staged.format)
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
        let p = std::env::temp_dir().join(format!("pc_upd_inst_{label}_{nanos}"));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        TempDirGuard(p)
    }

    #[test]
    fn resolves_platform_layouts_and_macos_bundle_roots() {
        let tmp = temp_dir("layout");
        let win_exe = tmp.0.join("papercraft.exe");
        assert!(matches!(
            probe_install_capability_for(
                AppId::PaperCraft,
                AssetFormat::Exe,
                &win_exe,
                None,
                "windows"
            ),
            InstallCapability::Supported { .. }
        ));

        let linux_exe = Path::new("/tmp/.mount_PC123/usr/bin/papercraft");
        let appimage_file = tmp.0.join("papercraft-linux-x86_64.AppImage");
        let cap = probe_install_capability_for(
            AppId::PaperCraft,
            AssetFormat::AppImage,
            linux_exe,
            Some(&appimage_file),
            "linux",
        );
        match cap {
            InstallCapability::Supported { target_path, .. } => {
                assert_eq!(target_path, appimage_file);
            }
            other => panic!("expected Supported, got {other:?}"),
        }

        let mac_loose = Path::new("/tmp/papercraft");
        assert!(matches!(
            probe_install_capability_for(
                AppId::PaperCraft,
                AssetFormat::MacOsAppTarGz,
                mac_loose,
                None,
                "macos"
            ),
            InstallCapability::UnsupportedLayout { .. }
        ));

        let mac_bundle_exe = tmp
            .0
            .join("PaperCraft.app")
            .join("Contents")
            .join("MacOS")
            .join("papercraft");
        let cap_mac = probe_install_capability_for(
            AppId::PaperCraft,
            AssetFormat::MacOsAppTarGz,
            &mac_bundle_exe,
            None,
            "macos",
        );
        match cap_mac {
            InstallCapability::Supported { target_path, .. } => {
                assert_eq!(target_path, tmp.0.join("PaperCraft.app"));
            }
            other => panic!("expected Supported bundle, got {other:?}"),
        }
    }

    #[test]
    fn windows_swap_moves_current_to_old_and_cleanup_removes_stale_files() {
        let tmp = temp_dir("swap");
        let target = tmp.0.join("papercraft.exe");
        let staged = tmp.0.join("papercraft-windows-x86_64.exe.update.stage");
        fs::write(&target, b"old-binary-v1").unwrap();
        fs::write(&staged, b"new-binary-v2").unwrap();

        replace_windows_executable(&staged, &target).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new-binary-v2");
        assert!(!staged.exists());
        let backup = tmp.0.join("papercraft.exe.old");
        assert_eq!(fs::read(&backup).unwrap(), b"old-binary-v1");

        let partial = tmp.0.join("papercraft-windows-x86_64.exe.update.part");
        fs::write(&partial, b"partial").unwrap();

        cleanup_stale_files_near_target(&target, AppId::PaperCraft);
        assert!(!backup.exists());
        assert!(!partial.exists());
        assert_eq!(fs::read(&target).unwrap(), b"new-binary-v2");
    }
}
