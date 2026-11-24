//! HTTP release manifest client, asset downloader, and SHA-256 verifier.

use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use sha2::{Digest, Sha256};
use ureq::Agent;

use crate::UpdateError;
use crate::install::{InstallCapability, StagedUpdate, probe_install_capability};
use crate::manifest::{
    AppId, DEFAULT_MANIFEST_URL, ENV_CURRENT_VERSION, ENV_MANIFEST_URL, MAX_ASSET_BYTES,
    MAX_MANIFEST_BYTES, PlatformAsset, ReleaseManifest, current_platform_key,
    safe_release_page_url, sanitize_asset_file_name, validate_asset_url, validate_manifest_url,
    validate_sha256_hex,
};
use crate::version::SemVer;

const CONNECT_TIMEOUT_CHECK: Duration = Duration::from_secs(10);
const GLOBAL_TIMEOUT_CHECK: Duration = Duration::from_secs(20);
const CONNECT_TIMEOUT_DOWNLOAD: Duration = Duration::from_secs(15);
const GLOBAL_TIMEOUT_DOWNLOAD: Duration = Duration::from_secs(300);
const MAX_REDIRECTS: u32 = 10;
const DOWNLOAD_CHUNK_BYTES: usize = 64 * 1024;

/// Configuration governing which manifest URL and baseline version the updater compares against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UpdateConfig {
    pub app: AppId,
    pub current_version: SemVer,
    pub manifest_url: String,
    pub is_custom_feed: bool,
}

impl UpdateConfig {
    /// Builds an [`UpdateConfig`] from the compiled crate version and optional dev env overrides.
    pub fn from_env(app: AppId, compiled_version: &str) -> Result<Self, UpdateError> {
        let version_raw = std::env::var(ENV_CURRENT_VERSION)
            .ok()
            .filter(|v| !v.trim().is_empty())
            .unwrap_or_else(|| compiled_version.to_string());
        let current_version = SemVer::parse(&version_raw)?;

        let override_url = std::env::var(ENV_MANIFEST_URL)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());

        let (manifest_url, is_custom_feed) = match override_url {
            Some(url) => {
                validate_manifest_url(&url, true)?;
                (url, true)
            }
            None => (DEFAULT_MANIFEST_URL.to_string(), false),
        };

        Ok(Self {
            app,
            current_version,
            manifest_url,
            is_custom_feed,
        })
    }
}

/// Metadata for a newer verified release matching the current platform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AvailableUpdate {
    pub app: AppId,
    pub current_version: SemVer,
    pub version: SemVer,
    pub tag: String,
    pub release_url: String,
    pub published_at: String,
    pub notes: String,
    pub platform: String,
    pub asset: PlatformAsset,
    pub capability: InstallCapability,
    pub is_custom_feed: bool,
}

/// Result of checking the remote release manifest against the running build.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CheckOutcome {
    UpToDate {
        latest_version: SemVer,
    },
    UpdateAvailable(Box<AvailableUpdate>),
    NoPlatformAsset {
        latest_version: SemVer,
        release_url: String,
        platform: String,
    },
}

fn build_agent(connect_timeout: Duration, global_timeout: Duration, user_agent: &str) -> Agent {
    let cfg = ureq::config::Config::builder()
        .timeout_connect(Some(connect_timeout))
        .timeout_global(Some(global_timeout))
        .max_redirects(MAX_REDIRECTS)
        .http_status_as_error(false)
        .user_agent(user_agent)
        .build();
    Agent::new_with_config(cfg)
}

