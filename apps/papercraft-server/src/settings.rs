//! Persistent, plain-English configuration for the PaperCraft Server control app.

use std::fs;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::path::{Path, PathBuf};

use papercraft_doc::UiTheme;
use papercraft_server_cli::config::{
    DEFAULT_ACCESS_TTL_HOURS, DEFAULT_DATA_DIR, DEFAULT_MAX_NOTEBOOK_MB, DEFAULT_REFRESH_TTL_DAYS,
    ServerConfig,
};
use serde::{Deserialize, Serialize};

pub const CONFIG_FILE_NAME: &str = "server_control_config.json";
pub const DEFAULT_SERVER_PORT: u16 = 8787;
pub const MIN_MAX_NOTEBOOK_MB: usize = 1;
pub const MAX_MAX_NOTEBOOK_MB: usize = 512;
pub const MIN_REFRESH_TTL_DAYS: u64 = 1;
pub const MAX_REFRESH_TTL_DAYS: u64 = 3650;
pub const MIN_ACCESS_TTL_HOURS: u64 = 1;
pub const MAX_ACCESS_TTL_HOURS: u64 = 24 * 365;
const MAX_CONFIG_FILE_BYTES: u64 = 1024 * 1024;

/// Who should be able to reach the PaperCraft sync server.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum NetworkScope {
    /// Listen only on `127.0.0.1` (this computer only).
    #[default]
    LocalOnly,
    /// Listen on `0.0.0.0` (this computer plus devices on Wi-Fi, LAN, or mesh VPN).
    LocalNetwork,
    /// Listen on a specific IP address chosen by the user.
    Custom,
}

impl NetworkScope {
    pub const fn label(self) -> &'static str {
        match self {
            Self::LocalOnly => "This PC only",
            Self::LocalNetwork => "Local network / VPN",
            Self::Custom => "Custom IP",
        }
    }

    pub const fn description(self) -> &'static str {
        match self {
            Self::LocalOnly => {
                "Only PaperCraft running on this same computer can connect (127.0.0.1)."
            }
            Self::LocalNetwork => {
                "Accepts connections from this PC and other devices on your Wi-Fi, LAN, or mesh VPN (0.0.0.0)."
            }
            Self::Custom => "Bind strictly to one IP address, such as a specific VPN interface.",
        }
    }
}

/// Ready-to-paste client connection URLs derived from the active bind settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConnectionUrls {
    pub local_url: String,
    pub network_url: Option<String>,
    pub primary_url: String,
    pub local_is_loopback: bool,
}

/// User-friendly persisted settings mapped directly to [`papercraft_server_cli`] runtime variables.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ServerControlSettings {
    pub data_dir: String,
    pub network_scope: NetworkScope,
    pub custom_ip: String,
    pub port: u16,
    pub allow_registration: bool,
    pub max_notebook_mb: usize,
    pub refresh_ttl_days: u64,
    pub access_ttl_hours: u64,
    pub auto_start_server: bool,
    pub start_in_tray: bool,
    pub theme: UiTheme,
}

impl Default for ServerControlSettings {
    fn default() -> Self {
        Self {
            data_dir: default_data_dir_string(),
            network_scope: NetworkScope::LocalOnly,
            custom_ip: "127.0.0.1".to_string(),
            port: DEFAULT_SERVER_PORT,
            allow_registration: true,
            max_notebook_mb: DEFAULT_MAX_NOTEBOOK_MB,
            refresh_ttl_days: DEFAULT_REFRESH_TTL_DAYS,
            access_ttl_hours: DEFAULT_ACCESS_TTL_HOURS,
            auto_start_server: true,
            start_in_tray: false,
            theme: UiTheme::Dark,
        }
    }
}

fn lookup_env_pair(
    lookup: &impl Fn(&str) -> Option<String>,
    primary: &str,
    legacy: &str,
) -> Option<String> {
    lookup(primary).or_else(|| lookup(legacy))
}

