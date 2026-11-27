use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use papercraft_sync_protocol::{
    DEFAULT_SERVER_URL, LocalDiscoverySummary, normalize_server_url, sanitize_device_name,
};

use crate::types::{
    ACCOUNTS_DIR, ActiveAccountSession, ActiveWorkspaceTarget, GLOBAL_SYNC_FILE, GlobalSyncState,
    MAX_PROFILE_NICKNAME_LEN, MAX_REMEMBERED_SERVER_ACCOUNTS, PROFILES_DIR,
    RememberedServerAccount, SYNC_MANIFEST_FILE, SyncError, SyncManifest,
};

static ATOMIC_WRITE_SEQ: AtomicU64 = AtomicU64::new(0);

pub fn sanitize_client_id(raw: &str) -> String {
    raw.trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(64)
        .collect()
}

pub fn default_device_name() -> String {
    let raw = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "Windows Desktop".to_string());
    sanitize_device_name(&raw)
}

pub fn sanitize_profile_nickname(raw: &str) -> String {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_PROFILE_NICKNAME_LEN)
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        "Personal".to_string()
    } else {
        trimmed.to_string()
    }
}

pub fn sanitize_account_dir_segment(user_id: &str) -> String {
    let cleaned: String = user_id
        .trim()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .take(64)
        .collect();
    if cleaned.is_empty() {
        "default-user".to_string()
    } else {
        cleaned
    }
}

pub fn account_library_dir(root_dir: &Path, user_id: &str) -> PathBuf {
    root_dir
        .join(ACCOUNTS_DIR)
        .join(sanitize_account_dir_segment(user_id))
}

pub fn profile_library_dir(root_dir: &Path, local_profile_id: &str) -> PathBuf {
    root_dir
        .join(PROFILES_DIR)
        .join(sanitize_account_dir_segment(local_profile_id))
}

fn unique_temp_path(path: &Path) -> PathBuf {
    let seq = ATOMIC_WRITE_SEQ.fetch_add(1, Ordering::Relaxed);
    let mut tmp_os = path.as_os_str().to_os_string();
    tmp_os.push(format!(".tmp.{}.{seq}", std::process::id()));
    PathBuf::from(tmp_os)
}

fn write_and_sync_file(file: &mut fs::File, bytes: &[u8]) -> std::io::Result<()> {
    file.write_all(bytes)?;
    file.flush()?;
    match file.sync_data() {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == std::io::ErrorKind::Unsupported => Ok(()),
        Err(err) => Err(err),
    }
}

pub fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<(), SyncError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|e| SyncError::Io(format!("failed to create `{}`: {e}", parent.display())))?;
    }
    let tmp = unique_temp_path(path);
    let mut file = fs::File::create(&tmp)
        .map_err(|e| SyncError::Io(format!("failed to create `{}`: {e}", tmp.display())))?;
    if let Err(err) = write_and_sync_file(&mut file, bytes) {
        drop(file);
        let _ = fs::remove_file(&tmp);
        return Err(SyncError::Io(format!(
            "failed to write `{}`: {err}",
            tmp.display()
        )));
    }
    drop(file);

    // Retry replacement briefly on Windows to survive transient antivirus/indexer locks
    // without ever deleting the existing destination file before rename succeeds.
    let mut last_err = None;
    for attempt in 0..5_u32 {
        match fs::rename(&tmp, path) {
            Ok(()) => return Ok(()),
            Err(err) => {
                last_err = Some(err);
                if attempt < 4 {
                    std::thread::sleep(Duration::from_millis(15 * u64::from(attempt + 1)));
                }
            }
        }
    }
    let _ = fs::remove_file(&tmp);
    let err_detail = last_err.map_or_else(
        || "unknown rename failure".to_string(),
        |err| err.to_string(),
    );
    Err(SyncError::Io(format!(
        "failed to commit `{}`: {err_detail}",
        path.display()
    )))
}