/// Fetches and validates the release manifest from `config.manifest_url`.
pub fn check_for_update(config: &UpdateConfig) -> Result<CheckOutcome, UpdateError> {
    validate_manifest_url(&config.manifest_url, config.is_custom_feed)?;
    let user_agent = format!("PaperCraft-Updater/{}", config.current_version);
    let agent = build_agent(CONNECT_TIMEOUT_CHECK, GLOBAL_TIMEOUT_CHECK, &user_agent);

    let mut resp = agent
        .get(&config.manifest_url)
        .call()
        .map_err(|err| UpdateError::Network(err.to_string()))?;

    let status = resp.status().as_u16();
    if status == 404 {
        return Err(UpdateError::FeedNotPublished);
    }
    if !(200..300).contains(&status) {
        return Err(UpdateError::HttpStatus(status));
    }

    let manifest: ReleaseManifest = resp
        .body_mut()
        .with_config()
        .limit(MAX_MANIFEST_BYTES)
        .read_json()
        .map_err(|err| UpdateError::InvalidManifest(format!("malformed manifest JSON: {err}")))?;

    evaluate_manifest(manifest, config)
}

pub(crate) fn evaluate_manifest(
    manifest: ReleaseManifest,
    config: &UpdateConfig,
) -> Result<CheckOutcome, UpdateError> {
    if manifest.schema_version != 1 {
        return Err(UpdateError::InvalidManifest(format!(
            "unsupported manifest schema_version {}",
            manifest.schema_version
        )));
    }
    let remote_version = SemVer::parse(&manifest.version)?;
    if remote_version <= config.current_version {
        return Ok(CheckOutcome::UpToDate {
            latest_version: remote_version,
        });
    }

    let release_url = safe_release_page_url(&manifest.release_url);
    let platform = current_platform_key();
    let Some(mut asset) = manifest
        .assets
        .get(config.app.manifest_key())
        .and_then(|by_plat| by_plat.get(&platform))
        .cloned()
    else {
        return Ok(CheckOutcome::NoPlatformAsset {
            latest_version: remote_version,
            release_url,
            platform,
        });
    };

    asset.file_name = sanitize_asset_file_name(&asset.file_name)?;
    validate_asset_url(&asset.url, &config.manifest_url, config.is_custom_feed)?;
    asset.sha256 = validate_sha256_hex(&asset.sha256)?;
    if asset.size_bytes > MAX_ASSET_BYTES {
        return Err(UpdateError::AssetTooLarge {
            actual: asset.size_bytes,
            max: MAX_ASSET_BYTES,
        });
    }

    let capability = probe_install_capability(config.app, asset.format);
    Ok(CheckOutcome::UpdateAvailable(Box::new(AvailableUpdate {
        app: config.app,
        current_version: config.current_version.clone(),
        version: remote_version,
        tag: manifest.tag,
        release_url,
        published_at: manifest.published_at,
        notes: manifest.notes,
        platform,
        asset,
        capability,
        is_custom_feed: config.is_custom_feed,
    })))
}

struct PartFileGuard {
    path: PathBuf,
    armed: bool,
}

