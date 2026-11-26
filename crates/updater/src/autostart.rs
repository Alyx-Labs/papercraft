//! Cross-platform per-user OS autostart (launch at login) registration and removal.

use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use crate::install::macos_app_bundle_root;
use crate::manifest::{AppId, PUBLISHER_NAME};
#[cfg(target_os = "linux")]
use crate::shortcut::resolve_linux_icons_dir;
use crate::shortcut::{
    absolutize_path, escape_desktop_string, format_desktop_exec_value, resolve_launch_target_for,
    write_atomic_bytes,
};

/// Command-line flag passed when the OS launches the app automatically at user sign-in.
pub const AUTOSTART_ARG: &str = "--autostart";

/// Per-user Windows startup registry key (`HKEY_CURRENT_USER\...\Run`).
pub const WINDOWS_RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Per-user Windows Task Manager Startup Apps enable/disable override registry key.
pub const WINDOWS_STARTUP_APPROVED_RUN_SUBKEY: &str =
    r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

/// Outcome of synchronizing OS login autostart configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AutostartOutcome {
    Enabled(String),
    Disabled(String),
    AlreadyDisabled(String),
}

impl AutostartOutcome {
    /// Returns the registry location or autostart file path acted upon.
    #[must_use]
    pub fn target(&self) -> &str {
        match self {
            Self::Enabled(t) | Self::Disabled(t) | Self::AlreadyDisabled(t) => t,
        }
    }

    /// Returns a concise, plain-English status string suitable for UI feedback labels.
    #[must_use]
    pub fn status_message(&self) -> String {
        match self {
            Self::Enabled(target) => format!("Startup launch enabled ({target})."),
            Self::Disabled(target) => format!("Startup launch disabled ({target})."),
            Self::AlreadyDisabled(_) => "Startup launch is already turned off.".to_string(),
        }
    }
}

/// Returns a concise, plain-English description of how PaperCraft registers login autostart on this OS.
#[must_use]
pub const fn autostart_architecture_hint() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "Registers PaperCraft Server under your Windows user startup apps (Task Manager -> Startup apps) to start automatically when you sign in."
    }
    #[cfg(target_os = "macos")]
    {
        "Creates a per-user LaunchAgent in ~/Library/LaunchAgents so PaperCraft Server starts automatically when you sign in."
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        "Creates a Freedesktop autostart entry in ~/.config/autostart so PaperCraft Server starts automatically when you sign in."
    }
}

/// Formats a Windows `HKCU\...\Run` command line with quoted executable path and safe arguments.
pub fn format_windows_run_command(target_path: &Path, args: &[&str]) -> Result<String, String> {
    let raw_target = target_path.to_string_lossy();
    let trimmed = raw_target.trim();
    if trimmed.is_empty() {
        return Err("Startup executable path must not be empty.".to_string());
    }
    if raw_target
        .chars()
        .any(|c| matches!(c, '\0' | '\r' | '\n' | '"') || c.is_control())
    {
        return Err("Startup executable path contains invalid characters.".to_string());
    }

    let mut command = format!("\"{raw_target}\"");
    for &arg in args {
        if arg.is_empty()
            || arg
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || c == '"')
        {
            return Err(format!("Invalid startup launch argument `{arg}`."));
        }
        command.push(' ');
        command.push_str(arg);
    }
    Ok(command)
}

/// Resolves the inner binary inside a macOS `.app` bundle (or returns `target_path` unchanged).
#[must_use]
pub fn resolve_macos_launch_agent_program(app: AppId, target_path: &Path) -> PathBuf {
    let is_app_bundle = target_path
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("app"));
    if is_app_bundle {
        target_path
            .join("Contents")
            .join("MacOS")
            .join(app.manifest_key())
    } else {
        target_path.to_path_buf()
    }
}