pub fn sanitize_remembered_accounts(
    entries: Vec<RememberedServerAccount>,
) -> Vec<RememberedServerAccount> {
    let mut cleaned = Vec::with_capacity(entries.len().min(MAX_REMEMBERED_SERVER_ACCOUNTS));
    for mut entry in entries {
        entry.server_url = normalize_server_url(&entry.server_url)
            .unwrap_or_else(|_| DEFAULT_SERVER_URL.to_string());
        entry.user_id = entry.user_id.trim().to_string();
        entry.username = entry.username.trim().to_string();
        entry.display_name = entry.display_name.trim().to_string();
        entry.email = entry
            .email
            .take()
            .map(|e| e.trim().to_string())
            .filter(|e| !e.is_empty());
        if entry.user_id.is_empty() || entry.username.is_empty() {
            continue;
        }
        let duplicate = cleaned.iter().any(|existing: &RememberedServerAccount| {
            existing.user_id == entry.user_id
                && existing.server_url.eq_ignore_ascii_case(&entry.server_url)
        });
        if !duplicate {
            cleaned.push(entry);
        }
        if cleaned.len() >= MAX_REMEMBERED_SERVER_ACCOUNTS {
            break;
        }
    }
    cleaned
}

fn sanitize_cached_sessions(entries: Vec<ActiveAccountSession>) -> Vec<ActiveAccountSession> {
    let mut cleaned = Vec::with_capacity(entries.len().min(MAX_REMEMBERED_SERVER_ACCOUNTS));
    for mut entry in entries {
        entry.server_url = normalize_server_url(&entry.server_url)
            .unwrap_or_else(|_| DEFAULT_SERVER_URL.to_string());
        entry.user.user_id = entry.user.user_id.trim().to_string();
        entry.user.username = entry.user.username.trim().to_string();
        entry.user.display_name = entry.user.display_name.trim().to_string();
        entry.user.email = entry
            .user
            .email
            .take()
            .map(|e| e.trim().to_string())
            .filter(|e| !e.is_empty());
        entry.device_name = sanitize_device_name(&entry.device_name);
        if entry.user.user_id.is_empty() || entry.user.username.is_empty() {
            continue;
        }
        let duplicate = cleaned.iter().any(|existing: &ActiveAccountSession| {
            existing.user.user_id == entry.user.user_id
                && existing.server_url.eq_ignore_ascii_case(&entry.server_url)
        });
        if !duplicate {
            cleaned.push(entry);
        }
        if cleaned.len() >= MAX_REMEMBERED_SERVER_ACCOUNTS {
            break;
        }
    }
    cleaned
}

pub fn load_global_state(root_dir: &Path) -> GlobalSyncState {
    let device = default_device_name();
    let path = root_dir.join(GLOBAL_SYNC_FILE);
    let Ok(raw) = fs::read_to_string(&path) else {
        return GlobalSyncState::with_defaults(device);
    };
    let Ok(mut parsed) = serde_json::from_str::<GlobalSyncState>(&raw) else {
        log::warn!("Ignoring malformed sync state file `{}`", path.display());
        return GlobalSyncState::with_defaults(device);
    };
    parsed.server_url =
        normalize_server_url(&parsed.server_url).unwrap_or_else(|_| DEFAULT_SERVER_URL.to_string());
    parsed.device_name = sanitize_device_name(&parsed.device_name);
    let _ = parsed.migrate_legacy_session();
    parsed.remembered_server_accounts =
        sanitize_remembered_accounts(std::mem::take(&mut parsed.remembered_server_accounts));
    parsed.cached_server_sessions =
        sanitize_cached_sessions(std::mem::take(&mut parsed.cached_server_sessions));
    for sess in parsed.cached_server_sessions.clone() {
        let already_remembered = parsed.remembered_server_accounts.iter().any(|r| {
            r.user_id == sess.user.user_id && r.server_url.eq_ignore_ascii_case(&sess.server_url)
        });
        if !already_remembered {
            parsed.remember_server_account(&sess);
        }
    }
    if let ActiveWorkspaceTarget::Cloud {
        ref mut server_url,
        ref mut user_id,
    } = parsed.active_workspace
    {
        *server_url =
            normalize_server_url(server_url).unwrap_or_else(|_| DEFAULT_SERVER_URL.to_string());
        *user_id = user_id.trim().to_string();
        if user_id.is_empty() {
            parsed.active_workspace = ActiveWorkspaceTarget::Local;
        }
    }
    for profile in &mut parsed.local_profiles {
        profile.local_profile_id = sanitize_account_dir_segment(&profile.local_profile_id);
        profile.nickname = sanitize_profile_nickname(&profile.nickname);
    }
    if !parsed.server_configured
        && (!parsed.cached_server_sessions.is_empty()
            || !parsed.remembered_server_accounts.is_empty()
            || !parsed.server_url.eq_ignore_ascii_case(DEFAULT_SERVER_URL))
    {
        parsed.server_configured = true;
    }
    parsed.client_id = sanitize_client_id(&parsed.client_id);
    parsed.sanitize_release_notices();
    parsed
}

