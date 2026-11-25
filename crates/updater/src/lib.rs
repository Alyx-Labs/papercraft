//! Pure-Rust GitHub Releases update checker, SHA-256 verifier, and self-updater.

#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod manifest;
pub mod version;

#[cfg(not(target_arch = "wasm32"))]
pub mod client;
#[cfg(not(target_arch = "wasm32"))]
pub mod install;
#[cfg(not(target_arch = "wasm32"))]
pub mod shortcut;
#[cfg(not(target_arch = "wasm32"))]
pub mod worker;

use std::error::Error;
use std::fmt;

pub use manifest::{
    AppId, AssetFormat, DEFAULT_MANIFEST_URL, DEFAULT_RELEASES_PAGE_URL, ENV_CURRENT_VERSION,
    ENV_MANIFEST_URL, MAX_ASSET_BYTES, MAX_MANIFEST_BYTES, PlatformAsset, ReleaseManifest,
    current_platform_key, current_platform_label, format_bytes,
};
pub use version::SemVer;

#[cfg(not(target_arch = "wasm32"))]
pub use client::{
    AvailableUpdate, CheckOutcome, UpdateConfig, check_for_update, download_and_verify_update,
};
#[cfg(not(target_arch = "wasm32"))]
pub use install::{
    InstallCapability, StagedUpdate, apply_staged_update_and_relaunch,
    apply_staged_update_and_relaunch_with_shortcut, cleanup_stale_files, probe_install_capability,
};
#[cfg(not(target_arch = "wasm32"))]
pub use shortcut::{
    ShortcutOutcome, shortcut_architecture_hint, sync_desktop_shortcut,
    sync_desktop_shortcut_for_target,
};
#[cfg(not(target_arch = "wasm32"))]
pub use worker::{UpdateStatus, UpdateWorker};

/// Errors returned during update discovery, download, verification, or replacement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum UpdateError {
    InvalidVersion(String),
    InvalidUrl(String),
    InvalidManifest(String),
    FeedNotPublished,
    HttpStatus(u16),
    Network(String),
    Io(String),
    AssetTooLarge { actual: u64, max: u64 },
    ChecksumMismatch { expected: String, actual: String },
    Cancelled,
    UnsupportedInstall(String),
    RelaunchFailed(String),
}

impl fmt::Display for UpdateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidVersion(msg) => write!(f, "Invalid version string: {msg}"),
            Self::InvalidUrl(msg) => write!(f, "Invalid update URL: {msg}"),
            Self::InvalidManifest(msg) => write!(f, "Invalid release manifest: {msg}"),
            Self::FeedNotPublished => write!(
                f,
                "No auto-update manifest is published on GitHub Releases yet."
            ),
            Self::HttpStatus(code) => write!(f, "Update server returned HTTP {code}."),
            Self::Network(msg) => write!(f, "Could not reach update server: {msg}"),
            Self::Io(msg) => write!(f, "Update file system error: {msg}"),
            Self::AssetTooLarge { actual, max } => write!(
                f,
                "Update download ({actual} bytes) exceeds maximum allowed size ({max} bytes)."
            ),
            Self::ChecksumMismatch { expected, actual } => write!(
                f,
                "Update verification failed (SHA-256 mismatch: expected {expected}, got {actual})."
            ),
            Self::Cancelled => write!(f, "Update download cancelled."),
            Self::UnsupportedInstall(msg) | Self::RelaunchFailed(msg) => write!(f, "{msg}"),
        }
    }
}

impl Error for UpdateError {}

/// Formats a human-readable "up to date" status line, clarifying when the running build
/// is newer than the latest published GitHub release.
#[must_use]
pub fn format_up_to_date_status(
    app_label: Option<&str>,
    current: Option<&SemVer>,
    latest: &SemVer,
) -> String {
    let is_ahead = current.is_some_and(|cur| cur > latest);
    match (app_label, is_ahead) {
        (Some(label), true) => {
            format!("{label} is up to date — newer than latest published release (v{latest}).")
        }
        (Some(label), false) => format!("{label} is up to date (latest: v{latest})."),
        (None, true) => format!("Up to date — newer than latest published release (v{latest})."),
        (None, false) => format!("Up to date (latest: v{latest})."),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn format_up_to_date_status_distinguishes_equal_ahead_and_unknown() {
        let v021 = SemVer::parse("0.2.1").unwrap();
        let v022 = SemVer::parse("0.2.2").unwrap();

        assert_eq!(
            format_up_to_date_status(None, Some(&v021), &v021),
            "Up to date (latest: v0.2.1)."
        );
        assert_eq!(
            format_up_to_date_status(None, Some(&v022), &v021),
            "Up to date — newer than latest published release (v0.2.1)."
        );
        assert_eq!(
            format_up_to_date_status(None, None, &v021),
            "Up to date (latest: v0.2.1)."
        );

        assert_eq!(
            format_up_to_date_status(Some("PaperCraft Server"), Some(&v021), &v021),
            "PaperCraft Server is up to date (latest: v0.2.1)."
        );
        assert_eq!(
            format_up_to_date_status(Some("PaperCraft Server"), Some(&v022), &v021),
            "PaperCraft Server is up to date — newer than latest published release (v0.2.1)."
        );
        assert_eq!(
            format_up_to_date_status(Some("PaperCraft Server"), None, &v021),
            "PaperCraft Server is up to date (latest: v0.2.1)."
        );
    }
}