/// Renders a per-user macOS LaunchAgent `.plist` launching `program_path` with `args` at login.
pub fn render_macos_launch_agent_plist(
    app: AppId,
    program_path: &Path,
    args: &[&str],
) -> Result<String, String> {
    let program_raw = program_path.to_string_lossy();
    if program_raw.trim().is_empty() {
        return Err("LaunchAgent program path must not be empty.".to_string());
    }
    let label = escape_xml_text(app.bundle_identifier())?;
    let program = escape_xml_text(&program_raw)?;
    let mut extra_args = String::new();
    for &arg in args {
        if arg.is_empty() {
            return Err("LaunchAgent argument must not be empty.".to_string());
        }
        let escaped = escape_xml_text(arg)?;
        extra_args.push_str(&format!("\t\t<string>{escaped}</string>\n"));
    }

    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \t<key>Label</key>\n\
         \t<string>{label}</string>\n\
         \t<key>AssociatedBundleIdentifiers</key>\n\
         \t<array>\n\
         \t\t<string>{label}</string>\n\
         \t</array>\n\
         \t<key>ProgramArguments</key>\n\
         \t<array>\n\
         \t\t<string>{program}</string>\n\
         {extra_args}\
         \t</array>\n\
         \t<key>RunAtLoad</key>\n\
         \t<true/>\n\
         \t<key>KeepAlive</key>\n\
         \t<false/>\n\
         \t<key>ProcessType</key>\n\
         \t<string>Interactive</string>\n\
         \t<key>LimitLoadToSessionType</key>\n\
         \t<string>Aqua</string>\n\
         </dict>\n\
         </plist>\n"
    ))
}

/// Renders a Freedesktop XDG autostart `.desktop` entry launching `exec_path` with `args`.
#[must_use]
pub fn render_linux_autostart_entry(
    app: AppId,
    exec_path: &Path,
    args: &[&str],
    icon_ref: &str,
) -> String {
    let categories = match app {
        AppId::PaperCraft => "Office;Graphics;",
        AppId::PaperCraftServer => "Utility;Network;",
    };
    let mut exec_line = format_desktop_exec_value(&exec_path.to_string_lossy());
    for &arg in args {
        if !arg.trim().is_empty() {
            exec_line.push(' ');
            exec_line.push_str(&format_desktop_exec_value(arg));
        }
    }
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
         GenericName={generic}\n\
         Comment={comment}\n\
         Exec={exec_line}\n\
         Icon={icon_escaped}\n\
         Terminal=false\n\
         Categories={categories}\n\
         StartupNotify=false\n\
         StartupWMClass={wm_class}\n\
         X-Publisher={publisher}\n\
         X-GNOME-Autostart-enabled=true\n\
         X-KDE-autostart-after=panel\n",
        name = app.display_name(),
        generic = app.generic_name(),
        comment = app.summary(),
        wm_class = app.manifest_key(),
        publisher = PUBLISHER_NAME,
    );
    if !path_str.is_empty() {
        out.push_str(&format!("Path={path_str}\n"));
    }
    out
}

/// Synchronizes a Freedesktop autostart `.desktop` entry inside `autostart_dir`.
pub fn sync_linux_autostart_in_dir(
    app: AppId,
    target_path: &Path,
    autostart_dir: &Path,
    icons_dir: Option<&Path>,
    enabled: bool,
) -> Result<AutostartOutcome, String> {
    if target_path.as_os_str().is_empty() {
        return Err("Executable path is empty.".to_string());
    }
    let entry_path = autostart_dir.join(format!("{}.desktop", app.manifest_key()));
    if !enabled {
        return remove_autostart_file(&entry_path);
    }

    fs::create_dir_all(autostart_dir).map_err(|err| {
        format!(
            "Could not create autostart folder `{}`: {err}",
            autostart_dir.display()
        )
    })?;
    let icon_ref = resolve_autostart_icon_ref(app, icons_dir);
    let contents = render_linux_autostart_entry(app, target_path, &[AUTOSTART_ARG], &icon_ref);
    write_atomic_bytes(&entry_path, contents.as_bytes(), false)?;
    Ok(AutostartOutcome::Enabled(entry_path.display().to_string()))
}