impl ServerControlSettings {
    /// Returns settings seeded from `PAPERCRAFT_*` (or legacy `NOTECRAFT_*`) environment variables on first launch.
    pub fn from_env_with_default_dir(
        default_dir: String,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Self {
        let mut s = Self {
            data_dir: default_dir,
            ..Self::default()
        };

        if let Some(dir) = lookup_env_pair(&lookup, "PAPERCRAFT_DATA_DIR", "NOTECRAFT_DATA_DIR") {
            let trimmed = dir.trim();
            if !trimmed.is_empty() {
                s.data_dir = trimmed.to_string();
            }
        }

        if let Some(bind_raw) = lookup_env_pair(&lookup, "PAPERCRAFT_BIND", "NOTECRAFT_BIND")
            && let Ok(addr) = bind_raw.trim().parse::<SocketAddr>()
        {
            s.port = addr.port().max(1);
            let ip = addr.ip();
            if ip == IpAddr::V4(Ipv4Addr::LOCALHOST) {
                s.network_scope = NetworkScope::LocalOnly;
            } else if ip.is_unspecified() {
                s.network_scope = NetworkScope::LocalNetwork;
            } else {
                s.network_scope = NetworkScope::Custom;
                s.custom_ip = ip.to_string();
            }
        }

        if let Some(disable_raw) = lookup_env_pair(
            &lookup,
            "PAPERCRAFT_DISABLE_REGISTRATION",
            "NOTECRAFT_DISABLE_REGISTRATION",
        ) && let Some(disabled) = parse_env_bool(&disable_raw)
        {
            s.allow_registration = !disabled;
        }

        if let Some(hours) = lookup_env_pair(
            &lookup,
            "PAPERCRAFT_ACCESS_TTL_HOURS",
            "NOTECRAFT_ACCESS_TTL_HOURS",
        )
        .and_then(|v| v.trim().parse::<u64>().ok())
        {
            s.access_ttl_hours = hours;
        }

        if let Some(days) = lookup_env_pair(
            &lookup,
            "PAPERCRAFT_REFRESH_TTL_DAYS",
            "NOTECRAFT_REFRESH_TTL_DAYS",
        )
        .and_then(|v| v.trim().parse::<u64>().ok())
        {
            s.refresh_ttl_days = days;
        }

        if let Some(max_mb) = lookup_env_pair(
            &lookup,
            "PAPERCRAFT_MAX_NOTEBOOK_MB",
            "NOTECRAFT_MAX_NOTEBOOK_MB",
        )
        .and_then(|v| v.trim().parse::<usize>().ok())
        {
            s.max_notebook_mb = max_mb;
        }

        s.sanitized()
    }

    /// Clamps numeric fields to safe supported bounds without mutating freeform paths unexpectedly.
    pub fn sanitized(mut self) -> Self {
        self.max_notebook_mb = self
            .max_notebook_mb
            .clamp(MIN_MAX_NOTEBOOK_MB, MAX_MAX_NOTEBOOK_MB);
        self.refresh_ttl_days = self
            .refresh_ttl_days
            .clamp(MIN_REFRESH_TTL_DAYS, MAX_REFRESH_TTL_DAYS);
        self.access_ttl_hours = self
            .access_ttl_hours
            .clamp(MIN_ACCESS_TTL_HOURS, MAX_ACCESS_TTL_HOURS);
        if self.custom_ip.trim().is_empty() {
            self.custom_ip = "127.0.0.1".to_string();
        }
        self
    }

    /// Loads config JSON from `path` if present, or bootstraps defaults from environment variables.
    pub fn load_or_init(path: &Path) -> (Self, Option<String>) {
        if !path.exists() {
            let initial = Self::from_env_with_default_dir(default_data_dir_string(), |key| {
                std::env::var(key).ok()
            });
            return (initial, None);
        }

        match fs::metadata(path) {
            Ok(meta) if meta.len() > MAX_CONFIG_FILE_BYTES => {
                let fallback = Self::default();
                return (
                    fallback,
                    Some(format!(
                        "Config file `{}` is larger than 1 MiB; loaded default settings instead.",
                        path.display()
                    )),
                );
            }
            Err(err) => {
                let fallback = Self::default();
                return (
                    fallback,
                    Some(format!(
                        "Could not inspect `{}` ({err}); loaded default settings.",
                        path.display()
                    )),
                );
            }
            Ok(_) => {}
        }

        match fs::read_to_string(path) {
            Ok(raw) => match serde_json::from_str::<Self>(&raw) {
                Ok(parsed) => (parsed.sanitized(), None),
                Err(err) => (
                    Self::default(),
                    Some(format!(
                        "Could not parse `{}` ({err}); loaded default settings.",
                        path.display()
                    )),
                ),
            },
            Err(err) => (
                Self::default(),
                Some(format!(
                    "Could not read `{}` ({err}); loaded default settings.",
                    path.display()
                )),
            ),
        }
    }

    /// Atomically saves settings JSON next to the executable (or target path).
    pub fn save_atomic(&self, path: &Path) -> Result<(), String> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|err| {
                format!(
                    "Could not create config folder `{}`: {err}",
                    parent.display()
                )
            })?;
        }
        let clean = self.clone().sanitized();
        let json = serde_json::to_string_pretty(&clean)
            .map_err(|err| format!("Failed to encode server settings: {err}"))?;

        let tmp_name = format!(
            "{}.tmp",
            path.file_name()
                .and_then(|n| n.to_str())
                .unwrap_or(CONFIG_FILE_NAME)
        );
        let tmp_path = path.with_file_name(tmp_name);
        fs::write(&tmp_path, json.as_bytes()).map_err(|err| {
            format!(
                "Could not write temporary config `{}`: {err}",
                tmp_path.display()
            )
        })?;
        if let Err(rename_err) = fs::rename(&tmp_path, path) {
            let _ = fs::remove_file(&tmp_path);
            return Err(format!(
                "Could not save config `{}`: {rename_err}",
                path.display()
            ));
        }
        Ok(())
    }

    /// Validates user edits and constructs a [`ServerConfig`] for [`papercraft_server_cli::ServerRuntime`].
    pub fn to_server_config(&self) -> Result<ServerConfig, String> {
        let trimmed_dir = self.data_dir.trim();
        if trimmed_dir.is_empty() {
            return Err(
                "Choose a data folder where accounts and synced notebooks will be saved."
                    .to_string(),
            );
        }
        if self.port == 0 {
            return Err("Port must be between 1 and 65535 (recommended: 8787).".to_string());
        }

        let ip = self.resolve_bind_ip()?;
        let bind_addr = SocketAddr::new(ip, self.port);
        let mut cfg = ServerConfig::from_options(
            PathBuf::from(trimmed_dir),
            bind_addr,
            self.allow_registration,
            self.access_ttl_hours,
            self.refresh_ttl_days,
            self.max_notebook_mb,
        );
        cfg.extra_library_roots = exe_sibling_path("library").into_iter().collect();
        Ok(cfg)
    }

    /// Resolves the IP address associated with `network_scope` and `custom_ip`.
    pub fn resolve_bind_ip(&self) -> Result<IpAddr, String> {
        match self.network_scope {
            NetworkScope::LocalOnly => Ok(IpAddr::V4(Ipv4Addr::LOCALHOST)),
            NetworkScope::LocalNetwork => Ok(IpAddr::V4(Ipv4Addr::UNSPECIFIED)),
            NetworkScope::Custom => parse_ip_input(&self.custom_ip).ok_or_else(|| {
                "Enter a valid IPv4 or IPv6 address (for example 127.0.0.1 or 100.64.0.12)."
                    .to_string()
            }),
        }
    }

    /// Returns `true` when every setting that affects a running HTTP/DB runtime matches `other`.
    pub fn same_runtime_config(&self, other: &Self) -> bool {
        let same_custom = self.network_scope != NetworkScope::Custom
            || self.custom_ip.trim() == other.custom_ip.trim();
        self.data_dir.trim() == other.data_dir.trim()
            && self.network_scope == other.network_scope
            && same_custom
            && self.port == other.port
            && self.allow_registration == other.allow_registration
            && self
                .max_notebook_mb
                .clamp(MIN_MAX_NOTEBOOK_MB, MAX_MAX_NOTEBOOK_MB)
                == other
                    .max_notebook_mb
                    .clamp(MIN_MAX_NOTEBOOK_MB, MAX_MAX_NOTEBOOK_MB)
            && self
                .refresh_ttl_days
                .clamp(MIN_REFRESH_TTL_DAYS, MAX_REFRESH_TTL_DAYS)
                == other
                    .refresh_ttl_days
                    .clamp(MIN_REFRESH_TTL_DAYS, MAX_REFRESH_TTL_DAYS)
            && self
                .access_ttl_hours
                .clamp(MIN_ACCESS_TTL_HOURS, MAX_ACCESS_TTL_HOURS)
                == other
                    .access_ttl_hours
                    .clamp(MIN_ACCESS_TTL_HOURS, MAX_ACCESS_TTL_HOURS)
    }

    /// Computes practical client URLs for connecting PaperCraft Desktop or Web to this server.
    pub fn connection_urls(&self, lan_ip: Option<IpAddr>) -> ConnectionUrls {
        let port = if self.port == 0 {
            DEFAULT_SERVER_PORT
        } else {
            self.port
        };
        let loopback = IpAddr::V4(Ipv4Addr::LOCALHOST);
        match self.network_scope {
            NetworkScope::LocalOnly => {
                let local_url = format_http_url(loopback, port);
                ConnectionUrls {
                    primary_url: local_url.clone(),
                    local_url,
                    network_url: None,
                    local_is_loopback: true,
                }
            }
            NetworkScope::LocalNetwork => {
                let local_url = format_http_url(loopback, port);
                let network_url = lan_ip
                    .filter(|ip| !ip.is_unspecified() && !ip.is_loopback())
                    .map(|ip| format_http_url(ip, port));
                let primary_url = network_url.clone().unwrap_or_else(|| local_url.clone());
                ConnectionUrls {
                    local_url,
                    network_url,
                    primary_url,
                    local_is_loopback: true,
                }
            }
            NetworkScope::Custom => {
                let ip = parse_ip_input(&self.custom_ip).unwrap_or(loopback);
                if ip.is_unspecified() {
                    let local_url = format_http_url(loopback, port);
                    let network_url = lan_ip
                        .filter(|addr| !addr.is_unspecified() && !addr.is_loopback())
                        .map(|addr| format_http_url(addr, port));
                    let primary_url = network_url.clone().unwrap_or_else(|| local_url.clone());
                    ConnectionUrls {
                        local_url,
                        network_url,
                        primary_url,
                        local_is_loopback: true,
                    }
                } else {
                    let bound_url = format_http_url(ip, port);
                    ConnectionUrls {
                        primary_url: bound_url.clone(),
                        local_url: bound_url,
                        network_url: None,
                        local_is_loopback: ip.is_loopback(),
                    }
                }
            }
        }
    }

    pub fn resolved_data_path(&self) -> PathBuf {
        let trimmed = self.data_dir.trim();
        if trimmed.is_empty() {
            PathBuf::from(default_data_dir_string())
        } else {
            PathBuf::from(trimmed)
        }
    }
}

