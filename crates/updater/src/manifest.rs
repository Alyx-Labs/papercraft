//! Release manifest schema, URL allowlists, and SHA-256 validation.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::UpdateError;

pub const PUBLISHER_NAME: &str = "Alyx Labs";

pub const DEFAULT_MANIFEST_URL: &str =
    "https://github.com/Alyx-Labs/papercraft/releases/latest/download/latest.json";
pub const DEFAULT_RELEASES_PAGE_URL: &str =
    "https://github.com/Alyx-Labs/papercraft/releases/latest";
pub const OFFICIAL_RELEASES_PREFIX: &str = "https://github.com/Alyx-Labs/papercraft/releases/";
pub const OFFICIAL_DOWNLOAD_PREFIX: &str =
    "https://github.com/Alyx-Labs/papercraft/releases/download/";

pub const ENV_MANIFEST_URL: &str = "PAPERCRAFT_UPDATE_MANIFEST_URL";
pub const ENV_CURRENT_VERSION: &str = "PAPERCRAFT_UPDATE_CURRENT_VERSION";

pub const MAX_MANIFEST_BYTES: u64 = 256 * 1024;
pub const MAX_ASSET_BYTES: u64 = 256 * 1024 * 1024;
const MAX_URL_CHARS: usize = 2048;

/// Identifies which PaperCraft desktop binary is requesting an update.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AppId {
    PaperCraft,
    PaperCraftServer,
}

impl AppId {
    #[must_use]
    pub const fn manifest_key(self) -> &'static str {
        match self {
            Self::PaperCraft => "papercraft",
            Self::PaperCraftServer => "papercraft-server",
        }
    }

    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::PaperCraft => "PaperCraft",
            Self::PaperCraftServer => "PaperCraft Server",
        }
    }

    #[must_use]
    pub const fn executable_name(self) -> &'static str {
        match (self, cfg!(target_os = "windows")) {
            (Self::PaperCraft, true) => "papercraft.exe",
            (Self::PaperCraft, false) => "papercraft",
            (Self::PaperCraftServer, true) => "papercraft-server.exe",
            (Self::PaperCraftServer, false) => "papercraft-server",
        }
    }

    #[must_use]
    pub const fn macos_app_bundle_name(self) -> &'static str {
        match self {
            Self::PaperCraft => "PaperCraft.app",
            Self::PaperCraftServer => "PaperCraft Server.app",
        }
    }

    /// Reverse-DNS bundle / tray / launch-agent identifier under Alyx Labs.
    #[must_use]
    pub const fn bundle_identifier(self) -> &'static str {
        // TODO(org): Adjust reverse-DNS domain if Alyx Labs adopts a custom domain other than com.alyxlabs.
        match self {
            Self::PaperCraft => "com.alyxlabs.papercraft",
            Self::PaperCraftServer => "com.alyxlabs.papercraft.server",
        }
    }

    /// Plain-English application summary used in OS desktop entries and metadata.
    #[must_use]
    pub const fn summary(self) -> &'static str {
        match self {
            Self::PaperCraft => "PaperCraft notebook and handwriting studio",
            Self::PaperCraftServer => "PaperCraft self-hosted sync server control",
        }
    }

    /// Short generic category title used in Freedesktop `.desktop` entries (`GenericName`).
    #[must_use]
    pub const fn generic_name(self) -> &'static str {
        match self {
            Self::PaperCraft => "Handwriting & Notes Studio",
            Self::PaperCraftServer => "Self-Hosted Sync Server",
        }
    }
}

/// Packaging format of a downloadable release artifact.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssetFormat {
    #[serde(rename = "exe")]
    Exe,
    #[serde(rename = "appimage")]
    AppImage,
    #[serde(rename = "binary")]
    Binary,
    #[serde(rename = "macos-app-tar-gz")]
    MacOsAppTarGz,
}

/// Per-platform downloadable binary entry inside [`ReleaseManifest`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformAsset {
    pub file_name: String,
    pub url: String,
    pub sha256: String,
    #[serde(default)]
    pub size_bytes: u64,
    pub format: AssetFormat,
}

/// JSON manifest published alongside GitHub Release artifacts (`latest.json`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseManifest {
    pub schema_version: u32,
    pub version: String,
    #[serde(default)]
    pub tag: String,
    #[serde(default)]
    pub release_url: String,
    #[serde(default)]
    pub published_at: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub assets: BTreeMap<String, BTreeMap<String, PlatformAsset>>,
}