/// Synchronizes a per-user macOS LaunchAgent `.plist` inside `launch_agents_dir`.
pub fn sync_macos_autostart_in_dir(
    app: AppId,
    target_path: &Path,
    launch_agents_dir: &Path,
    enabled: bool,
) -> Result<AutostartOutcome, String> {
    if target_path.as_os_str().is_empty() {
        return Err("Executable path is empty.".to_string());
    }
    let plist_path = launch_agents_dir.join(format!("{}.plist", app.bundle_identifier()));
    if !enabled {
        return remove_autostart_file(&plist_path);
    }

    fs::create_dir_all(launch_agents_dir).map_err(|err| {
        format!(
            "Could not create LaunchAgents folder `{}`: {err}",
            launch_agents_dir.display()
        )
    })?;
    let program = resolve_macos_launch_agent_program(app, target_path);
    let contents = render_macos_launch_agent_plist(app, &program, &[AUTOSTART_ARG])?;
    write_atomic_bytes(&plist_path, contents.as_bytes(), false)?;
    Ok(AutostartOutcome::Enabled(plist_path.display().to_string()))
}

/// Enables or disables login autostart for the currently running PaperCraft executable.
pub fn sync_autostart(app: AppId, enabled: bool) -> Result<AutostartOutcome, String> {
    sync_autostart_with_options(app, enabled, true)
}

/// Enables or disables login autostart for the currently running executable, optionally
/// clearing Windows Task Manager's per-app `StartupApproved` disabled override.
pub fn sync_autostart_with_options(
    app: AppId,
    enabled: bool,
    clear_os_disabled_override: bool,
) -> Result<AutostartOutcome, String> {
    let exe = std::env::current_exe()
        .map_err(|err| format!("Could not locate the running executable: {err}"))?;
    let appimage_env = std::env::var_os("APPIMAGE").map(PathBuf::from);
    let target = resolve_launch_target_for(&exe, appimage_env.as_deref(), std::env::consts::OS);
    sync_autostart_for_target(app, &target, enabled, clear_os_disabled_override)
}

/// Synchronizes OS login autostart for `app` pointing to `target_path`.
pub fn sync_autostart_for_target(
    app: AppId,
    target_path: &Path,
    enabled: bool,
    clear_os_disabled_override: bool,
) -> Result<AutostartOutcome, String> {
    let absolute_target = absolutize_path(target_path);
    let resolved_target = if cfg!(target_os = "macos") {
        macos_app_bundle_root(&absolute_target).unwrap_or(absolute_target)
    } else {
        absolute_target
    };

    #[cfg(target_os = "windows")]
    {
        sync_windows_autostart(app, &resolved_target, enabled, clear_os_disabled_override)
    }

    #[cfg(target_os = "macos")]
    {
        let _ = clear_os_disabled_override;
        let launch_agents_dir = resolve_macos_launch_agents_dir()?;
        sync_macos_autostart_in_dir(app, &resolved_target, &launch_agents_dir, enabled)
    }

    #[cfg(target_os = "linux")]
    {
        let _ = clear_os_disabled_override;
        let autostart_dir = resolve_linux_autostart_dir()?;
        let icons_dir = resolve_linux_icons_dir();
        sync_linux_autostart_in_dir(
            app,
            &resolved_target,
            &autostart_dir,
            icons_dir.as_deref(),
            enabled,
        )
    }

    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        let _ = (app, resolved_target, enabled, clear_os_disabled_override);
        Err("Automatic system startup is not supported on this operating system.".to_string())
    }
}

#[cfg(target_os = "windows")]
fn sync_windows_autostart(
    app: AppId,
    target_path: &Path,
    enabled: bool,
    clear_os_disabled_override: bool,
) -> Result<AutostartOutcome, String> {
    use papercraft_tablet::windows::{delete_hkcu_reg_value, set_hkcu_reg_sz};

    let value_name = app.display_name();
    let location = format!(r"HKCU\{WINDOWS_RUN_SUBKEY}\{value_name}");
    if enabled {
        let cmd = format_windows_run_command(target_path, &[AUTOSTART_ARG])?;
        set_hkcu_reg_sz(WINDOWS_RUN_SUBKEY, value_name, &cmd)?;
        if clear_os_disabled_override {
            let _ = delete_hkcu_reg_value(WINDOWS_STARTUP_APPROVED_RUN_SUBKEY, value_name);
        }
        Ok(AutostartOutcome::Enabled(location))
    } else {
        let removed = delete_hkcu_reg_value(WINDOWS_RUN_SUBKEY, value_name)?;
        let _ = delete_hkcu_reg_value(WINDOWS_STARTUP_APPROVED_RUN_SUBKEY, value_name);
        if removed {
            Ok(AutostartOutcome::Disabled(location))
        } else {
            Ok(AutostartOutcome::AlreadyDisabled(location))
        }
    }
}