/// Returns the portable config path (`<exe_dir>/server_control_config.json`).
pub fn default_config_path() -> PathBuf {
    exe_sibling_path(CONFIG_FILE_NAME).unwrap_or_else(|| PathBuf::from(CONFIG_FILE_NAME))
}

/// Returns the portable default server data directory (`<exe_dir>/server-data`).
pub fn default_data_dir_string() -> String {
    exe_sibling_path("server-data")
        .unwrap_or_else(|| PathBuf::from(DEFAULT_DATA_DIR))
        .to_string_lossy()
        .into_owned()
}

fn exe_sibling_path(name: &str) -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let appimage = std::env::var_os("APPIMAGE").map(PathBuf::from);
    let home = std::env::var_os("HOME").map(PathBuf::from);
    resolve_sibling_path(&exe, appimage.as_deref(), home.as_deref(), name)
}

fn resolve_sibling_path(
    exe_path: &Path,
    appimage: Option<&Path>,
    home: Option<&Path>,
    name: &str,
) -> Option<PathBuf> {
    if let Some(img) = appimage.filter(|p| !p.as_os_str().is_empty())
        && let Some(parent) = img.parent().filter(|p| !p.as_os_str().is_empty())
    {
        return Some(parent.join(name));
    }
    if is_inside_macos_app_bundle(exe_path)
        && let Some(home_dir) = home.filter(|p| !p.as_os_str().is_empty())
    {
        return Some(
            home_dir
                .join("Library")
                .join("Application Support")
                .join("PaperCraft Server")
                .join(name),
        );
    }
    let parent = exe_path.parent().filter(|p| !p.as_os_str().is_empty())?;
    Some(parent.join(name))
}