/// Returns the platform key used inside `ReleaseManifest.assets.<app>.<platform>`.
#[must_use]
pub fn current_platform_key() -> String {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "windows-x86_64".to_string(),
        ("linux", "x86_64") => "linux-x86_64".to_string(),
        ("macos", "aarch64") => "macos-aarch64".to_string(),
        (os, arch) => format!("{os}-{arch}"),
    }
}

/// Returns a human-readable label describing the host OS and CPU architecture.
#[must_use]
pub fn current_platform_label() -> String {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => "Windows (x86_64)".to_string(),
        ("linux", "x86_64") => "Linux AppImage (x86_64)".to_string(),
        ("macos", "aarch64") => "macOS (Apple Silicon)".to_string(),
        ("macos", "x86_64") => "macOS (Intel)".to_string(),
        (os, arch) => format!("{os} ({arch})"),
    }
}

/// Formats a byte count into a human-friendly size string (`B`, `KB`, `MB`, `GB`).
#[must_use]
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

/// Validates and normalizes a 64-character hexadecimal SHA-256 digest string.
pub fn validate_sha256_hex(raw: &str) -> Result<String, UpdateError> {
    let trimmed = raw.trim();
    if trimmed.len() != 64 || !trimmed.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(UpdateError::InvalidManifest(
            "asset `sha256` must be a 64-character hexadecimal digest".to_string(),
        ));
    }
    Ok(trimmed.to_ascii_lowercase())
}

/// Returns `true` when `url` is a local loopback `http://` URL (permitted only for local dev testing).
#[must_use]
pub fn is_loopback_http_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("http://127.0.0.1:")
        || lower.starts_with("http://127.0.0.1/")
        || lower.starts_with("http://localhost:")
        || lower.starts_with("http://localhost/")
        || lower.starts_with("http://[::1]:")
        || lower.starts_with("http://[::1]/")
}

/// Extracts `scheme://host[:port]` from an HTTP/HTTPS URL.
#[must_use]
pub fn url_origin(url: &str) -> Option<&str> {
    let trimmed = url.trim();
    let scheme_end = trimmed.find("://")?;
    let auth_start = scheme_end + 3;
    let rest = &trimmed[auth_start..];
    if rest.is_empty() {
        return None;
    }
    let auth_len = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    if auth_len == 0 {
        return None;
    }
    Some(&trimmed[..auth_start + auth_len])
}

fn check_basic_url_shape(url: &str) -> Result<&str, UpdateError> {
    let trimmed = url.trim();
    if trimmed.is_empty()
        || trimmed.len() > MAX_URL_CHARS
        || trimmed.chars().any(|c| c.is_control() || c.is_whitespace())
    {
        return Err(UpdateError::InvalidUrl(
            "URL is empty, too long, or contains whitespace/control characters".to_string(),
        ));
    }
    Ok(trimmed)
}

/// Validates a manifest URL before performing an HTTP request.
pub fn validate_manifest_url(url: &str, is_override: bool) -> Result<(), UpdateError> {
    let clean = check_basic_url_shape(url)?;
    if !is_override {
        if clean.starts_with(OFFICIAL_RELEASES_PREFIX) {
            return Ok(());
        }
        return Err(UpdateError::InvalidUrl(
            "official manifest URL must point to GitHub Releases".to_string(),
        ));
    }
    if clean.starts_with("https://") || is_loopback_http_url(clean) {
        return Ok(());
    }
    Err(UpdateError::InvalidUrl(
        "override manifest URL must use HTTPS or loopback HTTP".to_string(),
    ))
}

/// Validates a downloadable asset URL against the manifest origin / official GitHub release rules.
pub fn validate_asset_url(
    asset_url: &str,
    manifest_url: &str,
    is_override: bool,
) -> Result<(), UpdateError> {
    let clean_asset = check_basic_url_shape(asset_url)?;
    if clean_asset.starts_with(OFFICIAL_DOWNLOAD_PREFIX) {
        return Ok(());
    }
    if !is_override {
        return Err(UpdateError::InvalidUrl(format!(
            "asset URL `{clean_asset}` is outside official GitHub release downloads"
        )));
    }
    let allowed_scheme = clean_asset.starts_with("https://") || is_loopback_http_url(clean_asset);
    if !allowed_scheme {
        return Err(UpdateError::InvalidUrl(format!(
            "asset URL `{clean_asset}` must use HTTPS or loopback HTTP"
        )));
    }
    let asset_origin = url_origin(clean_asset);
    let manifest_origin = url_origin(manifest_url);
    if asset_origin.is_some() && asset_origin == manifest_origin {
        return Ok(());
    }
    Err(UpdateError::InvalidUrl(format!(
        "override asset URL `{clean_asset}` does not match manifest origin"
    )))
}