pub fn save_global_state(root_dir: &Path, state: &GlobalSyncState) -> Result<(), SyncError> {
    let bytes = serde_json::to_vec_pretty(state)
        .map_err(|e| SyncError::Io(format!("failed to serialize global sync state: {e}")))?;
    write_bytes_atomic(&root_dir.join(GLOBAL_SYNC_FILE), &bytes)
}

pub fn load_manifest(library_dir: &Path) -> SyncManifest {
    let path = library_dir.join(SYNC_MANIFEST_FILE);
    let Ok(raw) = fs::read_to_string(&path) else {
        return SyncManifest::default();
    };
    let mut manifest = serde_json::from_str::<SyncManifest>(&raw).unwrap_or_else(|err| {
        log::warn!(
            "Resetting malformed sync manifest `{}`: {err}",
            path.display()
        );
        SyncManifest::default()
    });
    manifest.migrate_if_needed();
    manifest
}

pub fn save_manifest(library_dir: &Path, manifest: &SyncManifest) -> Result<(), SyncError> {
    let bytes = serde_json::to_vec_pretty(manifest)
        .map_err(|e| SyncError::Io(format!("failed to serialize sync manifest: {e}")))?;
    write_bytes_atomic(&library_dir.join(SYNC_MANIFEST_FILE), &bytes)
}

fn count_active_notebook_files(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut count = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                e.eq_ignore_ascii_case("pcraft") || e.eq_ignore_ascii_case("ncraft")
            })
        {
            count = count.saturating_add(1);
        }
    }
    count
}

/// Inspects a client library root directory and returns ONLY aggregate counts of
/// unsynced local accounts and their notebooks (no nicknames or titles are read or returned).
pub fn inspect_local_accounts_in_library(
    root_dir: &Path,
) -> Option<(String, LocalDiscoverySummary)> {
    let session_path = root_dir.join(GLOBAL_SYNC_FILE);
    if !session_path.is_file() {
        return None;
    }
    let global = load_global_state(root_dir);
    let client_id = if global.client_id.is_empty() {
        format!(
            "local-{}",
            sanitize_account_dir_segment(&global.device_name)
        )
    } else {
        global.client_id.clone()
    };
    if global.needs_initial_nickname {
        return Some((client_id, LocalDiscoverySummary::default()));
    }
    let accessible = global.accessible_local_profiles();
    let unsynced_local_accounts = accessible.len();
    let mut total_local_notebooks = 0usize;
    for profile in accessible {
        let dir = profile_library_dir(root_dir, &profile.local_profile_id);
        total_local_notebooks =
            total_local_notebooks.saturating_add(count_active_notebook_files(&dir));
    }
    Some((
        client_id,
        LocalDiscoverySummary {
            unsynced_local_accounts,
            total_local_notebooks,
        },
    ))
}