#[cfg(target_os = "macos")]
fn resolve_macos_launch_agents_dir() -> Result<PathBuf, String> {
    let home = std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "Could not resolve HOME to locate ~/Library/LaunchAgents.".to_string())?;
    Ok(home.join("Library").join("LaunchAgents"))
}

#[cfg(target_os = "linux")]
fn resolve_linux_autostart_dir() -> Result<PathBuf, String> {
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        return Ok(config_home.join("autostart"));
    }
    let home = std::env::var_os("HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "Could not resolve HOME to locate ~/.config/autostart.".to_string())?;
    Ok(home.join(".config").join("autostart"))
}

fn resolve_autostart_icon_ref(app: AppId, icons_dir: Option<&Path>) -> String {
    #[cfg(any(target_os = "linux", test))]
    {
        match icons_dir {
            Some(dir) => match crate::shortcut::install_linux_icon(app, dir) {
                Ok(icon_path) => icon_path.to_string_lossy().into_owned(),
                Err(err) => {
                    log::warn!("Could not write autostart icon PNG cleanly: {err}");
                    app.manifest_key().to_string()
                }
            },
            None => app.manifest_key().to_string(),
        }
    }
    #[cfg(not(any(target_os = "linux", test)))]
    {
        let _ = icons_dir;
        app.manifest_key().to_string()
    }
}

fn remove_autostart_file(path: &Path) -> Result<AutostartOutcome, String> {
    let display = path.display().to_string();
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == ErrorKind::NotFound => {
            return Ok(AutostartOutcome::AlreadyDisabled(display));
        }
        Err(err) => {
            return Err(format!("Could not inspect `{display}`: {err}"));
        }
    };
    if meta.is_dir() && !meta.file_type().is_symlink() {
        return Err(format!("Refusing to remove directory `{display}`."));
    }
    fs::remove_file(path)
        .map_err(|err| format!("Could not remove autostart file `{display}`: {err}"))?;
    Ok(AutostartOutcome::Disabled(display))
}