/// Validates a human-facing release page URL and falls back to the official releases page on bad input.
#[must_use]
pub fn safe_release_page_url(raw: &str) -> String {
    let trimmed = raw.trim();
    if !trimmed.is_empty()
        && trimmed.len() <= MAX_URL_CHARS
        && !trimmed.chars().any(|c| c.is_control() || c.is_whitespace())
        && (trimmed.starts_with("https://") || is_loopback_http_url(trimmed))
    {
        trimmed.to_string()
    } else {
        DEFAULT_RELEASES_PAGE_URL.to_string()
    }
}

/// Sanitizes an asset `file_name` to a single flat filename (no path separators or traversal).
pub fn sanitize_asset_file_name(raw: &str) -> Result<String, UpdateError> {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed == "."
        || trimmed == ".."
        || trimmed.len() > 200
        || trimmed.contains('/')
        || trimmed.contains('\\')
        || trimmed.chars().any(char::is_control)
    {
        return Err(UpdateError::InvalidManifest(format!(
            "unsafe asset file name `{raw}`"
        )));
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn validates_manifest_and_asset_url_allowlists() {
        assert!(validate_manifest_url(DEFAULT_MANIFEST_URL, false).is_ok());
        assert!(validate_manifest_url("https://evil.example.com/latest.json", false).is_err());
        assert!(validate_manifest_url("http://127.0.0.1:8799/latest.json", false).is_err());
        assert!(validate_manifest_url("http://127.0.0.1:8799/latest.json", true).is_ok());
        assert!(validate_manifest_url("http://192.168.1.10:8799/latest.json", true).is_err());

        let official_dl = "https://github.com/Alyx-Labs/papercraft/releases/download/v0.1.2/papercraft-windows-x86_64.exe";
        assert!(validate_asset_url(official_dl, DEFAULT_MANIFEST_URL, false).is_ok());
        assert!(
            validate_asset_url(
                "https://github.com/other/repo/releases/download/v1.0.0/evil.exe",
                DEFAULT_MANIFEST_URL,
                false
            )
            .is_err()
        );
        assert!(
            validate_asset_url(
                "http://127.0.0.1:8799/assets/papercraft.exe",
                "http://127.0.0.1:8799/latest.json",
                true
            )
            .is_ok()
        );
        assert!(
            validate_asset_url(
                "http://127.0.0.1:9999/assets/papercraft.exe",
                "http://127.0.0.1:8799/latest.json",
                true
            )
            .is_err()
        );
    }

    #[test]
    fn validates_sha256_and_file_names() {
        let valid = "A".repeat(64);
        assert_eq!(validate_sha256_hex(&valid).unwrap(), "a".repeat(64));
        assert!(validate_sha256_hex("abc").is_err());
        assert!(validate_sha256_hex(&"z".repeat(64)).is_err());

        assert_eq!(
            sanitize_asset_file_name("papercraft-windows-x86_64.exe").unwrap(),
            "papercraft-windows-x86_64.exe"
        );
        assert!(sanitize_asset_file_name("../evil.exe").is_err());
        assert!(sanitize_asset_file_name("sub/evil.exe").is_err());
        assert!(sanitize_asset_file_name("sub\\evil.exe").is_err());
        assert!(sanitize_asset_file_name("").is_err());
    }

    #[test]
    fn release_manifest_json_roundtrip() {
        let json_str = r#"{
            "schema_version": 1,
            "version": "0.1.2",
            "tag": "v0.1.2",
            "release_url": "https://github.com/Alyx-Labs/papercraft/releases/tag/v0.1.2",
            "published_at": "2026-11-24T12:00:00Z",
            "notes": "- Added auto-updater",
            "assets": {
                "papercraft": {
                    "windows-x86_64": {
                        "file_name": "papercraft-windows-x86_64.exe",
                        "url": "https://github.com/Alyx-Labs/papercraft/releases/download/v0.1.2/papercraft-windows-x86_64.exe",
                        "sha256": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
                        "size_bytes": 123456,
                        "format": "exe"
                    }
                }
            }
        }"#;
        let parsed: ReleaseManifest = serde_json::from_str(json_str).unwrap();
        assert_eq!(parsed.schema_version, 1);
        assert_eq!(parsed.version, "0.1.2");
        let asset = &parsed.assets["papercraft"]["windows-x86_64"];
        assert_eq!(asset.format, AssetFormat::Exe);
        assert_eq!(asset.size_bytes, 123_456);
    }
}