impl PartFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PartFileGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Streams `update.asset.url` to a sibling `.update.part` file, validates size + SHA-256, and stages it.
pub fn download_and_verify_update(
    update: &AvailableUpdate,
    config: &UpdateConfig,
    cancel: &Arc<AtomicBool>,
    mut on_progress: impl FnMut(u64, u64),
) -> Result<StagedUpdate, UpdateError> {
    let InstallCapability::Supported {
        target_path,
        staging_dir,
    } = &update.capability
    else {
        return Err(UpdateError::UnsupportedInstall(
            "this installation folder does not support in-app replacement".to_string(),
        ));
    };

    let clean_name = sanitize_asset_file_name(&update.asset.file_name)?;
    validate_asset_url(
        &update.asset.url,
        &config.manifest_url,
        config.is_custom_feed,
    )?;
    let expected_sha = validate_sha256_hex(&update.asset.sha256)?;

    let part_path = staging_dir.join(format!("{clean_name}.update.part"));
    let stage_path = staging_dir.join(format!("{clean_name}.update.stage"));
    let _ = fs::remove_file(&part_path);
    let _ = fs::remove_file(&stage_path);
    let mut part_guard = PartFileGuard::new(part_path.clone());

    let user_agent = format!("PaperCraft-Updater/{}", config.current_version);
    let agent = build_agent(
        CONNECT_TIMEOUT_DOWNLOAD,
        GLOBAL_TIMEOUT_DOWNLOAD,
        &user_agent,
    );
    let mut resp = agent
        .get(&update.asset.url)
        .call()
        .map_err(|err| UpdateError::Network(err.to_string()))?;

    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(UpdateError::HttpStatus(status));
    }

    let content_len = resp.body().content_length().unwrap_or(0);
    let expected_total = if update.asset.size_bytes > 0 {
        update.asset.size_bytes
    } else {
        content_len
    };
    if expected_total > MAX_ASSET_BYTES {
        return Err(UpdateError::AssetTooLarge {
            actual: expected_total,
            max: MAX_ASSET_BYTES,
        });
    }

    let file = File::create(&part_path).map_err(|err| {
        UpdateError::Io(format!(
            "could not create staging file `{}`: {err}",
            part_path.display()
        ))
    })?;
    let mut writer = BufWriter::new(file);
    let mut hasher = Sha256::new();
    let mut downloaded: u64 = 0;

    on_progress(0, expected_total);
    let mut reader = resp.body_mut().as_reader();
    let mut buf = vec![0_u8; DOWNLOAD_CHUNK_BYTES];

    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(UpdateError::Cancelled);
        }
        let read_n = reader
            .read(&mut buf)
            .map_err(|err| UpdateError::Network(format!("stream read error: {err}")))?;
        if read_n == 0 {
            break;
        }
        downloaded = downloaded.saturating_add(read_n as u64);
        if downloaded > MAX_ASSET_BYTES {
            return Err(UpdateError::AssetTooLarge {
                actual: downloaded,
                max: MAX_ASSET_BYTES,
            });
        }
        let chunk = &buf[..read_n];
        hasher.update(chunk);
        writer
            .write_all(chunk)
            .map_err(|err| UpdateError::Io(format!("failed writing download buffer: {err}")))?;
        on_progress(downloaded, expected_total);
    }

    writer
        .flush()
        .map_err(|err| UpdateError::Io(format!("failed flushing download file: {err}")))?;
    let inner_file = writer
        .into_inner()
        .map_err(|err| UpdateError::Io(format!("failed finalizing buffered writer: {err}")))?;
    inner_file
        .sync_all()
        .map_err(|err| UpdateError::Io(format!("failed syncing download to disk: {err}")))?;
    drop(inner_file);

    if update.asset.size_bytes > 0 && downloaded != update.asset.size_bytes {
        return Err(UpdateError::ChecksumMismatch {
            expected: format!("{} bytes ({expected_sha})", update.asset.size_bytes),
            actual: format!("{downloaded} bytes"),
        });
    }

    let actual_sha = encode_lower_hex(&hasher.finalize());
    if actual_sha != expected_sha {
        return Err(UpdateError::ChecksumMismatch {
            expected: expected_sha,
            actual: actual_sha,
        });
    }

    promote_verified_part_file(&part_path, &stage_path)?;
    part_guard.disarm();

    Ok(StagedUpdate {
        app: update.app,
        version: update.version.clone(),
        staged_path: stage_path,
        target_path: target_path.clone(),
        format: update.asset.format,
        sha256: actual_sha,
    })
}

fn promote_verified_part_file(part_path: &Path, stage_path: &Path) -> Result<(), UpdateError> {
    fs::rename(part_path, stage_path).map_err(|err| {
        UpdateError::Io(format!(
            "could not finalize verified update at `{}`: {err}",
            stage_path.display()
        ))
    })
}