fn is_inside_macos_app_bundle(exe_path: &Path) -> bool {
    let Some(macos_dir) = exe_path.parent() else {
        return false;
    };
    let Some(contents_dir) = macos_dir.parent() else {
        return false;
    };
    let Some(bundle_dir) = contents_dir.parent() else {
        return false;
    };
    macos_dir.file_name().and_then(|n| n.to_str()) == Some("MacOS")
        && contents_dir.file_name().and_then(|n| n.to_str()) == Some("Contents")
        && bundle_dir
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("app"))
}

fn parse_ip_input(raw: &str) -> Option<IpAddr> {
    let trimmed = raw.trim().trim_start_matches('[').trim_end_matches(']');
    trimmed.parse::<IpAddr>().ok()
}

fn parse_env_bool(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// Formats an HTTP base URL, bracketing IPv6 literals automatically.
pub fn format_http_url(ip: IpAddr, port: u16) -> String {
    match ip {
        IpAddr::V4(v4) => format!("http://{v4}:{port}"),
        IpAddr::V6(v6) => format!("http://[{v6}]:{port}"),
    }
}

/// Best-effort detection of the primary outbound local IPv4 address without sending network traffic.
pub fn detect_local_ipv4() -> Option<IpAddr> {
    for probe_target in ["192.0.2.1:80", "10.255.255.254:80"] {
        let Ok(socket) = UdpSocket::bind("0.0.0.0:0") else {
            continue;
        };
        if socket.connect(probe_target).is_ok()
            && let Ok(local) = socket.local_addr()
        {
            let ip = local.ip();
            if !ip.is_unspecified() && !ip.is_loopback() {
                return Some(ip);
            }
        }
    }
    None
}

/// Formats a byte count into a clean human-readable label.
pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;

    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use std::net::Ipv6Addr;

    struct TempFileDir(PathBuf);
    impl Drop for TempFileDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir(label: &str) -> TempFileDir {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let p = std::env::temp_dir().join(format!("pc_srv_ctrl_test_{label}_{nanos}"));
        let _ = fs::remove_dir_all(&p);
        TempFileDir(p)
    }

    #[test]
    fn json_roundtrip_and_atomic_save_preserve_all_fields() {
        let dir = temp_dir("roundtrip");
        let cfg_path = dir.0.join(CONFIG_FILE_NAME);
        let original = ServerControlSettings {
            data_dir: "D:\\MyNotes\\ServerData".to_string(),
            network_scope: NetworkScope::LocalNetwork,
            custom_ip: "100.64.0.12".to_string(),
            port: 9090,
            allow_registration: false,
            max_notebook_mb: 256,
            refresh_ttl_days: 90,
            access_ttl_hours: 48,
            auto_start_server: false,
            start_in_tray: true,
            theme: UiTheme::Dark,
        };

        original.save_atomic(&cfg_path).unwrap();
        let (loaded, warning) = ServerControlSettings::load_or_init(&cfg_path);
        assert!(warning.is_none());
        assert_eq!(loaded, original);
    }

    #[test]
    fn env_bootstrapping_imports_existing_server_env_vars() {
        let env_map = [
            ("PAPERCRAFT_DATA_DIR", "C:\\SyncVault"),
            ("PAPERCRAFT_BIND", "0.0.0.0:9443"),
            ("PAPERCRAFT_DISABLE_REGISTRATION", "true"),
            ("PAPERCRAFT_ACCESS_TTL_HOURS", "12"),
            ("PAPERCRAFT_REFRESH_TTL_DAYS", "45"),
            ("PAPERCRAFT_MAX_NOTEBOOK_MB", "300"),
        ];
        let s = ServerControlSettings::from_env_with_default_dir("fallback-dir".to_string(), |k| {
            env_map
                .iter()
                .find(|(key, _)| *key == k)
                .map(|(_, val)| (*val).to_string())
        });
        assert_eq!(s.data_dir, "C:\\SyncVault");
        assert_eq!(s.network_scope, NetworkScope::LocalNetwork);
        assert_eq!(s.port, 9443);
        assert!(!s.allow_registration);
        assert_eq!(s.access_ttl_hours, 12);
        assert_eq!(s.refresh_ttl_days, 45);
        assert_eq!(s.max_notebook_mb, 300);
    }

    #[test]
    fn validation_rejects_empty_dir_zero_port_and_bad_ip_and_clamps_bounds() {
        let bad_dir = ServerControlSettings {
            data_dir: "   ".to_string(),
            ..Default::default()
        };
        assert!(bad_dir.to_server_config().is_err());

        let bad_port = ServerControlSettings {
            port: 0,
            ..Default::default()
        };
        assert!(bad_port.to_server_config().is_err());

        let bad_ip = ServerControlSettings {
            network_scope: NetworkScope::Custom,
            custom_ip: "not-an-ip".to_string(),
            ..Default::default()
        };
        assert!(bad_ip.to_server_config().is_err());

        let extreme = ServerControlSettings {
            data_dir: "vault".to_string(),
            network_scope: NetworkScope::Custom,
            custom_ip: "[::1]".to_string(),
            port: 8787,
            allow_registration: false,
            max_notebook_mb: 9999,
            refresh_ttl_days: 0,
            access_ttl_hours: 999999,
            ..ServerControlSettings::default()
        };
        let cfg = extreme.to_server_config().unwrap();
        assert_eq!(
            cfg.bind_addr,
            SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 8787)
        );
        assert!(!cfg.registration_open);
        assert_eq!(cfg.max_notebook_bytes, 512 * 1024 * 1024);
        assert_eq!(cfg.refresh_token_ttl_ms, 86_400_000);
        assert_eq!(cfg.access_token_ttl_ms, (24 * 365_i64) * 3_600_000);
    }

    #[test]
    fn connection_urls_handle_ipv4_lan_and_ipv6_bracketing() {
        let lan = Some(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 44)));

        let local = ServerControlSettings {
            network_scope: NetworkScope::LocalOnly,
            port: 8787,
            ..ServerControlSettings::default()
        };
        let local_urls = local.connection_urls(lan);
        assert_eq!(local_urls.local_url, "http://127.0.0.1:8787");
        assert!(local_urls.network_url.is_none());
        assert_eq!(local_urls.primary_url, "http://127.0.0.1:8787");

        let net = ServerControlSettings {
            network_scope: NetworkScope::LocalNetwork,
            port: 8787,
            ..ServerControlSettings::default()
        };
        let net_urls = net.connection_urls(lan);
        assert_eq!(net_urls.local_url, "http://127.0.0.1:8787");
        assert_eq!(
            net_urls.network_url.as_deref(),
            Some("http://192.168.1.44:8787")
        );
        assert_eq!(net_urls.primary_url, "http://192.168.1.44:8787");

        let custom_v6 = ServerControlSettings {
            network_scope: NetworkScope::Custom,
            custom_ip: "fd00::42".to_string(),
            port: 9001,
            ..ServerControlSettings::default()
        };
        let v6_urls = custom_v6.connection_urls(lan);
        assert_eq!(v6_urls.local_url, "http://[fd00::42]:9001");
        assert!(!v6_urls.local_is_loopback);
    }

    #[test]
    fn same_runtime_config_ignores_ui_only_changes() {
        let a = ServerControlSettings::default();
        let mut b = a.clone();
        b.theme = UiTheme::Dark;
        b.auto_start_server = false;
        b.start_in_tray = true;
        assert!(a.same_runtime_config(&b));

        b.port = 9999;
        assert!(!a.same_runtime_config(&b));
    }

    #[test]
    fn sibling_paths_adapt_for_appimage_and_macos_app_bundle() {
        let win_exe = Path::new("/portable/papercraft-server.exe");
        assert_eq!(
            resolve_sibling_path(win_exe, None, None, "server-data"),
            Some(PathBuf::from("/portable/server-data"))
        );

        let appimage_exe = Path::new("/tmp/.mount_Srv123/usr/bin/papercraft-server");
        let appimage_file = Path::new("/home/alex/Apps/papercraft-server-linux-x86_64.AppImage");
        assert_eq!(
            resolve_sibling_path(
                appimage_exe,
                Some(appimage_file),
                Some(Path::new("/home/alex")),
                CONFIG_FILE_NAME,
            ),
            Some(PathBuf::from("/home/alex/Apps/server_control_config.json"))
        );

        let mac_exe =
            Path::new("/Applications/PaperCraft Server.app/Contents/MacOS/papercraft-server");
        assert_eq!(
            resolve_sibling_path(mac_exe, None, Some(Path::new("/Users/alex")), "server-data"),
            Some(PathBuf::from(
                "/Users/alex/Library/Application Support/PaperCraft Server/server-data"
            ))
        );
    }
}