fn escape_xml_text(raw: &str) -> Result<String, String> {
    if raw.chars().any(char::is_control) {
        return Err("LaunchAgent string contains invalid control characters.".to_string());
    }
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    Ok(out)
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
            "pc_upd_autostart_{label}_{}_{nanos}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        TempDirGuard(p)
    }

    #[test]
    fn windows_run_command_quotes_paths_and_rejects_control_chars() {
        let cmd = format_windows_run_command(
            Path::new(r"C:\Users\Alex\Apps\PaperCraft Server\papercraft-server.exe"),
            &[AUTOSTART_ARG],
        )
        .unwrap();
        assert_eq!(
            cmd,
            r#""C:\Users\Alex\Apps\PaperCraft Server\papercraft-server.exe" --autostart"#
        );

        assert!(format_windows_run_command(Path::new(""), &[AUTOSTART_ARG]).is_err());
        assert!(format_windows_run_command(Path::new(r#"C:\bad"quote\a.exe"#), &[]).is_err());
        assert!(format_windows_run_command(Path::new("C:\\bad\nline.exe"), &[]).is_err());
        assert!(format_windows_run_command(Path::new("C:\\ok.exe"), &["bad arg"]).is_err());
    }

    #[test]
    fn linux_autostart_creates_updates_and_removes_desktop_entry_in_temp_dir() {
        let tmp = temp_dir("linux_autostart");
        let autostart_dir = tmp.0.join("autostart");
        let icons_dir = tmp.0.join("icons");
        let appimage = tmp
            .0
            .join("My Apps")
            .join("papercraft-server-linux-x86_64.AppImage");

        let created = sync_linux_autostart_in_dir(
            AppId::PaperCraftServer,
            &appimage,
            &autostart_dir,
            Some(&icons_dir),
            true,
        )
        .unwrap();
        let desktop_file = autostart_dir.join("papercraft-server.desktop");
        assert_eq!(
            created,
            AutostartOutcome::Enabled(desktop_file.display().to_string())
        );
        assert!(icons_dir.join("papercraft-server.png").is_file());

        let text = fs::read_to_string(&desktop_file).unwrap();
        assert!(text.contains("Name=PaperCraft Server\n"));
        assert!(text.contains("GenericName=Self-Hosted Sync Server\n"));
        assert!(text.contains("StartupNotify=false\n"));
        assert!(text.contains("StartupWMClass=papercraft-server\n"));
        assert!(text.contains("X-Publisher=Alyx Labs\n"));
        assert!(text.contains("X-GNOME-Autostart-enabled=true\n"));
        assert!(text.contains("X-KDE-autostart-after=panel\n"));
        assert!(text.contains(&format!(
            "Exec={} --autostart\n",
            format_desktop_exec_value(&appimage.to_string_lossy())
        )));

        let disabled = sync_linux_autostart_in_dir(
            AppId::PaperCraftServer,
            &appimage,
            &autostart_dir,
            Some(&icons_dir),
            false,
        )
        .unwrap();
        assert_eq!(
            disabled,
            AutostartOutcome::Disabled(desktop_file.display().to_string())
        );
        assert!(!desktop_file.exists());

        let already = sync_linux_autostart_in_dir(
            AppId::PaperCraftServer,
            &appimage,
            &autostart_dir,
            Some(&icons_dir),
            false,
        )
        .unwrap();
        assert_eq!(
            already,
            AutostartOutcome::AlreadyDisabled(desktop_file.display().to_string())
        );
    }

    #[test]
    fn macos_launch_agent_resolves_bundle_escapes_xml_and_removes_plist_in_temp_dir() {
        let tmp = temp_dir("macos_autostart");
        let launch_agents = tmp.0.join("LaunchAgents");
        let bundle = Path::new("/Applications/Alyx & Co/PaperCraft Server.app");

        let resolved = resolve_macos_launch_agent_program(AppId::PaperCraftServer, bundle);
        assert_eq!(
            resolved,
            PathBuf::from(
                "/Applications/Alyx & Co/PaperCraft Server.app/Contents/MacOS/papercraft-server"
            )
        );

        let created =
            sync_macos_autostart_in_dir(AppId::PaperCraftServer, bundle, &launch_agents, true)
                .unwrap();
        let plist_path = launch_agents.join("com.alyxlabs.papercraft.server.plist");
        assert_eq!(
            created,
            AutostartOutcome::Enabled(plist_path.display().to_string())
        );

        let plist = fs::read_to_string(&plist_path).unwrap();
        assert!(
            plist.contains("<key>Label</key>\n\t<string>com.alyxlabs.papercraft.server</string>")
        );
        assert!(plist.contains(
            "<key>AssociatedBundleIdentifiers</key>\n\t<array>\n\t\t<string>com.alyxlabs.papercraft.server</string>\n\t</array>"
        ));
        let expected_program = resolved.to_string_lossy().replace('&', "&amp;");
        assert!(resolved.ends_with("Contents/MacOS/papercraft-server"));
        assert!(plist.contains(&format!("<string>{expected_program}</string>")));
        assert!(expected_program.contains("Alyx &amp; Co"));
        assert!(plist.contains("<string>--autostart</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>\n\t<true/>"));

        let disabled =
            sync_macos_autostart_in_dir(AppId::PaperCraftServer, bundle, &launch_agents, false)
                .unwrap();
        assert_eq!(
            disabled,
            AutostartOutcome::Disabled(plist_path.display().to_string())
        );
        assert!(!plist_path.exists());

        let already =
            sync_macos_autostart_in_dir(AppId::PaperCraftServer, bundle, &launch_agents, false)
                .unwrap();
        assert_eq!(
            already,
            AutostartOutcome::AlreadyDisabled(plist_path.display().to_string())
        );
    }
}