fn encode_lower_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(char::from(HEX[usize::from(b >> 4)]));
        out.push(char::from(HEX[usize::from(b & 0x0F)]));
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

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
        let p = std::env::temp_dir().join(format!("pc_upd_cli_{label}_{nanos}"));
        let _ = fs::remove_dir_all(&p);
        fs::create_dir_all(&p).unwrap();
        TempDirGuard(p)
    }

    fn sha256_of(bytes: &[u8]) -> String {
        let digest = Sha256::digest(bytes);
        encode_lower_hex(&digest)
    }

    #[test]
    fn checks_downloads_verifies_sha256_and_rejects_corruption() {
        let tmp = temp_dir("e2e");
        let target_exe = tmp.0.join("papercraft-test.exe");
        fs::write(&target_exe, b"installed-v011").unwrap();

        let payload = b"new-verified-binary-payload-v012".repeat(64);
        let good_sha = sha256_of(&payload);
        let plat = current_platform_key();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let base_url = format!("http://{addr}");
        let asset_url = format!("{base_url}/assets/app-bin");
        let manifest_json = serde_json::json!({
            "schema_version": 1,
            "version": "0.1.2",
            "tag": "v0.1.2",
            "release_url": format!("{base_url}/release"),
            "published_at": "2026-11-24T12:00:00Z",
            "notes": "Test update notes",
            "assets": {
                "papercraft": {
                    plat: {
                        "file_name": "app-bin",
                        "url": asset_url,
                        "sha256": good_sha,
                        "size_bytes": payload.len(),
                        "format": "binary"
                    }
                }
            }
        })
        .to_string();

        let server = thread::spawn(move || {
            for _ in 0..4 {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let mut req_buf = [0_u8; 2048];
                let n = stream.read(&mut req_buf).unwrap_or(0);
                let req_str = String::from_utf8_lossy(&req_buf[..n]);
                let target = req_str
                    .lines()
                    .next()
                    .unwrap_or("")
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/");
                let (status, ctype, body): (&str, &str, &[u8]) = if target == "/latest.json" {
                    ("200 OK", "application/json", manifest_json.as_bytes())
                } else if target == "/assets/app-bin" {
                    ("200 OK", "application/octet-stream", &payload)
                } else {
                    ("404 Not Found", "text/plain", b"missing")
                };
                let hdr = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(hdr.as_bytes());
                let _ = stream.write_all(body);
                let _ = stream.flush();
            }
        });

        let config = UpdateConfig {
            app: AppId::PaperCraft,
            current_version: SemVer::parse("0.1.1").unwrap(),
            manifest_url: format!("{base_url}/latest.json"),
            is_custom_feed: true,
        };

        let outcome = check_for_update(&config).unwrap();
        let mut update = match outcome {
            CheckOutcome::UpdateAvailable(u) => *u,
            other => panic!("expected UpdateAvailable, got {other:?}"),
        };
        assert_eq!(update.version.to_string(), "0.1.2");
        update.capability = InstallCapability::Supported {
            target_path: target_exe.clone(),
            staging_dir: tmp.0.clone(),
        };

        let cancel = Arc::new(AtomicBool::new(false));
        let mut progress_updates = Vec::new();
        let staged = download_and_verify_update(&update, &config, &cancel, |done, total| {
            progress_updates.push((done, total));
        })
        .unwrap();
        assert!(!progress_updates.is_empty());
        assert_eq!(staged.sha256, good_sha);
        assert!(staged.staged_path.is_file());

        // Now force a SHA-256 mismatch and confirm partial file is deleted.
        update.asset.sha256 = "f".repeat(64);
        let err = download_and_verify_update(&update, &config, &cancel, |_, _| {}).unwrap_err();
        assert!(matches!(err, UpdateError::ChecksumMismatch { .. }));
        assert!(!tmp.0.join("app-bin.update.part").exists());

        // And check 404 returns FeedNotPublished.
        let missing_cfg = UpdateConfig {
            manifest_url: format!("{base_url}/missing.json"),
            ..config
        };
        assert!(matches!(
            check_for_update(&missing_cfg),
            Err(UpdateError::FeedNotPublished)
        ));

        let _ = server.join();
    }
}
