use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use papercraft_color::Color;
use papercraft_doc::{
    Document, UserStudioConfig, sanitize_folder_path, sanitize_library_folders,
    sanitize_notebook_id, sanitize_notebook_title,
};
use papercraft_sync::{
    ACCOUNTS_DIR, ActiveAccountSession, EntitySyncStatus, FolderSyncMeta, GLOBAL_SYNC_FILE,
    GlobalSyncState, LocalNotebookSnapshot, LocalProfile, OwnerId, RememberedServerAccount,
    RemoteNotebookUpdate, SyncBatch, SyncConnectionState, SyncCycleOutcome, SyncManifest,
    SyncStatusSummary, default_device_name, load_global_state, load_manifest,
    outcome_from_full_pull, profile_library_dir, sanitize_account_dir_segment, sanitize_client_id,
    sanitize_profile_nickname, sanitize_remembered_accounts, save_global_state, save_manifest,
};
use papercraft_sync_protocol::{
    DEFAULT_SERVER_URL, LocalDiscoveryReport, LocalFolderChange, LocalPrefsPush,
    RemoteFolderRecord, SyncPullResponse, canonical_document_body_hash, canonical_document_hash,
    canonical_prefs_hash, normalize_server_url, now_unix_ms, sanitize_device_name,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::library::{
    LIBRARY_MARKER_FILE, LibraryInitMode, LibraryState, STUDIO_PREFS_FILE, TrashedNotebook,
    count_ncraft_files_in_dir, seed_starter_library_prefs, starter_documents,
};
use crate::{MAX_NOTEBOOKS, Session};

const WEB_CACHE_VERSION: u32 = 3;
const LEGACY_WEB_CACHE_VERSION_1: u32 = 1;
const LEGACY_WEB_CACHE_VERSION_2: u32 = 2;
const TRASH_SUBDIR: &str = "trash";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct CachedTrashEntry {
    pub(crate) doc: Document,
    pub(crate) deleted_at_ms: i64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub(crate) struct CachedLibraryBundle {
    pub(crate) notebooks: Vec<Document>,
    #[serde(default)]
    pub(crate) active_notebook: usize,
    #[serde(default)]
    pub(crate) trash: Vec<CachedTrashEntry>,
    #[serde(default)]
    pub(crate) prefs: UserStudioConfig,
    #[serde(default)]
    pub(crate) manifest: SyncManifest,
    #[serde(default)]
    pub(crate) slot_ids: Vec<String>,
    #[serde(default)]
    pub(crate) slot_dirty_hash: Vec<bool>,
    #[serde(default)]
    pub(crate) slot_placeholder_blank: Vec<bool>,
    #[serde(default)]
    pub(crate) prefs_dirty_pending: bool,
    #[serde(default)]
    pub(crate) last_dirty_at_ms: Option<i64>,
}

/// Persisted browser `localStorage` payload storing ONLY offline local profiles and metadata shortcuts.
/// Server account notebooks, preferences, and auth tokens are never stored locally.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct WebWorkspaceCache {
    version: u32,
    global: GlobalSyncState,
    #[serde(default)]
    guest: Option<CachedLibraryBundle>,
    #[serde(default)]
    local_profiles: HashMap<String, CachedLibraryBundle>,
}

#[derive(Clone, Debug)]
pub(crate) struct SessionSyncState {
    pub(crate) desktop_root: Option<PathBuf>,
    pub(crate) track_manifest: bool,
    pub(crate) web_cache_enabled: bool,
    pub(crate) web_local_bundles: HashMap<String, CachedLibraryBundle>,
    pub(crate) global: GlobalSyncState,
    /// Active authenticated server session kept strictly in RAM (never persisted to disk or localStorage).
    pub(crate) active_server_session: Option<ActiveAccountSession>,
    pub(crate) manifest: SyncManifest,
    pub(crate) slot_ids: Vec<String>,
    pub(crate) slot_dirty_hash: Vec<bool>,
    pub(crate) slot_placeholder_blank: Vec<bool>,
    pub(crate) prefs_dirty_pending: bool,
    pub(crate) connection: SyncConnectionState,
    pub(crate) last_error: Option<String>,
    pub(crate) last_dirty_at_ms: Option<i64>,
    pub(crate) sync_in_flight: bool,
    pub(crate) network_probed_offline: bool,
}

impl Default for SessionSyncState {
    fn default() -> Self {
        Self {
            desktop_root: None,
            track_manifest: false,
            web_cache_enabled: false,
            web_local_bundles: HashMap::new(),
            global: GlobalSyncState::with_defaults(default_device_name()),
            active_server_session: None,
            manifest: SyncManifest::default(),
            slot_ids: vec![String::new()],
            slot_dirty_hash: vec![false],
            slot_placeholder_blank: vec![false],
            prefs_dirty_pending: false,
            connection: SyncConnectionState::SignedOut,
            last_error: None,
            last_dirty_at_ms: None,
            sync_in_flight: false,
            network_probed_offline: false,
        }
    }
}

impl Session {
    /// Returns `true` when local persistence (desktop directory or browser local storage) is enabled.
    pub fn is_sync_persistence_enabled(&self) -> bool {
        self.sync.desktop_root.is_some() || self.sync.web_cache_enabled
    }

    /// Opens (or initializes) a browser-cached local workspace. Server sessions are never auto-restored.
    pub fn open_web_cache(raw_json: Option<&str>) -> Self {
        if let Some(mut cache) = raw_json
            .and_then(|raw| serde_json::from_str::<WebWorkspaceCache>(raw).ok())
            .filter(|c| {
                matches!(
                    c.version,
                    LEGACY_WEB_CACHE_VERSION_1 | LEGACY_WEB_CACHE_VERSION_2 | WEB_CACHE_VERSION
                )
            })
        {
            sanitize_global_state(&mut cache.global);
            migrate_web_cache_if_needed(&mut cache);

            let has_saved =
                !cache.global.needs_initial_nickname && !cache.global.local_profiles.is_empty();
            let profile_id = ensure_default_profile_in_global(&mut cache.global);
            if has_saved || cache.local_profiles.contains_key(&profile_id) {
                let bundle = cache
                    .local_profiles
                    .get(&profile_id)
                    .cloned()
                    .unwrap_or_else(starter_web_bundle);
                let mut session = restore_web_bundle(&bundle);
                session.sync.global = cache.global;
                session.sync.active_server_session = None;
                session.sync.web_cache_enabled = true;
                session.sync.track_manifest = true;
                session.sync.web_local_bundles = cache.local_profiles;
                session.sync.connection = SyncConnectionState::SignedOut;
                session.sync.manifest.owner_id = Some(OwnerId::LocalProfile(profile_id));
                session.sync_reconcile_slots();
                return session;
            }

            let mut session = Self::default();
            session.replace_notebooks_for_library(starter_documents(), None);
            seed_starter_library_prefs(&mut session.prefs);
            cache.global.needs_initial_nickname = true;
            session.sync.global = cache.global;
            session.sync.active_server_session = None;
            session.sync.web_cache_enabled = true;
            session.sync.track_manifest = true;
            session.sync.web_local_bundles = cache.local_profiles;
            session.sync.connection = SyncConnectionState::SignedOut;
            let pid = ensure_default_profile_in_global(&mut session.sync.global);
            session.sync.manifest.owner_id = Some(OwnerId::LocalProfile(pid));
            session.sync_reconcile_slots();
            return session;
        }

        let mut session = Self::default();
        session.replace_notebooks_for_library(starter_documents(), None);
        seed_starter_library_prefs(&mut session.prefs);
        session.sync.global = GlobalSyncState::with_defaults("Web Browser".to_string());
        session.sync.global.needs_initial_nickname = true;
        let pid = ensure_default_profile_in_global(&mut session.sync.global);
        session.sync.active_server_session = None;
        session.sync.web_cache_enabled = true;
        session.sync.track_manifest = true;
        session.sync.connection = SyncConnectionState::SignedOut;
        session.sync.manifest.owner_id = Some(OwnerId::LocalProfile(pid));
        session.sync_reconcile_slots();
        session
    }

    /// Snapshots ONLY local profiles and server-shortcut metadata (never cloud data or tokens).
    pub fn export_web_cache_json(&mut self) -> Result<String, String> {
        self.sync_active_notebook_slot();
        self.save_current_web_bundle();
        let cache = WebWorkspaceCache {
            version: WEB_CACHE_VERSION,
            global: self.sync.global.clone(),
            guest: None,
            local_profiles: self.sync.web_local_bundles.clone(),
        };
        serde_json::to_string(&cache)
            .map_err(|err| format!("failed to serialize web workspace cache: {err}"))
    }

    pub(crate) fn save_current_web_bundle(&mut self) {
        if !self.sync.web_cache_enabled || self.is_viewing_cloud() {
            return;
        }
        let current = self.capture_web_bundle();
        let pid = ensure_default_profile_in_global(&mut self.sync.global);
        self.sync.web_local_bundles.insert(pid, current);
    }

    fn capture_web_bundle(&self) -> CachedLibraryBundle {
        let notebooks = self
            .notebooks
            .iter()
            .map(|doc| {
                let mut d = (**doc).clone();
                d.sync_active_to_pages();
                d
            })
            .collect();
        let trash = self
            .trashed_notebooks
            .iter()
            .map(|entry| CachedTrashEntry {
                doc: (*entry.doc).clone(),
                deleted_at_ms: entry.deleted_at_ms,
            })
            .collect();
        CachedLibraryBundle {
            notebooks,
            active_notebook: self.active_notebook,
            trash,
            prefs: self.prefs.clone(),
            manifest: self.sync.manifest.clone(),
            slot_ids: self.sync.slot_ids.clone(),
            slot_dirty_hash: self.sync.slot_dirty_hash.clone(),
            slot_placeholder_blank: self.sync.slot_placeholder_blank.clone(),
            prefs_dirty_pending: self.sync.prefs_dirty_pending,
            last_dirty_at_ms: self.sync.last_dirty_at_ms,
        }
    }

    /// Opens the desktop library root in offline local mode. Cloud accounts are never cached locally.
    pub fn open_desktop_library(root_dir: PathBuf) -> Self {
        let _ = fs::create_dir_all(&root_dir);
        let global_path = root_dir.join(GLOBAL_SYNC_FILE);
        let first_access = !global_path.exists() && !has_legacy_root_library(&root_dir);
        let legacy_file_cleanup = global_path.is_file()
            && fs::read_to_string(&global_path).is_ok_and(|raw| {
                raw.contains("\"session\"") || raw.contains("\"active_workspace\"")
            });
        let mut global = load_global_state(&root_dir);
        let mut save_global = first_access || legacy_file_cleanup;
        if first_access {
            global.needs_initial_nickname = true;
        }
        if global.client_id.is_empty() {
            global.client_id = Uuid::new_v4().to_string();
            save_global = true;
        }
        if ensure_desktop_default_profile(&root_dir, &mut global) {
            save_global = true;
        }
        cleanup_legacy_cloud_cache_dir(&root_dir);
        if save_global {
            let _ = save_global_state(&root_dir, &global);
        }

        let pid = ensure_default_profile_in_global(&mut global);
        let prof_dir = profile_library_dir(&root_dir, &pid);
        let (mut session, is_placeholder) =
            Self::open_library_mode(prof_dir, LibraryInitMode::EmptyClean);

        session.sync.desktop_root = Some(root_dir);
        session.sync.track_manifest = true;
        session.sync.global = global;
        session.sync.active_server_session = None;
        session.sync.connection = SyncConnectionState::SignedOut;
        session.sync_init_from_disk(is_placeholder, OwnerId::LocalProfile(pid));
        session
    }

    pub fn is_signed_in(&self) -> bool {
        self.sync.active_server_session.is_some()
    }

    pub fn is_viewing_cloud(&self) -> bool {
        self.sync.active_server_session.is_some()
    }

    pub fn local_profiles(&self) -> &[LocalProfile] {
        &self.sync.global.local_profiles
    }

    /// Returns only finalized local accounts saved on this device (empty before initial nickname setup).
    pub fn saved_local_profiles(&self) -> &[LocalProfile] {
        self.sync.global.saved_local_profiles()
    }

    /// Returns all saved local accounts on this device.
    pub fn accessible_local_profiles(&self) -> Vec<LocalProfile> {
        self.sync
            .global
            .accessible_local_profiles()
            .into_iter()
            .cloned()
            .collect()
    }

    pub fn remembered_server_accounts(&self) -> &[RememberedServerAccount] {
        &self.sync.global.remembered_server_accounts
    }

    pub fn has_saved_local_accounts(&self) -> bool {
        self.sync.global.has_saved_local_accounts()
    }

    pub fn active_local_profile(&self) -> Option<&LocalProfile> {
        self.sync.global.active_local_profile()
    }

    pub fn desktop_root_dir(&self) -> Option<&Path> {
        self.sync.desktop_root.as_deref()
    }

    pub fn active_account_session(&self) -> Option<&ActiveAccountSession> {
        self.sync.active_server_session.as_ref()
    }

    pub fn server_url(&self) -> &str {
        self.sync
            .active_server_session
            .as_ref()
            .map_or(self.sync.global.server_url.as_str(), |s| {
                s.server_url.as_str()
            })
    }

    pub fn device_name(&self) -> &str {
        &self.sync.global.device_name
    }

    pub fn client_id(&self) -> &str {
        &self.sync.global.client_id
    }

    pub fn needs_initial_nickname(&self) -> bool {
        !self.is_viewing_cloud() && self.sync.global.needs_initial_nickname
    }

    /// Returns `true` when the release welcome banner has not yet been dismissed
    /// for `current_version` on a persisted Desktop or Web workspace.
    pub fn should_show_welcome_banner_for_version(&self, current_version: &str) -> bool {
        if !self.is_sync_persistence_enabled() {
            return false;
        }
        let Some(target) = sanitize_welcome_version(current_version) else {
            return false;
        };
        self.sync.global.welcome_banner_seen_version.as_deref() != Some(target.as_str())
    }

    /// Returns `true` when the release welcome banner has not yet been dismissed
    /// for the running build's version on a persisted Desktop or Web workspace.
    pub fn should_show_welcome_banner(&self) -> bool {
        self.should_show_welcome_banner_for_version(env!("CARGO_PKG_VERSION"))
    }

    /// Returns the last release version for which the welcome banner was dismissed.
    pub fn welcome_banner_seen_version(&self) -> Option<&str> {
        self.sync.global.welcome_banner_seen_version.as_deref()
    }

    /// Marks the release welcome banner as seen for `current_version` and persists global state.
    pub fn dismiss_welcome_banner_for_version(&mut self, current_version: &str) {
        let Some(target) = sanitize_welcome_version(current_version) else {
            return;
        };
        if self.sync.global.welcome_banner_seen
            && self.sync.global.welcome_banner_seen_version.as_deref() == Some(target.as_str())
        {
            return;
        }
        self.sync.global.welcome_banner_seen = true;
        self.sync.global.welcome_banner_seen_version = Some(target);
        self.sync_persist_global_state();
        self.save_current_web_bundle();
        self.bump_revision();
    }

    /// Marks the release welcome banner as seen for the running build's version.
    pub fn dismiss_welcome_banner(&mut self) {
        self.dismiss_welcome_banner_for_version(env!("CARGO_PKG_VERSION"));
    }

    /// Returns `true` when the first-notebook data & privacy notice has not yet been dismissed
    /// on a persisted Desktop or Web workspace.
    pub fn should_show_first_notebook_data_notice(&self) -> bool {
        if !self.is_sync_persistence_enabled() {
            return false;
        }
        !self.sync.global.first_notebook_data_notice_seen
    }

    /// Marks the first-notebook data & privacy notice as seen and persists global state.
    pub fn dismiss_first_notebook_data_notice(&mut self) {
        if self.sync.global.first_notebook_data_notice_seen {
            return;
        }
        self.sync.global.first_notebook_data_notice_seen = true;
        self.sync_persist_global_state();
        self.save_current_web_bundle();
        self.bump_revision();
    }

    pub fn set_network_offline_state(&mut self, offline: bool) {
        self.sync.network_probed_offline = offline;
    }

    pub(crate) fn touch_cloud_dirty_clock_if_needed(&mut self) {
        if self.is_viewing_cloud() {
            self.sync.last_dirty_at_ms.get_or_insert_with(now_unix_ms);
        }
    }

    /// Returns ONLY aggregate counts of saved local accounts and their notebooks (no PII).
    pub fn local_discovery_report(&self) -> LocalDiscoveryReport {
        if self.sync.global.needs_initial_nickname {
            return LocalDiscoveryReport {
                client_id: self.sync.global.client_id.clone(),
                unsynced_local_accounts: 0,
                total_local_notebooks: 0,
            };
        }
        let saved = self.sync.global.saved_local_profiles();
        let unsynced_local_accounts = saved.len();
        let total_local_notebooks = saved
            .iter()
            .map(|p| self.count_local_profile_notebooks(&p.local_profile_id))
            .sum();
        LocalDiscoveryReport {
            client_id: self.sync.global.client_id.clone(),
            unsynced_local_accounts,
            total_local_notebooks,
        }
    }

    /// Completes the local nickname prompt and starts a local session on this device.
    pub fn complete_first_access_nickname(
        &mut self,
        nickname_raw: &str,
    ) -> Result<LocalProfile, String> {
        if nickname_raw.trim().is_empty() {
            return Err("Please enter a nickname to start your session.".to_string());
        }
        let clean_nick = sanitize_profile_nickname(nickname_raw);
        let target_id = ensure_default_profile_in_global(&mut self.sync.global);
        let now = now_unix_ms();
        let mut updated_profile = None;
        for prof in &mut self.sync.global.local_profiles {
            if prof.local_profile_id == target_id {
                prof.nickname = clean_nick.clone();
                prof.last_used_at_ms = now;
                updated_profile = Some(prof.clone());
                break;
            }
        }
        self.sync.global.needs_initial_nickname = false;
        self.sync_persist_global_state();
        if self.sync.desktop_root.is_some() {
            if self.library.dir.is_none() {
                self.load_local_profile_workspace(&target_id);
            } else {
                self.bump_revision();
            }
        } else if self.sync.web_cache_enabled {
            if self.notebooks.is_empty() {
                self.load_local_profile_workspace(&target_id);
            }
            self.save_current_web_bundle();
            self.bump_revision();
        } else {
            if self.notebooks.is_empty() {
                self.replace_notebooks_for_library(starter_documents(), None);
                seed_starter_library_prefs(&mut self.prefs);
                self.sync.manifest.owner_id = Some(OwnerId::LocalProfile(target_id));
                self.sync_reconcile_slots();
            }
            self.bump_revision();
        }
        updated_profile.ok_or_else(|| "failed to initialize local profile".to_string())
    }

    /// Returns the explicit status badge for notebook slot `index`.
    pub fn notebook_sync_status(&self, index: usize) -> EntitySyncStatus {
        if !self.is_viewing_cloud() {
            return EntitySyncStatus::LocalOnly;
        }
        if self
            .sync
            .slot_placeholder_blank
            .get(index)
            .copied()
            .unwrap_or(false)
        {
            return EntitySyncStatus::LocalOnly;
        }
        if self
            .sync
            .slot_dirty_hash
            .get(index)
            .copied()
            .unwrap_or(false)
        {
            return EntitySyncStatus::PendingPush;
        }
        let Some(id) = self.sync.slot_ids.get(index) else {
            return EntitySyncStatus::PendingPush;
        };
        match self.sync.manifest.meta(id) {
            Some(meta) => meta.effective_status(true),
            None => EntitySyncStatus::PendingPush,
        }
    }

    /// Counts pending real-time server writes for the active server session (`0` when offline/local).
    pub fn cloud_pending_changes_count(&self) -> usize {
        if !self.is_viewing_cloud() {
            return 0;
        }
        self.compute_pending_changes_fast()
    }

    pub fn sync_status_summary(&self) -> SyncStatusSummary {
        let viewing_cloud = self.is_viewing_cloud();
        let pending_changes = self.cloud_pending_changes_count();
        let (username, email, display_name) = match self.sync.active_server_session.as_ref() {
            Some(acct) => (
                Some(acct.user.username.clone()),
                acct.user.email.clone(),
                Some(acct.user.display_name.clone()),
            ),
            None => (None, None, None),
        };
        let active_prof = if viewing_cloud || self.sync.global.needs_initial_nickname {
            None
        } else {
            self.sync.global.active_local_profile()
        };
        SyncStatusSummary {
            signed_in: viewing_cloud,
            viewing_cloud,
            server_url: self.server_url().to_string(),
            device_name: self.sync.global.device_name.clone(),
            username,
            email,
            display_name,
            active_local_profile_id: active_prof.map(|p| p.local_profile_id.clone()),
            active_local_profile_name: active_prof.map(|p| p.nickname.clone()),
            connection: self.sync.connection,
            pending_changes,
            last_sync_at_ms: self.sync.manifest.last_sync_at_ms,
            last_error: self.sync.last_error.clone(),
        }
    }

    pub fn set_sync_in_flight(&mut self, in_flight: bool) {
        self.sync.sync_in_flight = in_flight;
        if !self.is_viewing_cloud() {
            self.sync.connection = SyncConnectionState::SignedOut;
        } else if in_flight {
            self.sync.connection = SyncConnectionState::Syncing;
        }
    }

    pub fn is_sync_in_flight(&self) -> bool {
        self.sync.sync_in_flight
    }

    pub fn last_dirty_at_ms(&self) -> Option<i64> {
        self.sync.last_dirty_at_ms
    }

    pub fn record_sync_failure(&mut self, offline: bool, message: String) {
        self.sync.sync_in_flight = false;
        self.sync.network_probed_offline = offline;
        self.sync.connection = if !self.is_viewing_cloud() {
            SyncConnectionState::SignedOut
        } else if offline {
            SyncConnectionState::Offline
        } else {
            SyncConnectionState::Error
        };
        self.sync.last_error = Some(message);
    }

    pub fn update_active_session_tokens(&mut self, updated: ActiveAccountSession) {
        if self
            .sync
            .active_server_session
            .as_ref()
            .is_some_and(|cur| cur == &updated)
        {
            return;
        }
        self.sync.global.server_url = updated.server_url.clone();
        self.sync.global.remember_server_account(&updated);
        self.sync.active_server_session = Some(updated);
        self.sync_persist_global_state();
        self.bump_revision();
    }

    pub fn set_server_url_preference(&mut self, raw_url: &str) -> Result<String, String> {
        let normalized = normalize_server_url(raw_url).map_err(|e| e.to_string())?;
        self.sync.global.server_url = normalized.clone();
        self.sync_persist_global_state();
        Ok(normalized)
    }

    /// Removes a remembered server account shortcut from this device without touching server data.
    pub fn forget_server_account(
        &mut self,
        server_url: &str,
        user_id: &str,
    ) -> Result<bool, String> {
        if user_id.trim().is_empty() {
            return Err("server user_id must not be empty".to_string());
        }
        let norm_url = normalize_server_url(server_url).map_err(|e| e.to_string())?;
        let removed = self.sync.global.forget_server_account(&norm_url, user_id);
        self.sync_persist_global_state();
        if removed {
            self.bump_revision();
        }
        Ok(removed)
    }

    /// Creates a new isolated local profile stored exclusively on this device.
    pub fn create_local_profile(
        &mut self,
        nickname_raw: &str,
        switch_now: bool,
    ) -> Result<LocalProfile, String> {
        if nickname_raw.trim().is_empty() {
            return Err("Please enter a nickname for the local account.".to_string());
        }
        let nickname = sanitize_profile_nickname(nickname_raw);
        let now = now_unix_ms();
        let was_uninitialized = self.sync.global.needs_initial_nickname;
        if was_uninitialized {
            self.remove_unconfirmed_local_placeholders();
        }
        let profile = LocalProfile {
            local_profile_id: Uuid::new_v4().to_string(),
            nickname,
            created_at_ms: now,
            last_used_at_ms: now,
        };
        self.sync.global.local_profiles.push(profile.clone());
        self.sync.global.needs_initial_nickname = false;
        self.sync_persist_global_state();
        if switch_now {
            self.switch_to_local_profile(&profile.local_profile_id)?;
        } else {
            self.bump_revision();
        }
        Ok(profile)
    }

    /// Renames an existing saved local profile without modifying its notebooks or ID.
    pub fn rename_local_profile(
        &mut self,
        profile_id: &str,
        nickname_raw: &str,
    ) -> Result<(), String> {
        if nickname_raw.trim().is_empty() {
            return Err("Nickname must not be empty.".to_string());
        }
        if self.sync.global.needs_initial_nickname {
            return Err("local profile not found".to_string());
        }
        let Some(prof) = self
            .sync
            .global
            .local_profiles
            .iter_mut()
            .find(|p| p.local_profile_id == profile_id)
        else {
            return Err("local profile not found".to_string());
        };
        prof.nickname = sanitize_profile_nickname(nickname_raw);
        self.sync_persist_global_state();
        self.bump_revision();
        Ok(())
    }

    /// Deletes a local profile from this device (allowed anytime while in a cloud session, or while
    /// at least one other local profile remains when viewing locally).
    pub fn delete_local_profile(&mut self, profile_id: &str) -> Result<(), String> {
        if self.sync.global.needs_initial_nickname {
            return Err("local profile not found".to_string());
        }
        if !self.is_viewing_cloud() && self.sync.global.local_profiles.len() <= 1 {
            return Err("at least one local profile must stay on this device".to_string());
        }
        let Some(pos) = self
            .sync
            .global
            .local_profiles
            .iter()
            .position(|p| p.local_profile_id == profile_id)
        else {
            return Err("local profile not found".to_string());
        };
        let was_active_local =
            !self.is_viewing_cloud() && self.sync.global.active_local_profile_id == profile_id;
        self.sync.global.local_profiles.remove(pos);
        self.remove_local_profile_disk_and_web_data(profile_id);
        if self.sync.global.local_profiles.is_empty() {
            self.sync.global.active_local_profile_id.clear();
            self.sync.global.needs_initial_nickname = true;
        } else if self.sync.global.active_local_profile_id == profile_id {
            self.sync.global.active_local_profile_id = self
                .sync
                .global
                .local_profiles
                .first()
                .map(|p| p.local_profile_id.clone())
                .unwrap_or_default();
        }
        self.sync_persist_global_state();
        if was_active_local && !self.sync.global.local_profiles.is_empty() {
            let fallback_id = self.sync.global.active_local_profile_id.clone();
            self.load_local_profile_workspace(&fallback_id);
        } else {
            self.bump_revision();
        }
        Ok(())
    }

    /// Switches into a saved local profile on this device, strictly closing any online server session.
    pub fn switch_to_local_profile(&mut self, profile_id: &str) -> Result<(), String> {
        if self.sync.global.needs_initial_nickname {
            return Err("local profile not found".to_string());
        }
        if !self
            .sync
            .global
            .local_profiles
            .iter()
            .any(|p| p.local_profile_id == profile_id)
        {
            return Err("local profile not found".to_string());
        }
        if !self.is_viewing_cloud() {
            self.flush_active_workspace_to_storage();
        } else {
            // Strict Online/Offline separation: drop in-memory server session without caching cloud data.
            self.sync.active_server_session = None;
            self.sync.last_error = None;
            self.sync.sync_in_flight = false;
            self.sync.last_dirty_at_ms = None;
        }
        if let Some(prof) = self
            .sync
            .global
            .local_profiles
            .iter_mut()
            .find(|p| p.local_profile_id == profile_id)
        {
            prof.last_used_at_ms = now_unix_ms();
        }
        self.sync.global.active_local_profile_id = profile_id.to_string();
        self.sync_persist_global_state();
        self.load_local_profile_workspace(profile_id);
        Ok(())
    }

    /// Enters a live online server session strictly in RAM (zero local cache or token persistence),
    /// hydrating notebooks, folders, and preferences directly from `initial_pull`.
    pub fn enter_online_server_session(
        &mut self,
        mut auth: ActiveAccountSession,
        initial_pull: SyncPullResponse,
    ) {
        auth.server_url =
            normalize_server_url(&auth.server_url).unwrap_or_else(|_| auth.server_url.clone());
        if !self.is_viewing_cloud() {
            if self.sync.global.needs_initial_nickname {
                self.remove_unconfirmed_local_placeholders();
            } else if let Some(OwnerId::LocalProfile(ref cur_pid)) = self.sync.manifest.owner_id
                && self
                    .sync
                    .global
                    .local_profiles
                    .iter()
                    .any(|p| p.local_profile_id == *cur_pid)
            {
                self.flush_active_workspace_to_storage();
            }
        }

        if let Some(ref root) = self.sync.desktop_root {
            cleanup_legacy_cloud_cache_dir(root);
        }

        self.sync.global.server_url = auth.server_url.clone();
        self.sync.global.device_name = auth.device_name.clone();
        self.sync.global.remember_server_account(&auth);
        self.sync_persist_global_state();

        let keep_root = self.sync.desktop_root.clone();
        let keep_web_enabled = self.sync.web_cache_enabled;
        let keep_web_bundles = self.sync.web_local_bundles.clone();
        let keep_global = self.sync.global.clone();

        let outcome = outcome_from_full_pull(initial_pull, None);
        let owner = OwnerId::CloudUser(auth.user.user_id.clone());

        let mut next = Self {
            notebooks: Vec::new(),
            active_notebook: 0,
            trashed_notebooks: Vec::new(),
            library: LibraryState::default(), // `dir == None`: ZERO local disk writes!
            ..Default::default()
        };
        let mut blank = Document::default();
        blank.sync_active_to_pages();
        next.doc = Arc::new(blank);

        if let Some(ref rp) = outcome.updated_prefs {
            next.prefs = rp.prefs.clone().sanitized();
            next.sync.manifest.prefs_version = rp.version;
            next.sync.manifest.prefs_hash = rp.content_hash.clone();
        } else {
            next.prefs = UserStudioConfig::default();
        }
        next.apply_pen_size_preferences();

        next.sync.desktop_root = keep_root;
        next.sync.web_cache_enabled = keep_web_enabled;
        next.sync.web_local_bundles = keep_web_bundles;
        next.sync.global = keep_global;
        next.sync.track_manifest = true;
        next.sync.manifest.owner_id = Some(owner);
        next.sync.manifest.last_server_cursor = outcome.new_cursor;
        next.sync.manifest.last_sync_at_ms = Some(outcome.synced_at_ms);
        next.sync.manifest.prefs_dirty = false;
        next.sync.slot_ids.clear();
        next.sync.slot_dirty_hash.clear();
        next.sync.slot_placeholder_blank.clear();

        for update in &outcome.downloaded_updates {
            if !update.deleted {
                let _ = next.apply_remote_upsert(update, outcome.synced_at_ms);
            }
        }
        let _ = next.apply_remote_folders(&outcome.downloaded_folders);
        next.sync_reconcile_folders();

        if !next.notebooks.is_empty() {
            next.active_notebook = 0;
            next.doc = Arc::clone(&next.notebooks[0]);
        }
        next.history.clear();
        next.sync.active_server_session = Some(auth);
        next.sync.connection = SyncConnectionState::Synced;
        next.sync.network_probed_offline = false;
        next.sync.sync_in_flight = false;
        next.sync.last_error = None;
        next.sync.last_dirty_at_ms = None;
        next.sync.prefs_dirty_pending = false;

        *self = next;
        self.bump_revision();
    }

    /// Drops the active in-memory server session and clears all cloud data from RAM.
    /// Returns `true` if one or more saved local accounts exist on this device.
    pub fn sign_out_from_server_session(&mut self) -> bool {
        self.sync.active_server_session = None;
        self.sync.sync_in_flight = false;
        self.sync.last_error = None;
        self.sync.last_dirty_at_ms = None;
        self.sync.prefs_dirty_pending = false;
        self.sync.connection = SyncConnectionState::SignedOut;

        let keep_root = self.sync.desktop_root.clone();
        let keep_web_enabled = self.sync.web_cache_enabled;
        let keep_web_bundles = self.sync.web_local_bundles.clone();
        let mut keep_global = self.sync.global.clone();
        if keep_global.local_profiles.is_empty() {
            keep_global.needs_initial_nickname = true;
        }
        let has_locals = keep_global.has_saved_local_accounts();

        let mut next = Self::default();
        next.notebooks.clear();
        next.library = LibraryState::default();
        let mut blank = Document::default();
        blank.sync_active_to_pages();
        next.doc = Arc::new(blank);
        next.sync.desktop_root = keep_root;
        next.sync.web_cache_enabled = keep_web_enabled;
        next.sync.web_local_bundles = keep_web_bundles;
        next.sync.global = keep_global;
        next.sync.track_manifest = true;
        next.sync.connection = SyncConnectionState::SignedOut;

        *self = next;
        self.sync_persist_global_state();
        self.bump_revision();
        has_locals
    }

    /// Captures all notebooks, folders, and studio preferences from `source_profile_id` for
    /// atomic upload when converting a local account to a cloud account (does NOT delete local data).
    pub fn prepare_local_account_conversion_batch(
        &mut self,
        source_profile_id: &str,
    ) -> Result<(String, SyncBatch), String> {
        if self.sync.global.needs_initial_nickname {
            return Err("Local account not found on this device.".to_string());
        }
        let Some(profile) = self
            .sync
            .global
            .local_profiles
            .iter()
            .find(|p| p.local_profile_id == source_profile_id)
            .cloned()
        else {
            return Err("Local account not found on this device.".to_string());
        };

        if !self.is_viewing_cloud()
            && self
                .active_local_profile()
                .is_some_and(|p| p.local_profile_id == source_profile_id)
        {
            self.flush_active_workspace_to_storage();
        }

        let captured = self.read_local_profile_payload(source_profile_id);
        let now = now_unix_ms();
        let mut local_docs = Vec::with_capacity(captured.docs.len());
        for mut doc in captured.docs {
            doc.id = sanitize_notebook_id(&doc.id);
            if doc.id.is_empty() {
                doc.id = Uuid::new_v4().to_string();
            }
            doc.updated_at_ms = doc.updated_at_ms.max(now);
            doc.meta_updated_at_ms = doc.meta_updated_at_ms.max(doc.updated_at_ms);
            doc.sync_active_to_pages();
            let id = doc.id.clone();
            let content_hash = canonical_document_hash(&doc);
            local_docs.push(LocalNotebookSnapshot {
                notebook_id: id,
                base_version: 0,
                cloud_dirty: true,
                content_hash,
                doc,
            });
        }

        let clean_prefs = captured.prefs.sanitized();
        let folder_paths = sanitize_library_folders(&clean_prefs.library_folders);
        let folder_changes: Vec<LocalFolderChange> = folder_paths
            .into_iter()
            .map(|path| LocalFolderChange {
                folder_id: Uuid::new_v4().to_string(),
                base_version: 0,
                color_hex: clean_prefs.folder_color(&path).map(Color::to_hex),
                path,
                deleted: false,
                updated_at_ms: now,
                deleted_at_ms: None,
            })
            .collect();

        let prefs_push = Some(LocalPrefsPush {
            base_version: 0,
            content_hash: canonical_prefs_hash(&clean_prefs),
            prefs: clean_prefs,
        });

        Ok((
            profile.nickname,
            SyncBatch {
                client_cursor: 0,
                device_name: self.sync.global.device_name.clone(),
                local_docs,
                tombstones: Vec::new(),
                folder_changes,
                prefs_push,
                force_overwrite: true,
            },
        ))
    }

    /// Finalizes local-to-cloud conversion after the server upload succeeds: deletes ALL local
    /// notebooks/prefs for `source_profile_id` and transitions directly into the live server session.
    pub fn complete_local_account_conversion(
        &mut self,
        source_profile_id: &str,
        auth: ActiveAccountSession,
        initial_pull: SyncPullResponse,
    ) {
        self.remove_local_profile_storage(source_profile_id);
        if self.sync.global.local_profiles.is_empty() {
            self.sync.global.active_local_profile_id.clear();
            self.sync.global.needs_initial_nickname = true;
        }
        self.enter_online_server_session(auth, initial_pull);
    }

    /// Returns the number of notebooks stored in a saved local profile on this device.
    pub fn local_profile_notebook_count(&self, profile_id: &str) -> usize {
        if self.sync.global.needs_initial_nickname
            || !self
                .sync
                .global
                .local_profiles
                .iter()
                .any(|p| p.local_profile_id == profile_id)
        {
            return 0;
        }
        self.count_local_profile_notebooks(profile_id)
    }

    fn count_local_profile_notebooks(&self, profile_id: &str) -> usize {
        if !self.is_viewing_cloud()
            && self
                .active_local_profile()
                .is_some_and(|p| p.local_profile_id == profile_id)
        {
            return count_non_placeholder_slots(
                self.notebooks.len(),
                &self.sync.slot_placeholder_blank,
            );
        }
        if let Some(ref root) = self.sync.desktop_root {
            let dir = profile_library_dir(root, profile_id);
            return count_ncraft_files_in_dir(&dir);
        }
        if self.sync.web_cache_enabled
            && let Some(bundle) = self.sync.web_local_bundles.get(profile_id)
        {
            return count_non_placeholder_slots(
                bundle.notebooks.len(),
                &bundle.slot_placeholder_blank,
            );
        }
        0
    }

    fn flush_active_workspace_to_storage(&mut self) {
        if self.is_viewing_cloud() {
            return;
        }
        self.sync_active_notebook_slot();
        self.sync_reconcile_folders();
        let _ = self.save_library();
        let _ = self.save_prefs();
        self.sync_persist_manifest();
        self.save_current_web_bundle();
    }

    fn load_local_profile_workspace(&mut self, profile_id: &str) {
        let owner = OwnerId::LocalProfile(profile_id.to_string());
        if let Some(root) = self.sync.desktop_root.clone() {
            let keep_global = self.sync.global.clone();
            let prof_dir = profile_library_dir(&root, profile_id);
            let (next_session, is_placeholder) =
                Self::open_library_mode(prof_dir, LibraryInitMode::EmptyClean);
            *self = next_session;
            self.sync.desktop_root = Some(root);
            self.sync.track_manifest = true;
            self.sync.global = keep_global;
            self.sync.active_server_session = None;
            self.sync.connection = SyncConnectionState::SignedOut;
            self.sync_init_from_disk(is_placeholder, owner);
            self.bump_revision();
            return;
        }
        if self.sync.web_cache_enabled {
            let bundle = self
                .sync
                .web_local_bundles
                .get(profile_id)
                .cloned()
                .unwrap_or_else(starter_web_bundle);
            let mut next = restore_web_bundle(&bundle);
            next.sync.global = self.sync.global.clone();
            next.sync.active_server_session = None;
            next.sync.web_cache_enabled = true;
            next.sync.track_manifest = true;
            next.sync.web_local_bundles = self.sync.web_local_bundles.clone();
            next.sync.connection = SyncConnectionState::SignedOut;
            next.sync.manifest.owner_id = Some(owner);
            *self = next;
            self.sync_reconcile_slots();
            self.bump_revision();
            return;
        }
        self.sync.active_server_session = None;
        self.sync.manifest.owner_id = Some(owner);
        self.sync.connection = SyncConnectionState::SignedOut;
        self.bump_revision();
    }

    fn read_local_profile_payload(&mut self, source_profile_id: &str) -> CapturedProfileData {
        if !self.is_viewing_cloud()
            && self
                .active_local_profile()
                .is_some_and(|p| p.local_profile_id == source_profile_id)
        {
            self.sync_active_notebook_slot();
            self.sync_reconcile_slots();
            let docs = self
                .notebooks
                .iter()
                .enumerate()
                .filter(|(idx, _)| {
                    !self
                        .sync
                        .slot_placeholder_blank
                        .get(*idx)
                        .copied()
                        .unwrap_or(false)
                })
                .map(|(_, doc)| (**doc).clone())
                .collect();
            return CapturedProfileData {
                docs,
                prefs: self.prefs.clone(),
            };
        }

        let mut docs = Vec::new();
        let mut prefs = UserStudioConfig::default();

        if let Some(ref root) = self.sync.desktop_root {
            let src_dir = profile_library_dir(root, source_profile_id);
            let (mut src_sess, is_placeholder) =
                Self::open_library_mode(src_dir, LibraryInitMode::BlankPlaceholderOnly);
            if !is_placeholder {
                src_sess.sync_reconcile_slots();
                docs = src_sess.notebooks.iter().map(|d| (**d).clone()).collect();
            }
            prefs = src_sess.prefs.clone();
        } else if self.sync.web_cache_enabled
            && let Some(bundle) = self.sync.web_local_bundles.get(source_profile_id)
        {
            for (idx, doc) in bundle.notebooks.iter().enumerate() {
                if bundle
                    .slot_placeholder_blank
                    .get(idx)
                    .copied()
                    .unwrap_or(false)
                {
                    continue;
                }
                let mut d = doc.clone();
                if d.id.is_empty() {
                    d.id = Uuid::new_v4().to_string();
                }
                docs.push(d);
            }
            prefs = bundle.prefs.clone();
        }

        CapturedProfileData { docs, prefs }
    }

    fn remove_local_profile_disk_and_web_data(&mut self, profile_id: &str) {
        self.sync.web_local_bundles.remove(profile_id);
        if let Some(ref root) = self.sync.desktop_root {
            let dir = profile_library_dir(root, profile_id);
            if dir.exists() {
                let _ = fs::remove_dir_all(dir);
            }
        }
    }

    fn remove_local_profile_storage(&mut self, source_profile_id: &str) {
        if let Some(pos) = self
            .sync
            .global
            .local_profiles
            .iter()
            .position(|p| p.local_profile_id == source_profile_id)
        {
            self.sync.global.local_profiles.remove(pos);
        }
        if self.sync.global.active_local_profile_id == source_profile_id {
            self.sync.global.active_local_profile_id = self
                .sync
                .global
                .local_profiles
                .first()
                .map(|p| p.local_profile_id.clone())
                .unwrap_or_default();
        }
        self.remove_local_profile_disk_and_web_data(source_profile_id);
    }

    fn remove_unconfirmed_local_placeholders(&mut self) {
        let ids: Vec<String> = self
            .sync
            .global
            .local_profiles
            .iter()
            .map(|p| p.local_profile_id.clone())
            .collect();
        self.sync.global.local_profiles.clear();
        self.sync.global.active_local_profile_id.clear();
        for pid in ids {
            self.remove_local_profile_disk_and_web_data(&pid);
        }
    }

    /// Captures a lock-free snapshot of dirty notebooks, folders, and tombstones for live server writes.
    pub fn prepare_sync_batch(&mut self) -> Option<SyncBatch> {
        let _ = self.sync.active_server_session.as_ref()?;
        self.sync_active_notebook_slot();
        self.sync_reconcile_slots();
        self.sync_reconcile_folders();

        let count = self.notebooks.len();
        self.ensure_sync_vec_lens(count);

        let mut local_docs = Vec::new();
        for idx in 0..count {
            if self.sync.slot_placeholder_blank[idx] {
                continue;
            }
            let id = self.sync.slot_ids[idx].clone();
            let slot_dirty = self.sync.slot_dirty_hash[idx];
            let needs_check = slot_dirty
                || self
                    .sync
                    .manifest
                    .meta(&id)
                    .is_none_or(|m| m.cloud_dirty || m.server_version == 0);
            if !needs_check {
                continue;
            }

            let doc_arc = &self.notebooks[idx];
            let content_hash = canonical_document_hash(doc_arc);
            if slot_dirty || self.sync.manifest.meta(&id).is_none() {
                self.sync.manifest.mark_notebook_dirty(&id, &content_hash);
                self.sync.slot_dirty_hash[idx] = false;
            }

            if let Some(meta) = self.sync.manifest.meta(&id)
                && (meta.cloud_dirty || meta.server_version == 0)
            {
                local_docs.push(LocalNotebookSnapshot {
                    notebook_id: id,
                    base_version: meta.server_version,
                    cloud_dirty: true,
                    content_hash,
                    doc: (**doc_arc).clone(),
                });
            }
        }

        let folder_changes: Vec<LocalFolderChange> = self
            .sync
            .manifest
            .folders
            .iter()
            .filter(|f| f.cloud_dirty || f.server_version == 0 || f.deleted_at_ms.is_some())
            .map(|f| LocalFolderChange {
                folder_id: f.folder_id.clone(),
                base_version: f.server_version,
                path: f.path.clone(),
                color_hex: f.color_hex.clone(),
                deleted: f.deleted_at_ms.is_some(),
                updated_at_ms: f.updated_at_ms,
                deleted_at_ms: f.deleted_at_ms,
            })
            .collect();

        if self.sync.prefs_dirty_pending {
            let hash = canonical_prefs_hash(&self.prefs);
            self.sync.manifest.mark_prefs_dirty(&hash);
            self.sync.prefs_dirty_pending = false;
        }

        let prefs_push = if self.sync.manifest.prefs_dirty {
            Some(LocalPrefsPush {
                base_version: self.sync.manifest.prefs_version,
                content_hash: canonical_prefs_hash(&self.prefs),
                prefs: self.prefs.clone(),
            })
        } else {
            None
        };

        Some(SyncBatch {
            client_cursor: self.sync.manifest.last_server_cursor,
            device_name: self.sync.global.device_name.clone(),
            local_docs,
            tombstones: self.sync.manifest.tombstones.clone(),
            folder_changes,
            prefs_push,
            force_overwrite: true,
        })
    }

    /// Applies the completed real-time server outcome onto the live Session (never touches disk).
    pub fn apply_sync_outcome(&mut self, outcome: SyncCycleOutcome) -> Result<(), String> {
        if self.live_stroke.is_some() || self.live_eraser.is_some() {
            return Err("cannot apply server update during an active stroke".to_string());
        }
        if !self.is_viewing_cloud() {
            return Err("cannot apply server sync outcome while using a local profile".to_string());
        }
        self.sync_active_notebook_slot();

        if let Some(refreshed) = outcome.refreshed_session {
            self.sync.global.server_url = refreshed.server_url.clone();
            self.sync.global.remember_server_account(&refreshed);
            self.sync.active_server_session = Some(refreshed);
            self.sync_persist_global_state();
        } else if let Some(sess) = self.sync.active_server_session.as_mut()
            && sess.user != outcome.server_user
        {
            sess.user = outcome.server_user.clone();
            let sess_clone = sess.clone();
            self.sync.global.remember_server_account(&sess_clone);
            self.sync_persist_global_state();
        }

        self.sync.manifest.last_server_cursor = outcome.new_cursor;
        self.ack_accepted_uploads(&outcome.accepted_uploads, outcome.synced_at_ms);
        self.ack_accepted_folders(&outcome.accepted_folders);

        for deleted_id in &outcome.cleared_tombstones {
            self.sync.manifest.clear_tombstone(deleted_id);
        }

        let mut updated_any_doc = false;
        for update in &outcome.downloaded_updates {
            if !update.deleted && self.apply_remote_upsert(update, outcome.synced_at_ms) {
                updated_any_doc = true;
            }
        }
        for update in &outcome.downloaded_updates {
            if update.deleted && self.apply_remote_delete(&update.notebook_id) {
                updated_any_doc = true;
            }
        }
        if self.apply_remote_folders(&outcome.downloaded_folders) {
            updated_any_doc = true;
        }

        if let Some((ver, ref sent_hash)) = outcome.ack_prefs {
            let cur_hash = canonical_prefs_hash(&self.prefs);
            self.sync.manifest.prefs_version = ver;
            self.sync.manifest.prefs_hash = sent_hash.clone();
            self.sync.manifest.prefs_dirty =
                self.sync.prefs_dirty_pending || cur_hash != *sent_hash;
        }
        if let Some(ref rp) = outcome.updated_prefs
            && !self.sync.manifest.prefs_dirty
            && !self.sync.prefs_dirty_pending
        {
            self.apply_remote_prefs_preserving_folders(&rp.prefs);
            self.sync.manifest.prefs_version = rp.version;
            self.sync.manifest.prefs_hash = rp.content_hash.clone();
            self.sync.manifest.prefs_dirty = false;
            updated_any_doc = true;
        }

        self.sync.manifest.last_sync_at_ms = Some(outcome.synced_at_ms);
        self.sync.network_probed_offline = false;
        self.sync.sync_in_flight = false;
        self.sync.connection = SyncConnectionState::Synced;
        self.sync.last_error = None;
        if self.compute_pending_changes_fast() == 0 {
            self.sync.last_dirty_at_ms = None;
        }

        if updated_any_doc {
            self.bump_revision();
        }
        Ok(())
    }

    pub(crate) fn sync_mark_slot_dirty(&mut self, index: usize) {
        if !self.sync.track_manifest {
            return;
        }
        self.ensure_sync_vec_lens(self.notebooks.len());
        if index < self.sync.slot_dirty_hash.len() {
            self.sync.slot_dirty_hash[index] = true;
            self.sync.slot_placeholder_blank[index] = false;
            self.sync.last_dirty_at_ms.get_or_insert_with(now_unix_ms);
        }
    }

    pub(crate) fn sync_mark_prefs_dirty(&mut self) {
        if !self.sync.track_manifest {
            return;
        }
        self.sync.prefs_dirty_pending = true;
        let hash = canonical_prefs_hash(&self.prefs);
        self.sync.manifest.mark_prefs_dirty(&hash);
        self.sync_reconcile_folders();
        self.sync.last_dirty_at_ms.get_or_insert_with(now_unix_ms);
        self.sync_persist_manifest();
    }

    pub(crate) fn sync_before_remove_slot(&mut self, index: usize) {
        if !self.sync.track_manifest {
            return;
        }
        self.ensure_sync_vec_lens(self.notebooks.len());
        if index < self.sync.slot_ids.len() {
            let id = self.sync.slot_ids.remove(index);
            if !id.is_empty() {
                self.sync.manifest.record_local_delete(&id, now_unix_ms());
                self.sync.last_dirty_at_ms.get_or_insert_with(now_unix_ms);
            }
        }
        if index < self.sync.slot_dirty_hash.len() {
            self.sync.slot_dirty_hash.remove(index);
        }
        if index < self.sync.slot_placeholder_blank.len() {
            self.sync.slot_placeholder_blank.remove(index);
        }
        self.sync_persist_manifest();
    }

    pub(crate) fn sync_restore_notebook_slot(&mut self, index: usize, notebook_id: &str) {
        if !self.sync.track_manifest {
            return;
        }
        let clean_id = sanitize_notebook_id(notebook_id);
        let id = if clean_id.is_empty() {
            Uuid::new_v4().to_string()
        } else {
            clean_id
        };
        let restored_base_version = self
            .sync
            .manifest
            .tombstones
            .iter()
            .find(|t| t.notebook_id == id)
            .map_or(0, |t| t.base_version);
        self.sync.manifest.clear_tombstone(&id);
        self.ensure_sync_vec_lens(self.notebooks.len());
        if index < self.sync.slot_ids.len() {
            self.sync.slot_ids[index] = id.clone();
            self.sync.slot_dirty_hash[index] = true;
            self.sync.slot_placeholder_blank[index] = false;
        }
        if let Some(doc) = self.notebooks.get(index) {
            let hash = canonical_document_hash(doc);
            self.sync.manifest.mark_notebook_dirty(&id, &hash);
            if restored_base_version > 0
                && let Some(meta) = self
                    .sync
                    .manifest
                    .notebooks
                    .iter_mut()
                    .find(|m| m.notebook_id == id)
            {
                meta.server_version = restored_base_version;
                meta.last_synced_hash.clear();
            }
        }
        self.sync.last_dirty_at_ms.get_or_insert_with(now_unix_ms);
        self.sync_persist_manifest();
    }

    pub(crate) fn sync_reconcile_slots(&mut self) {
        if !self.sync.track_manifest {
            return;
        }
        let count = self.notebooks.len();
        let mut changed = self.sync.slot_ids.len() != count;
        self.ensure_sync_vec_lens(count);

        for idx in 0..count {
            if self.sync.slot_placeholder_blank[idx] {
                continue;
            }
            let file_name = self.library.slot_files.get(idx).and_then(slot_file_name);
            let embedded_id = sanitize_notebook_id(&self.notebooks[idx].id);
            if self.sync.slot_ids[idx].is_empty() || !embedded_id.is_empty() {
                let id = if !embedded_id.is_empty() {
                    embedded_id
                } else {
                    let resolved = file_name
                        .as_deref()
                        .and_then(|name| self.sync.manifest.id_for_local_file(name))
                        .map(str::to_string)
                        .unwrap_or_else(|| Uuid::new_v4().to_string());
                    let mut updated_doc = (*self.notebooks[idx]).clone();
                    updated_doc.id = resolved.clone();
                    let arc = Arc::new(updated_doc);
                    self.notebooks[idx] = Arc::clone(&arc);
                    if idx == self.active_notebook {
                        self.doc = arc;
                    }
                    self.library.mark_dirty(idx, count);
                    resolved
                };
                if self.sync.slot_ids[idx] != id {
                    self.sync.slot_ids[idx] = id.clone();
                    changed = true;
                }
                if self.sync.manifest.meta(&id).is_none() {
                    let hash = canonical_document_hash(&self.notebooks[idx]);
                    self.sync.manifest.mark_notebook_dirty(&id, &hash);
                    self.sync.last_dirty_at_ms.get_or_insert_with(now_unix_ms);
                    changed = true;
                }
                if let Some(name) = file_name.as_deref() {
                    let bound = self
                        .sync
                        .manifest
                        .meta(&id)
                        .and_then(|m| m.local_file.as_deref());
                    if bound != Some(name) {
                        self.sync.manifest.bind_local_file(&id, name);
                        changed = true;
                    }
                }
            } else if let Some(name) = file_name.as_deref() {
                let id = &self.sync.slot_ids[idx];
                let bound = self
                    .sync
                    .manifest
                    .meta(id)
                    .and_then(|m| m.local_file.as_deref());
                if bound != Some(name) {
                    self.sync.manifest.bind_local_file(id, name);
                    changed = true;
                }
            }
        }

        if changed {
            self.sync_persist_manifest();
        }
    }

    fn sync_init_from_disk(&mut self, is_placeholder: bool, owner: OwnerId) {
        let Some(ref lib_dir) = self.library.dir else {
            return;
        };
        self.sync.manifest = load_manifest(lib_dir);
        self.sync.manifest.owner_id = Some(owner);
        let count = self.notebooks.len();
        self.sync.slot_ids = vec![String::new(); count];
        self.sync.slot_dirty_hash = vec![false; count];
        self.sync.slot_placeholder_blank = vec![is_placeholder && count == 1; count];
        self.sync.prefs_dirty_pending = false;
        self.sync.last_dirty_at_ms = None;

        if !is_placeholder {
            self.sync_reconcile_slots();
            self.sync_reconcile_folders();
            for idx in 0..count {
                let id = self.sync.slot_ids[idx].clone();
                let cur_hash = canonical_document_hash(&self.notebooks[idx]);
                let is_diff = self
                    .sync
                    .manifest
                    .meta(&id)
                    .is_none_or(|m| m.server_version == 0 || m.last_synced_hash != cur_hash);
                if is_diff {
                    self.sync.manifest.mark_notebook_dirty(&id, &cur_hash);
                }
            }
            let prefs_hash = canonical_prefs_hash(&self.prefs);
            if self.sync.manifest.prefs_dirty
                || (self.sync.manifest.prefs_version > 0
                    && self.sync.manifest.prefs_hash != prefs_hash)
            {
                self.sync.manifest.mark_prefs_dirty(&prefs_hash);
            }
        }
        if self.sync.manifest.pending_changes_count() > 0 {
            self.sync.last_dirty_at_ms = Some(now_unix_ms());
        }
        self.sync_persist_manifest();
    }

    fn sync_reconcile_folders(&mut self) {
        if !self.sync.track_manifest {
            return;
        }
        let now = now_unix_ms();
        let owner = self.sync.manifest.owner_id.clone();
        let is_cloud = owner.as_ref().is_some_and(OwnerId::is_cloud);
        let current_paths = sanitize_library_folders(&self.prefs.library_folders);
        let mut changed = false;

        for path in &current_paths {
            let color_hex = self.prefs.folder_color(path).map(Color::to_hex);
            if let Some(existing) = self
                .sync
                .manifest
                .folders
                .iter_mut()
                .find(|f| f.path.eq_ignore_ascii_case(path))
            {
                if existing.path != *path
                    || existing.color_hex != color_hex
                    || existing.deleted_at_ms.is_some()
                {
                    existing.path = path.clone();
                    existing.color_hex = color_hex;
                    existing.deleted_at_ms = None;
                    existing.updated_at_ms = now.max(existing.updated_at_ms);
                    existing.cloud_dirty = is_cloud;
                    existing.status = if is_cloud {
                        EntitySyncStatus::PendingPush
                    } else {
                        EntitySyncStatus::LocalOnly
                    };
                    changed = true;
                }
            } else {
                self.sync.manifest.folders.push(FolderSyncMeta {
                    folder_id: Uuid::new_v4().to_string(),
                    owner_id: owner.clone(),
                    path: path.clone(),
                    color_hex,
                    status: if is_cloud {
                        EntitySyncStatus::PendingPush
                    } else {
                        EntitySyncStatus::LocalOnly
                    },
                    server_version: 0,
                    cloud_dirty: is_cloud,
                    updated_at_ms: now,
                    deleted_at_ms: None,
                });
                changed = true;
            }
        }

        let mut keep = Vec::with_capacity(self.sync.manifest.folders.len());
        for mut folder in std::mem::take(&mut self.sync.manifest.folders) {
            let still_present = current_paths
                .iter()
                .any(|p| p.eq_ignore_ascii_case(&folder.path));
            if still_present {
                keep.push(folder);
            } else if folder.server_version > 0 && is_cloud {
                if folder.deleted_at_ms.is_none() {
                    folder.deleted_at_ms = Some(now);
                    folder.updated_at_ms = now.max(folder.updated_at_ms);
                    folder.cloud_dirty = true;
                    folder.status = EntitySyncStatus::PendingPush;
                    changed = true;
                }
                keep.push(folder);
            } else {
                changed = true;
            }
        }
        self.sync.manifest.folders = keep;
        if changed {
            self.sync_persist_manifest();
        }
    }

    fn ack_accepted_folders(&mut self, accepted: &[papercraft_sync_protocol::AcceptedFolderSync]) {
        for acc in accepted {
            if acc.deleted {
                self.sync
                    .manifest
                    .folders
                    .retain(|f| f.folder_id != acc.folder_id);
            } else if let Some(entry) = self
                .sync
                .manifest
                .folders
                .iter_mut()
                .find(|f| f.folder_id == acc.folder_id)
            {
                entry.server_version = acc.new_version;
                entry.cloud_dirty = false;
                entry.status = EntitySyncStatus::Synced;
            }
        }
    }

    fn apply_remote_folders(&mut self, remote_folders: &[RemoteFolderRecord]) -> bool {
        if remote_folders.is_empty() {
            return false;
        }
        let mut changed = false;
        for rf in remote_folders {
            let existing_pos =
                self.sync.manifest.folders.iter().position(|f| {
                    f.folder_id == rf.folder_id || f.path.eq_ignore_ascii_case(&rf.path)
                });
            if rf.deleted {
                if let Some(pos) = existing_pos {
                    let removed = self.sync.manifest.folders.remove(pos);
                    self.prefs
                        .library_folders
                        .retain(|p| !p.eq_ignore_ascii_case(&removed.path));
                    self.prefs.set_folder_color(&removed.path, None);
                    changed = true;
                }
                continue;
            }
            let clean_path = sanitize_folder_path(&rf.path);
            if clean_path.is_empty() {
                continue;
            }
            let parsed_color = rf.color_hex.as_deref().and_then(Color::from_hex);
            let is_local_newer = existing_pos.is_some_and(|pos| {
                let cur = &self.sync.manifest.folders[pos];
                cur.cloud_dirty && cur.updated_at_ms > rf.updated_at_ms
            });
            if is_local_newer {
                continue;
            }
            if let Some(pos) = existing_pos {
                let old_path = self.sync.manifest.folders[pos].path.clone();
                if !old_path.eq_ignore_ascii_case(&clean_path) {
                    self.prefs
                        .library_folders
                        .retain(|p| !p.eq_ignore_ascii_case(&old_path));
                }
                let entry = &mut self.sync.manifest.folders[pos];
                entry.folder_id = rf.folder_id.clone();
                entry.path = clean_path.clone();
                entry.color_hex = rf.color_hex.clone();
                entry.server_version = rf.version;
                entry.cloud_dirty = false;
                entry.deleted_at_ms = None;
                entry.updated_at_ms = rf.updated_at_ms;
                entry.status = EntitySyncStatus::Synced;
            } else {
                self.sync.manifest.folders.push(FolderSyncMeta {
                    folder_id: rf.folder_id.clone(),
                    owner_id: self.sync.manifest.owner_id.clone(),
                    path: clean_path.clone(),
                    color_hex: rf.color_hex.clone(),
                    status: EntitySyncStatus::Synced,
                    server_version: rf.version,
                    cloud_dirty: false,
                    updated_at_ms: rf.updated_at_ms,
                    deleted_at_ms: None,
                });
            }
            self.prefs.library_folders.push(clean_path.clone());
            self.prefs.set_folder_color(&clean_path, parsed_color);
            changed = true;
        }
        if changed {
            self.prefs.library_folders = sanitize_library_folders(&self.prefs.library_folders);
            self.prefs = self.prefs.sanitized();
            let _ = self.save_prefs();
        }
        changed
    }

    fn apply_remote_prefs_preserving_folders(&mut self, remote_prefs: &UserStudioConfig) {
        let local_folders = self.prefs.library_folders.clone();
        let local_folder_colors = self.prefs.library_folder_colors.clone();
        let mut merged = remote_prefs.clone().sanitized();
        if !self.sync.manifest.folders.is_empty() {
            merged.library_folders = local_folders;
            merged.library_folder_colors = local_folder_colors;
        }
        self.prefs = merged.sanitized();
        self.apply_pen_size_preferences();
    }

    fn compute_pending_changes_fast(&self) -> usize {
        let mut count = self.sync.manifest.pending_changes_count();
        for (idx, dirty) in self.sync.slot_dirty_hash.iter().enumerate() {
            if !*dirty
                || self
                    .sync
                    .slot_placeholder_blank
                    .get(idx)
                    .copied()
                    .unwrap_or(false)
            {
                continue;
            }
            let already_counted = self
                .sync
                .slot_ids
                .get(idx)
                .and_then(|id| self.sync.manifest.meta(id))
                .is_some_and(|m| m.cloud_dirty || m.server_version == 0);
            if !already_counted {
                count += 1;
            }
        }
        if self.sync.prefs_dirty_pending && !self.sync.manifest.prefs_dirty {
            count += 1;
        }
        count
    }

    fn ensure_sync_vec_lens(&mut self, count: usize) {
        self.sync.slot_ids.resize_with(count, String::new);
        self.sync.slot_dirty_hash.resize(count, false);
        self.sync.slot_placeholder_blank.resize(count, false);
    }

    fn sync_persist_global_state(&self) {
        if let Some(ref root) = self.sync.desktop_root
            && let Err(err) = save_global_state(root, &self.sync.global)
        {
            log::warn!("Failed saving global sync state: {err}");
        }
    }

    pub(crate) fn sync_persist_manifest(&self) {
        if !self.sync.track_manifest || self.is_viewing_cloud() {
            return;
        }
        if let Some(ref dir) = self.library.dir
            && let Err(err) = save_manifest(dir, &self.sync.manifest)
        {
            log::warn!("Failed saving sync manifest: {err}");
        }
    }

    fn ack_accepted_uploads(
        &mut self,
        accepted: &[papercraft_sync_protocol::AcceptedNotebookSync],
        synced_at_ms: i64,
    ) {
        for acc in accepted {
            let cur_hash = self
                .find_slot_by_id(&acc.notebook_id)
                .map(|idx| canonical_document_hash(&self.notebooks[idx]))
                .unwrap_or_else(|| acc.content_hash.clone());
            self.sync.manifest.mark_notebook_synced(
                &acc.notebook_id,
                acc.new_version,
                acc.content_hash.clone(),
                &cur_hash,
                synced_at_ms,
            );
            if let Some(idx) = self.find_slot_by_id(&acc.notebook_id)
                && cur_hash == acc.content_hash
            {
                self.sync.slot_dirty_hash[idx] = false;
            }
        }
    }

    fn apply_remote_upsert(&mut self, update: &RemoteNotebookUpdate, synced_at_ms: i64) -> bool {
        let Some(mut doc) = update.document.clone() else {
            return false;
        };
        doc.id = update.notebook_id.clone();
        doc.sync_active_to_pages();
        doc.load_active_from_pages();

        if let Some(idx) = self.find_slot_by_id(&update.notebook_id) {
            let cur_hash = canonical_document_hash(&self.notebooks[idx]);
            let local_changed = self.sync.slot_dirty_hash[idx]
                || self
                    .sync
                    .manifest
                    .meta(&update.notebook_id)
                    .is_some_and(|m| m.cloud_dirty && cur_hash != m.last_synced_hash);

            if local_changed && cur_hash != update.content_hash {
                let local_body = canonical_document_body_hash(&self.notebooks[idx]);
                let remote_body = canonical_document_body_hash(&doc);
                if local_body == remote_body {
                    let local_doc = &self.notebooks[idx];
                    if local_doc.meta_updated_at_ms > doc.meta_updated_at_ms {
                        doc.title = local_doc.title.clone();
                        doc.cover = local_doc.cover;
                        doc.folder_path = local_doc.folder_path.clone();
                        doc.favorite = local_doc.favorite;
                        doc.tags = local_doc.tags.clone();
                        doc.meta_updated_at_ms = local_doc.meta_updated_at_ms;
                    }
                    doc.sync_active_to_pages();
                    let merged_hash = canonical_document_hash(&doc);
                    self.notebooks[idx] = Arc::new(doc);
                    self.sync.slot_dirty_hash[idx] = merged_hash != update.content_hash;
                    self.sync.slot_placeholder_blank[idx] = false;
                    if idx == self.active_notebook {
                        self.doc = Arc::clone(&self.notebooks[idx]);
                        self.history.clear();
                    }
                    self.sync.manifest.mark_notebook_synced(
                        &update.notebook_id,
                        update.version,
                        update.content_hash.clone(),
                        &merged_hash,
                        synced_at_ms,
                    );
                    return true;
                }
                // Preserve local unpushed edits in RAM so the next real-time push overwrites directly
                // without creating "(Conflict Copy)" duplicates.
                self.sync.manifest.mark_notebook_synced(
                    &update.notebook_id,
                    update.version,
                    update.content_hash.clone(),
                    &cur_hash,
                    synced_at_ms,
                );
                return false;
            }

            self.notebooks[idx] = Arc::new(doc);
            self.sync.slot_dirty_hash[idx] = false;
            self.sync.slot_placeholder_blank[idx] = false;
            if idx == self.active_notebook {
                self.doc = Arc::clone(&self.notebooks[idx]);
                self.history.clear();
            }
            self.sync.manifest.mark_notebook_synced(
                &update.notebook_id,
                update.version,
                update.content_hash.clone(),
                &update.content_hash,
                synced_at_ms,
            );
            return true;
        }

        if let Some(p_idx) = self.sync.slot_placeholder_blank.iter().position(|&b| b) {
            self.notebooks[p_idx] = Arc::new(doc);
            self.sync.slot_ids[p_idx] = update.notebook_id.clone();
            self.sync.slot_dirty_hash[p_idx] = false;
            self.sync.slot_placeholder_blank[p_idx] = false;
            if p_idx == self.active_notebook {
                self.doc = Arc::clone(&self.notebooks[p_idx]);
                self.history.clear();
            }
            self.sync.manifest.mark_notebook_synced(
                &update.notebook_id,
                update.version,
                update.content_hash.clone(),
                &update.content_hash,
                synced_at_ms,
            );
            return true;
        }

        if self.notebooks.len() >= MAX_NOTEBOOKS {
            log::warn!(
                "Skipping remote notebook `{}`: reached MAX_NOTEBOOKS ({MAX_NOTEBOOKS})",
                update.notebook_id
            );
            return false;
        }

        self.notebooks.push(Arc::new(doc));
        self.library.push_slot(None, false, self.notebooks.len());
        self.sync.slot_ids.push(update.notebook_id.clone());
        self.sync.slot_dirty_hash.push(false);
        self.sync.slot_placeholder_blank.push(false);
        if self.notebooks.len() == 1 {
            self.active_notebook = 0;
            self.doc = Arc::clone(&self.notebooks[0]);
            self.history.clear();
        }
        self.sync.manifest.mark_notebook_synced(
            &update.notebook_id,
            update.version,
            update.content_hash.clone(),
            &update.content_hash,
            synced_at_ms,
        );
        true
    }

    fn apply_remote_delete(&mut self, notebook_id: &str) -> bool {
        self.sync.manifest.clear_tombstone(notebook_id);
        self.sync
            .manifest
            .notebooks
            .retain(|m| m.notebook_id != notebook_id);

        let Some(idx) = self.find_slot_by_id(notebook_id) else {
            return false;
        };

        self.notebooks.remove(idx);
        self.library.remove_slot(idx);
        self.sync.slot_ids.remove(idx);
        self.sync.slot_dirty_hash.remove(idx);
        self.sync.slot_placeholder_blank.remove(idx);

        if self.notebooks.is_empty() {
            self.active_notebook = 0;
            let mut blank = Document::default();
            blank.sync_active_to_pages();
            self.doc = Arc::new(blank);
        } else {
            if idx < self.active_notebook {
                self.active_notebook = self.active_notebook.saturating_sub(1);
            } else if self.active_notebook >= self.notebooks.len() {
                self.active_notebook = self.notebooks.len().saturating_sub(1);
            }
            self.doc = Arc::clone(&self.notebooks[self.active_notebook]);
        }
        self.history.clear();
        true
    }

    pub(crate) fn find_slot_by_id(&self, notebook_id: &str) -> Option<usize> {
        self.sync.slot_ids.iter().position(|id| id == notebook_id)
    }
}

struct CapturedProfileData {
    docs: Vec<Document>,
    prefs: UserStudioConfig,
}

/// Merges folders, colors, custom templates, elements, and style presets cleanly into `target`.
pub(crate) fn merge_studio_prefs(
    target: &mut UserStudioConfig,
    incoming: UserStudioConfig,
    overwrite_appearance: bool,
) {
    let mut combined_folders = target.library_folders.clone();
    combined_folders.extend(incoming.library_folders);
    target.library_folders = sanitize_library_folders(&combined_folders);

    for (k, v) in incoming.library_folder_colors {
        if overwrite_appearance {
            target.library_folder_colors.insert(k, v);
        } else {
            target.library_folder_colors.entry(k).or_insert(v);
        }
    }
    for (k, v) in incoming.library_tag_colors {
        if overwrite_appearance {
            target.library_tag_colors.insert(k, v);
        } else {
            target.library_tag_colors.entry(k).or_insert(v);
        }
    }
    for tmpl in incoming.custom_templates {
        if !target.custom_templates.iter().any(|t| t.id == tmpl.id) {
            target.custom_templates.push(tmpl);
        }
    }
    for elem in incoming.saved_elements {
        if !target.saved_elements.iter().any(|e| e.id == elem.id) {
            target.saved_elements.push(elem);
        }
    }
    for fav in incoming.style_favorites {
        if !target.style_favorites.iter().any(|f| f.id == fav.id) {
            target.style_favorites.push(fav);
        }
    }
    for preset in incoming.palette_presets {
        if !target.palette_presets.iter().any(|p| p.id == preset.id) {
            target.palette_presets.push(preset);
        }
    }
    if overwrite_appearance {
        target.theme = incoming.theme;
        target.auto_contrast_with_theme = incoming.auto_contrast_with_theme;
        target.ui_scale = incoming.ui_scale;
        target.pen_sizes = incoming.pen_sizes;
        target.pen_smoothings = incoming.pen_smoothings;
        target.ink_palette = incoming.ink_palette;
        target.highlighter_palette = incoming.highlighter_palette;
        target.wildcard_ink = incoming.wildcard_ink;
        target.wildcard_highlighter = incoming.wildcard_highlighter;
        target.active_palette_id = incoming.active_palette_id;
        target.stylus_buttons = incoming.stylus_buttons;
        target.stylus_only_drawing = incoming.stylus_only_drawing;
        target.library_view_mode = incoming.library_view_mode;
        target.library_sort_by = incoming.library_sort_by;
        target.library_sort_ascending = incoming.library_sort_ascending;
    }
    *target = target.sanitized();
}

fn count_non_placeholder_slots(total: usize, placeholders: &[bool]) -> usize {
    (0..total)
        .filter(|&idx| !placeholders.get(idx).copied().unwrap_or(false))
        .count()
}

fn ensure_default_profile_in_global(global: &mut GlobalSyncState) -> String {
    if let Some(active) = global.active_local_profile() {
        let id = active.local_profile_id.clone();
        if global.active_local_profile_id != id {
            global.active_local_profile_id = id.clone();
        }
        return id;
    }
    let now = now_unix_ms();
    let id = Uuid::new_v4().to_string();
    global.local_profiles.push(LocalProfile {
        local_profile_id: id.clone(),
        nickname: "Personal".to_string(),
        created_at_ms: now,
        last_used_at_ms: now,
    });
    global.active_local_profile_id = id.clone();
    id
}

fn cleanup_legacy_cloud_cache_dir(root_dir: &Path) {
    let legacy_accounts = root_dir.join(ACCOUNTS_DIR);
    if legacy_accounts.is_dir() {
        let _ = fs::remove_dir_all(legacy_accounts);
    }
}

fn recover_existing_desktop_profiles(root_dir: &Path, global: &mut GlobalSyncState) -> bool {
    let profiles_root = root_dir.join(papercraft_sync::PROFILES_DIR);
    if !profiles_root.is_dir() {
        return false;
    }
    let Ok(entries) = fs::read_dir(&profiles_root) else {
        return false;
    };
    let mut found: Vec<(usize, String)> = Vec::new();
    for entry in entries.flatten().take(64) {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(raw_id) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if raw_id.is_empty() || sanitize_account_dir_segment(raw_id) != raw_id {
            continue;
        }
        let ncraft_count = count_ncraft_files_in_dir(&path);
        let has_data = ncraft_count > 0
            || path.join(LIBRARY_MARKER_FILE).exists()
            || path.join(STUDIO_PREFS_FILE).exists();
        if has_data {
            found.push((ncraft_count, raw_id.to_string()));
        }
    }
    if found.is_empty() {
        return false;
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let now = now_unix_ms();
    global.local_profiles = found
        .into_iter()
        .enumerate()
        .map(|(idx, (_, id))| LocalProfile {
            local_profile_id: id,
            nickname: if idx == 0 {
                "Personal".to_string()
            } else {
                format!("Recovered {}", idx + 1)
            },
            created_at_ms: now,
            last_used_at_ms: now,
        })
        .collect();
    if let Some(first) = global.local_profiles.first() {
        global.active_local_profile_id = first.local_profile_id.clone();
    }
    global.needs_initial_nickname = false;
    true
}

fn ensure_desktop_default_profile(root_dir: &Path, global: &mut GlobalSyncState) -> bool {
    if !global.local_profiles.is_empty() {
        let before = global.active_local_profile_id.clone();
        let resolved = ensure_default_profile_in_global(global);
        return resolved != before;
    }
    if recover_existing_desktop_profiles(root_dir, global) {
        return true;
    }
    let pid = ensure_default_profile_in_global(global);
    let target_dir = profile_library_dir(root_dir, &pid);
    if !target_dir.exists() {
        migrate_legacy_root_library(root_dir, &target_dir);
    }
    true
}

fn has_legacy_root_library(root_dir: &Path) -> bool {
    root_dir.join(LIBRARY_MARKER_FILE).exists()
        || root_dir.join(STUDIO_PREFS_FILE).exists()
        || root_dir.join(TRASH_SUBDIR).is_dir()
        || fs::read_dir(root_dir).ok().is_some_and(|entries| {
            entries.flatten().any(|e| {
                e.path()
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| {
                        ext.eq_ignore_ascii_case("pcraft") || ext.eq_ignore_ascii_case("ncraft")
                    })
            })
        })
}

fn migrate_legacy_root_library(root_dir: &Path, target_dir: &Path) {
    if !has_legacy_root_library(root_dir) {
        return;
    }
    let _ = fs::create_dir_all(target_dir);
    for fixed in [
        LIBRARY_MARKER_FILE,
        STUDIO_PREFS_FILE,
        papercraft_sync::SYNC_MANIFEST_FILE,
        TRASH_SUBDIR,
    ] {
        let src = root_dir.join(fixed);
        if src.exists() {
            let _ = fs::rename(&src, target_dir.join(fixed));
        }
    }
    if let Ok(entries) = fs::read_dir(root_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| {
                        ext.eq_ignore_ascii_case("pcraft") || ext.eq_ignore_ascii_case("ncraft")
                    })
                && let Some(name) = path.file_name()
            {
                let _ = fs::rename(&path, target_dir.join(name));
            }
        }
    }
}

fn migrate_web_cache_if_needed(cache: &mut WebWorkspaceCache) {
    if cache.local_profiles.is_empty()
        && !cache.global.needs_initial_nickname
        && let Some(guest_bundle) = cache.guest.take()
    {
        let pid = ensure_default_profile_in_global(&mut cache.global);
        cache.local_profiles.insert(pid, guest_bundle);
    }
    cache.version = WEB_CACHE_VERSION;
}

fn sanitize_welcome_version(raw: &str) -> Option<String> {
    let cleaned: String = raw
        .trim()
        .chars()
        .filter(|c| !c.is_control() && !c.is_whitespace())
        .take(32)
        .collect();
    (!cleaned.is_empty()).then_some(cleaned)
}

fn sanitize_global_state(global: &mut GlobalSyncState) {
    global.server_url =
        normalize_server_url(&global.server_url).unwrap_or_else(|_| DEFAULT_SERVER_URL.to_string());
    global.device_name = sanitize_device_name(&global.device_name);
    global.client_id = sanitize_client_id(&global.client_id);
    if global.client_id.is_empty() {
        global.client_id = Uuid::new_v4().to_string();
    }
    global.welcome_banner_seen_version = global
        .welcome_banner_seen_version
        .as_deref()
        .and_then(sanitize_welcome_version);
    let _ = global.migrate_legacy_session();
    global.remembered_server_accounts =
        sanitize_remembered_accounts(std::mem::take(&mut global.remembered_server_accounts));
    for prof in &mut global.local_profiles {
        prof.local_profile_id = sanitize_account_dir_segment(&prof.local_profile_id);
        prof.nickname = sanitize_profile_nickname(&prof.nickname);
    }
}

fn starter_web_bundle() -> CachedLibraryBundle {
    let notebooks = starter_documents()
        .into_iter()
        .map(|mut d| {
            d.sync_active_to_pages();
            d
        })
        .collect();
    let mut prefs = UserStudioConfig::default();
    seed_starter_library_prefs(&mut prefs);
    CachedLibraryBundle {
        notebooks,
        prefs,
        ..Default::default()
    }
}

fn restore_web_bundle(bundle: &CachedLibraryBundle) -> Session {
    let docs: Vec<Arc<Document>> = bundle
        .notebooks
        .iter()
        .take(MAX_NOTEBOOKS)
        .cloned()
        .map(|mut d| {
            d.id = sanitize_notebook_id(&d.id);
            d.title = sanitize_notebook_title(&d.title);
            d.sync_active_to_pages();
            d.load_active_from_pages();
            Arc::new(d)
        })
        .collect();

    let mut session = Session {
        prefs: bundle.prefs.clone().sanitized(),
        ..Default::default()
    };
    session.apply_pen_size_preferences();
    if docs.is_empty() {
        session.notebooks.clear();
        let mut blank = Document::default();
        blank.sync_active_to_pages();
        session.doc = Arc::new(blank);
        session.active_notebook = 0;
    } else {
        let active = bundle.active_notebook.min(docs.len().saturating_sub(1));
        session.notebooks = docs;
        session.active_notebook = active;
        session.doc = Arc::clone(&session.notebooks[active]);
    }

    let count = session.notebooks.len();
    session.library.sync_len(count);
    session.trashed_notebooks = bundle
        .trash
        .iter()
        .map(|entry| {
            let mut d = entry.doc.clone();
            d.id = sanitize_notebook_id(&d.id);
            d.title = sanitize_notebook_title(&d.title);
            d.sync_active_to_pages();
            d.load_active_from_pages();
            TrashedNotebook {
                doc: Arc::new(d),
                deleted_at_ms: entry.deleted_at_ms,
                path: None,
            }
        })
        .collect();
    session.cleanup_expired_trash();

    session.sync.manifest = bundle.manifest.clone();
    session.sync.slot_ids = bundle.slot_ids.clone();
    session.sync.slot_dirty_hash = bundle.slot_dirty_hash.clone();
    session.sync.slot_placeholder_blank = bundle.slot_placeholder_blank.clone();
    session.sync.prefs_dirty_pending = bundle.prefs_dirty_pending;
    session.sync.last_dirty_at_ms = bundle.last_dirty_at_ms;
    session.ensure_sync_vec_lens(count);
    session
}

fn slot_file_name(opt_path: &Option<PathBuf>) -> Option<String> {
    opt_path
        .as_ref()
        .and_then(|p| p.file_name())
        .map(|s| s.to_string_lossy().to_string())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::Session;
    use papercraft_doc::{Document, NotebookCover, PenSettings, StrokeSample, UiTheme};
    use papercraft_sync::{
        ActiveAccountSession, EntitySyncStatus, RemoteNotebookUpdate, SyncCycleOutcome,
    };
    use papercraft_sync_protocol::{
        RemoteNotebookRecord, SyncPullResponse, UserProfile, canonical_document_body_hash,
        canonical_document_hash, canonical_document_meta_hash,
    };
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::Arc;

    struct TempDirGuard(PathBuf);
    impl Drop for TempDirGuard {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir(label: &str) -> TempDirGuard {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let p = std::env::temp_dir().join(format!("papercraft_engine_sync_{label}_{nanos}"));
        let _ = std::fs::remove_dir_all(&p);
        TempDirGuard(p)
    }

    fn fake_account(id: &str, email: &str, username: &str) -> ActiveAccountSession {
        ActiveAccountSession {
            server_url: "http://127.0.0.1:8787".to_string(),
            user: UserProfile {
                user_id: id.to_string(),
                email: Some(email.to_string()),
                username: username.to_string(),
                display_name: username.to_string(),
                created_at_ms: 1000,
                must_change_password: false,
            },
            access_token: "acc-secret-token".to_string(),
            refresh_token: "ref-secret-token".to_string(),
            access_expires_at_ms: 9_999_999,
            refresh_expires_at_ms: 9_999_999,
            device_name: "TestPC".to_string(),
            last_auth_ms: 1000,
        }
    }

    fn empty_pull_for(acct: &ActiveAccountSession, cursor: u64) -> SyncPullResponse {
        SyncPullResponse {
            server_cursor: cursor,
            user: acct.user.clone(),
            notebooks: Vec::new(),
            folders: Vec::new(),
            studio_prefs: None,
        }
    }

    fn apply_fake_upload_acks(
        session: &mut Session,
        batch: &papercraft_sync::SyncBatch,
        synced_at_ms: i64,
    ) {
        let accepted: Vec<_> = batch
            .local_docs
            .iter()
            .enumerate()
            .map(|(i, snap)| papercraft_sync_protocol::AcceptedNotebookSync {
                notebook_id: snap.notebook_id.clone(),
                new_version: snap.base_version + 1,
                cursor: (i + 1) as u64,
                deleted: false,
                content_hash: snap.content_hash.clone(),
                body_hash: canonical_document_body_hash(&snap.doc),
                meta_hash: canonical_document_meta_hash(&snap.doc),
            })
            .collect();
        let count = accepted.len();
        session
            .apply_sync_outcome(SyncCycleOutcome {
                new_cursor: count as u64,
                refreshed_session: None,
                server_user: fake_account("user-a", "alice@example.com", "alice").user,
                accepted_uploads: accepted,
                accepted_folders: Vec::new(),
                cleared_tombstones: Vec::new(),
                downloaded_updates: Vec::new(),
                downloaded_folders: Vec::new(),
                updated_prefs: None,
                ack_prefs: None,
                pushed_count: count,
                pulled_count: 0,
                synced_at_ms,
            })
            .unwrap();
    }

    fn has_notebook_title(session: &Session, title: &str) -> bool {
        session.notebooks.iter().any(|n| n.title == title)
    }

    #[test]
    fn online_server_session_stores_zero_local_files_or_tokens_and_remembers_shortcut() {
        let dir = temp_dir("zero_cache_desktop");
        let mut session = Session::open_desktop_library(dir.0.clone());
        session
            .execute(
                "profile.completeFirstAccess",
                &json!({ "nickname": "Personal" }),
            )
            .unwrap();
        let personal_id = session
            .active_local_profile()
            .unwrap()
            .local_profile_id
            .clone();
        session
            .execute("notebook.create", &json!({ "title": "Offline Sketchbook" }))
            .unwrap();

        let acct = fake_account("user-a", "alice@example.com", "alice");
        let remote_doc = Document {
            id: "nb-remote-1".to_string(),
            title: "Cloud Lecture Notes".to_string(),
            ..Default::default()
        };
        let remote_hash = canonical_document_hash(&remote_doc);
        let pull = SyncPullResponse {
            server_cursor: 4,
            user: acct.user.clone(),
            notebooks: vec![RemoteNotebookRecord {
                notebook_id: "nb-remote-1".to_string(),
                version: 2,
                cursor: 4,
                deleted: false,
                updated_at_ms: 2000,
                deleted_at_ms: None,
                title: "Cloud Lecture Notes".to_string(),
                content_hash: remote_hash,
                body_hash: canonical_document_body_hash(&remote_doc),
                meta_hash: canonical_document_meta_hash(&remote_doc),
                meta_updated_at_ms: 2000,
                document: Some(remote_doc),
            }],
            folders: Vec::new(),
            studio_prefs: None,
        };

        session.enter_online_server_session(acct.clone(), pull);
        assert!(session.is_signed_in());
        assert!(session.is_viewing_cloud());
        assert!(session.library_dir().is_none());
        assert!(!session.has_dirty_notebooks());
        assert!(has_notebook_title(&session, "Cloud Lecture Notes"));
        assert!(!has_notebook_title(&session, "Offline Sketchbook"));
        assert_eq!(session.cloud_pending_changes_count(), 0);
        assert_eq!(session.notebook_sync_status(0), EntitySyncStatus::Synced);

        // Creating/drawing in the online session pushes directly with force_overwrite and writes ZERO files to disk!
        session
            .execute("notebook.create", &json!({ "title": "Live Cloud Note" }))
            .unwrap();
        let batch = session.prepare_sync_batch().unwrap();
        assert!(batch.force_overwrite);
        assert_eq!(batch.local_docs.len(), 1);
        apply_fake_upload_acks(&mut session, &batch, 3000);
        assert_eq!(session.cloud_pending_changes_count(), 0);

        // Verify zero cloud folders or auth tokens exist on disk!
        assert!(!dir.0.join(papercraft_sync::ACCOUNTS_DIR).exists());
        let global_raw =
            std::fs::read_to_string(dir.0.join(papercraft_sync::GLOBAL_SYNC_FILE)).unwrap();
        assert!(!global_raw.contains("acc-secret-token"));
        assert!(!global_raw.contains("ref-secret-token"));
        assert!(global_raw.contains("alice@example.com"));

        // Reopening desktop starts cleanly offline in local workspace; cloud shortcut stays remembered!
        drop(session);
        verify_zero_cache_reopen_and_forget(&dir.0, &personal_id);
    }

    fn verify_zero_cache_reopen_and_forget(dir: &std::path::Path, personal_id: &str) {
        let mut reopened = Session::open_desktop_library(dir.to_path_buf());
        assert!(!reopened.is_signed_in());
        assert!(!reopened.is_viewing_cloud());
        assert!(has_notebook_title(&reopened, "Offline Sketchbook"));
        assert!(!has_notebook_title(&reopened, "Cloud Lecture Notes"));
        assert!(!has_notebook_title(&reopened, "Live Cloud Note"));
        assert_eq!(reopened.remembered_server_accounts().len(), 1);
        assert_eq!(reopened.remembered_server_accounts()[0].user_id, "user-a");

        // Local profile ID remained intact; forget_server_account removes shortcut cleanly.
        assert_eq!(
            reopened.active_local_profile().unwrap().local_profile_id,
            personal_id
        );
        assert!(
            reopened
                .execute(
                    "profile.forgetServerAccount",
                    &json!({ "serverUrl": "http://127.0.0.1:8787", "userId": "user-a" })
                )
                .unwrap()["forgotten"]
                .as_bool()
                .unwrap()
        );
        assert!(reopened.remembered_server_accounts().is_empty());
    }

    #[test]
    fn switching_from_server_to_local_exits_cloud_and_preserves_local_data() {
        let dir = temp_dir("strict_switch");
        let mut session = Session::open_desktop_library(dir.0.clone());
        session
            .execute(
                "profile.completeFirstAccess",
                &json!({ "nickname": "Personal" }),
            )
            .unwrap();
        let personal_id = session
            .active_local_profile()
            .unwrap()
            .local_profile_id
            .clone();
        session
            .execute("notebook.create", &json!({ "title": "Personal Idea" }))
            .unwrap();

        let acct = fake_account("user-sw", "sw@example.com", "switcher");
        session.enter_online_server_session(acct.clone(), empty_pull_for(&acct, 1));
        session
            .execute("notebook.create", &json!({ "title": "Ephemeral Cloud" }))
            .unwrap();
        assert!(session.is_viewing_cloud());

        // Switching to a local account strictly drops the server session (zero dual-workspace state).
        session.switch_to_local_profile(&personal_id).unwrap();
        assert!(!session.is_signed_in());
        assert!(!session.is_viewing_cloud());
        assert!(has_notebook_title(&session, "Personal Idea"));
        assert!(!has_notebook_title(&session, "Ephemeral Cloud"));
        assert_eq!(session.remembered_server_accounts().len(), 1);

        // `profile.switch target=cloud` is rejected because password sign-in is always required.
        assert!(
            session
                .execute("profile.switch", &json!({ "target": "cloud" }))
                .is_err()
        );
    }

    #[test]
    fn metadata_only_divergence_merges_lww_and_content_divergence_never_creates_conflict_copy() {
        let mut session = Session::default();
        let acct = fake_account("user-b", "bob@example.com", "bob");
        session.enter_online_server_session(acct.clone(), empty_pull_for(&acct, 0));

        let mut base = Document {
            id: "nb-lww".to_string(),
            title: "Original Title".to_string(),
            cover: NotebookCover::Terracotta,
            meta_updated_at_ms: 1000,
            ..Default::default()
        };
        let sid = base.alloc_stroke_id();
        let pts = vec![
            StrokeSample::from_xy_pressure(10.0, 10.0, 0.5),
            StrokeSample::from_xy_pressure(50.0, 50.0, 0.7),
        ];
        base.strokes.push(Arc::new(papercraft_doc::InkStroke::new(
            sid,
            PenSettings::fountain_pen(),
            pts.clone(),
            pts,
        )));
        base.sync_active_to_pages();
        let base_hash = canonical_document_hash(&base);

        // Pull version 1.
        session
            .apply_sync_outcome(SyncCycleOutcome {
                new_cursor: 1,
                refreshed_session: None,
                server_user: acct.user.clone(),
                accepted_uploads: Vec::new(),
                accepted_folders: Vec::new(),
                cleared_tombstones: Vec::new(),
                downloaded_updates: vec![RemoteNotebookUpdate {
                    notebook_id: "nb-lww".to_string(),
                    version: 1,
                    cursor: 1,
                    deleted: false,
                    updated_at_ms: 1000,
                    deleted_at_ms: None,
                    content_hash: base_hash,
                    body_hash: canonical_document_body_hash(&base),
                    meta_hash: canonical_document_meta_hash(&base),
                    meta_updated_at_ms: 1000,
                    document: Some(base.clone()),
                }],
                downloaded_folders: Vec::new(),
                updated_prefs: None,
                ack_prefs: None,
                pushed_count: 0,
                pulled_count: 1,
                synced_at_ms: 1000,
            })
            .unwrap();

        // Draw locally while a remote update also arrives with different ink.
        session.commit_raw_stroke(
            PenSettings::ball_pen(),
            vec![
                StrokeSample::from_xy_pressure(80.0, 80.0, 0.5),
                StrokeSample::from_xy_pressure(110.0, 110.0, 0.8),
            ],
        );

        let mut remote_changed = base;
        let rsid = remote_changed.alloc_stroke_id();
        let rpts = vec![
            StrokeSample::from_xy_pressure(200.0, 200.0, 0.5),
            StrokeSample::from_xy_pressure(240.0, 240.0, 0.8),
        ];
        remote_changed
            .strokes
            .push(Arc::new(papercraft_doc::InkStroke::new(
                rsid,
                PenSettings::fountain_pen(),
                rpts.clone(),
                rpts,
            )));
        remote_changed.sync_active_to_pages();
        let remote_hash = canonical_document_hash(&remote_changed);

        session
            .apply_sync_outcome(SyncCycleOutcome {
                new_cursor: 2,
                refreshed_session: None,
                server_user: acct.user,
                accepted_uploads: Vec::new(),
                accepted_folders: Vec::new(),
                cleared_tombstones: Vec::new(),
                downloaded_updates: vec![RemoteNotebookUpdate {
                    notebook_id: "nb-lww".to_string(),
                    version: 2,
                    cursor: 2,
                    deleted: false,
                    updated_at_ms: 2000,
                    deleted_at_ms: None,
                    content_hash: remote_hash,
                    body_hash: canonical_document_body_hash(&remote_changed),
                    meta_hash: canonical_document_meta_hash(&remote_changed),
                    meta_updated_at_ms: 2000,
                    document: Some(remote_changed),
                }],
                downloaded_folders: Vec::new(),
                updated_prefs: None,
                ack_prefs: None,
                pushed_count: 0,
                pulled_count: 1,
                synced_at_ms: 2500,
            })
            .unwrap();

        assert_eq!(session.notebooks.len(), 1);
        assert!(
            !session
                .notebooks
                .iter()
                .any(|d| d.title.contains("Conflict Copy"))
        );
    }

    #[test]
    fn web_cache_never_persists_cloud_notebooks_or_tokens() {
        let mut session = Session::open_web_cache(None);
        session
            .execute(
                "profile.completeFirstAccess",
                &json!({ "nickname": "WebMe" }),
            )
            .unwrap();
        let start_count = session.notebooks.len();
        session
            .execute("notebook.create", &json!({ "title": "Web Local Note" }))
            .unwrap();
        assert_eq!(session.notebooks.len(), start_count + 1);

        let acct = fake_account("web-u1", "web@example.com", "web_user");
        let cloud_doc = Document {
            id: "cloud-1".to_string(),
            title: "Secret Cloud Note".to_string(),
            ..Default::default()
        };
        let pull = SyncPullResponse {
            server_cursor: 3,
            user: acct.user.clone(),
            notebooks: vec![RemoteNotebookRecord {
                notebook_id: "cloud-1".to_string(),
                version: 1,
                cursor: 3,
                deleted: false,
                updated_at_ms: 1000,
                deleted_at_ms: None,
                title: "Secret Cloud Note".to_string(),
                content_hash: canonical_document_hash(&cloud_doc),
                body_hash: canonical_document_body_hash(&cloud_doc),
                meta_hash: canonical_document_meta_hash(&cloud_doc),
                meta_updated_at_ms: 1000,
                document: Some(cloud_doc),
            }],
            folders: Vec::new(),
            studio_prefs: None,
        };
        session.enter_online_server_session(acct, pull);
        session
            .execute("notebook.create", &json!({ "title": "Extra Cloud Note" }))
            .unwrap();

        let cached_json = session.export_web_cache_json().unwrap();
        assert!(!cached_json.contains("Secret Cloud Note"));
        assert!(!cached_json.contains("Extra Cloud Note"));
        assert!(!cached_json.contains("acc-secret-token"));
        assert!(!cached_json.contains("ref-secret-token"));
        assert!(cached_json.contains("web@example.com"));

        // Reloading browser page restores ONLY the offline local profile & server shortcut!
        let reloaded = Session::open_web_cache(Some(&cached_json));
        assert!(!reloaded.is_signed_in());
        assert!(!reloaded.is_viewing_cloud());
        assert!(has_notebook_title(&reloaded, "Web Local Note"));
        assert!(!has_notebook_title(&reloaded, "Secret Cloud Note"));
        assert_eq!(reloaded.remembered_server_accounts().len(), 1);
    }

    #[test]
    fn local_account_conversion_uploads_everything_and_deletes_local_storage() {
        let dir = temp_dir("convert_local");
        let mut session = Session::open_desktop_library(dir.0.clone());
        session
            .execute(
                "profile.completeFirstAccess",
                &json!({ "nickname": "Bruno" }),
            )
            .unwrap();
        let bruno_id = session
            .active_local_profile()
            .unwrap()
            .local_profile_id
            .clone();
        session
            .execute("library.folder.create", &json!({ "path": "Physics" }))
            .unwrap();
        session
            .execute(
                "notebook.create",
                &json!({ "title": "Mechanics", "folder_path": "Physics" }),
            )
            .unwrap();
        session
            .execute("theme.set", &json!({ "theme": "Dark" }))
            .unwrap();
        let bruno_dir = papercraft_sync::profile_library_dir(&dir.0, &bruno_id);
        assert!(bruno_dir.is_dir());

        let (nick, batch) = session
            .prepare_local_account_conversion_batch(&bruno_id)
            .unwrap();
        assert_eq!(nick, "Bruno");
        assert!(batch.force_overwrite);
        assert_eq!(batch.local_docs.len(), 1);
        assert_eq!(batch.local_docs[0].doc.title, "Mechanics");
        assert_eq!(batch.folder_changes.len(), 1);
        assert_eq!(batch.folder_changes[0].path, "Physics");
        assert_eq!(
            batch.prefs_push.as_ref().unwrap().prefs.theme,
            UiTheme::Dark
        );
        // Local directory still untouched before completion!
        assert!(bruno_dir.is_dir());

        let acct = fake_account("user-bruno", "bruno@example.com", "bruno");
        let uploaded_doc = batch.local_docs[0].doc.clone();
        let pull = SyncPullResponse {
            server_cursor: 1,
            user: acct.user.clone(),
            notebooks: vec![RemoteNotebookRecord {
                notebook_id: uploaded_doc.id.clone(),
                version: 1,
                cursor: 1,
                deleted: false,
                updated_at_ms: 500,
                deleted_at_ms: None,
                title: uploaded_doc.title.clone(),
                content_hash: canonical_document_hash(&uploaded_doc),
                body_hash: canonical_document_body_hash(&uploaded_doc),
                meta_hash: canonical_document_meta_hash(&uploaded_doc),
                meta_updated_at_ms: 500,
                document: Some(uploaded_doc),
            }],
            folders: Vec::new(),
            studio_prefs: None,
        };
        session.complete_local_account_conversion(&bruno_id, acct, pull);

        assert!(!bruno_dir.exists());
        assert!(!session.has_saved_local_accounts());
        assert!(session.local_profiles().is_empty());
        assert!(session.is_viewing_cloud());
        assert!(has_notebook_title(&session, "Mechanics"));
        assert!(session.library_dir().is_none());

        // Signing out when 0 local accounts remain requires starting a fresh local session.
        let has_locals = session.sign_out_from_server_session();
        assert!(!has_locals);
        assert!(session.needs_initial_nickname());
    }

    #[test]
    fn first_access_nickname_and_local_discovery_report_work_end_to_end() {
        let dir = temp_dir("first_access");
        let mut session = Session::open_desktop_library(dir.0.clone());
        assert!(session.needs_initial_nickname());
        assert!(!session.has_saved_local_accounts());
        let before = session.local_discovery_report();
        assert_eq!(before.unsynced_local_accounts, 0);
        assert_eq!(before.total_local_notebooks, 0);

        assert!(
            session
                .execute("profile.completeFirstAccess", &json!({ "nickname": "   " }))
                .is_err()
        );
        assert!(session.needs_initial_nickname());

        session
            .execute(
                "profile.completeFirstAccess",
                &json!({ "nickname": "Bruno" }),
            )
            .unwrap();
        assert!(!session.needs_initial_nickname());
        assert!(session.has_saved_local_accounts());
        assert_eq!(session.active_local_profile().unwrap().nickname, "Bruno");

        let after = session.local_discovery_report();
        assert_eq!(after.unsynced_local_accounts, 1);
        assert_eq!(after.total_local_notebooks, session.notebooks.len());

        let reopened = Session::open_desktop_library(dir.0.clone());
        assert!(!reopened.needs_initial_nickname());
        assert_eq!(reopened.active_local_profile().unwrap().nickname, "Bruno");
    }

    #[test]
    fn welcome_banner_shows_once_per_release_on_persisted_desktop_and_web_sessions() {
        let ephemeral = Session::default();
        assert!(!ephemeral.should_show_welcome_banner());

        let dir = temp_dir("welcome_banner");
        let mut desktop = Session::open_desktop_library(dir.0.clone());
        assert!(desktop.should_show_welcome_banner());
        desktop
            .execute("profile.dismissWelcomeBanner", &json!({}))
            .unwrap();
        assert!(!desktop.should_show_welcome_banner());
        assert_eq!(
            desktop.welcome_banner_seen_version(),
            Some(env!("CARGO_PKG_VERSION"))
        );

        let reopened_desktop = Session::open_desktop_library(dir.0.clone());
        assert!(!reopened_desktop.should_show_welcome_banner());
        assert!(reopened_desktop.should_show_welcome_banner_for_version("99.0.0"));

        let mut legacy = papercraft_sync::load_global_state(&dir.0);
        legacy.welcome_banner_seen = true;
        legacy.welcome_banner_seen_version = None;
        papercraft_sync::save_global_state(&dir.0, &legacy).unwrap();
        let updated_from_legacy = Session::open_desktop_library(dir.0.clone());
        assert!(updated_from_legacy.should_show_welcome_banner());

        let mut web = Session::open_web_cache(None);
        assert!(web.should_show_welcome_banner());
        web.execute("profile.dismissWelcomeBanner", &json!({}))
            .unwrap();
        assert!(!web.should_show_welcome_banner());
        let saved_web = web.export_web_cache_json().unwrap();
        let reopened_web = Session::open_web_cache(Some(&saved_web));
        assert!(!reopened_web.should_show_welcome_banner());
        assert!(reopened_web.should_show_welcome_banner_for_version("99.0.0"));
    }

    #[test]
    fn first_notebook_data_notice_shows_once_on_persisted_desktop_and_web_sessions() {
        let ephemeral = Session::default();
        assert!(!ephemeral.should_show_first_notebook_data_notice());

        let dir = temp_dir("first_notebook_data_notice");
        let mut desktop = Session::open_desktop_library(dir.0.clone());
        assert!(desktop.should_show_first_notebook_data_notice());
        desktop
            .execute("profile.dismissFirstNotebookDataNotice", &json!({}))
            .unwrap();
        assert!(!desktop.should_show_first_notebook_data_notice());

        let reopened_desktop = Session::open_desktop_library(dir.0.clone());
        assert!(!reopened_desktop.should_show_first_notebook_data_notice());

        let mut web = Session::open_web_cache(None);
        assert!(web.should_show_first_notebook_data_notice());
        web.execute("profile.dismissFirstNotebookDataNotice", &json!({}))
            .unwrap();
        assert!(!web.should_show_first_notebook_data_notice());
        let saved_web = web.export_web_cache_json().unwrap();
        let reopened_web = Session::open_web_cache(Some(&saved_web));
        assert!(!reopened_web.should_show_first_notebook_data_notice());
    }

    #[test]
    fn package_export_and_import_restore_all_notebooks_trash_and_prefs() {
        let dir_a = temp_dir("pkg_src");
        let mut src = Session::open_desktop_library(dir_a.0.clone());
        src.execute(
            "profile.completeFirstAccess",
            &json!({ "nickname": "Bruno" }),
        )
        .unwrap();
        src.execute(
            "notebook.create",
            &json!({ "title": "Math", "folder_path": "School" }),
        )
        .unwrap();
        src.execute("notebook.create", &json!({ "title": "Draft To Trash" }))
            .unwrap();
        let trash_idx = src
            .notebooks
            .iter()
            .position(|d| d.title == "Draft To Trash")
            .unwrap();
        src.execute("notebook.delete", &json!({ "index": trash_idx }))
            .unwrap();
        src.execute("library.folder.create", &json!({ "path": "School" }))
            .unwrap();
        src.execute("theme.set", &json!({ "theme": "Dark" }))
            .unwrap();

        let pkg_path = dir_a.0.join("full_backup.pcraftpkg");
        let export_res = src
            .execute(
                "package.export",
                &json!({ "path": pkg_path.to_string_lossy() }),
            )
            .unwrap();
        assert!(export_res["bytes"].as_u64().unwrap() > 0);

        let dir_b = temp_dir("pkg_dst");
        let mut dst = Session::open_desktop_library(dir_b.0.clone());
        dst.execute(
            "profile.completeFirstAccess",
            &json!({ "nickname": "Alyx" }),
        )
        .unwrap();
        let import_res = dst
            .execute(
                "package.import",
                &json!({ "path": pkg_path.to_string_lossy() }),
            )
            .unwrap();
        assert!(import_res["importedNotebooks"].as_u64().unwrap() >= 1);
        assert_eq!(import_res["importedTrashed"], 1);
        assert!(dst.notebooks.iter().any(|n| n.title == "Math"));
        assert!(
            dst.trashed_notebooks
                .iter()
                .any(|t| t.doc.title == "Draft To Trash")
        );
        assert!(dst.prefs.library_folders.iter().any(|f| f == "School"));
        assert_eq!(dst.prefs.theme, papercraft_doc::UiTheme::Dark);
    }

    #[test]
    fn profile_commands_create_switch_rename_and_delete_local_profiles() {
        let dir = temp_dir("profile_cmds");
        let mut session = Session::open_desktop_library(dir.0.clone());
        session
            .execute(
                "profile.completeFirstAccess",
                &json!({ "nickname": "Primary" }),
            )
            .unwrap();
        let primary_id = session
            .active_local_profile()
            .unwrap()
            .local_profile_id
            .clone();
        session
            .execute("notebook.create", &json!({ "title": "Primary Note" }))
            .unwrap();

        let created_test = session
            .execute(
                "profile.create",
                &json!({ "nickname": "test", "switchNow": true }),
            )
            .unwrap();
        let test_id = created_test["localProfileId"].as_str().unwrap().to_string();
        assert_eq!(session.active_local_profile().unwrap().nickname, "test");
        assert!(!session.notebooks.iter().any(|n| n.title == "Primary Note"));

        let created_test2 = session
            .execute(
                "profile.create",
                &json!({ "nickname": "test2", "switch_now": false }),
            )
            .unwrap();
        let test2_id = created_test2["localProfileId"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(session.active_local_profile().unwrap().nickname, "test");

        session
            .execute("profile.switch", &json!({ "profileId": test2_id }))
            .unwrap();
        assert_eq!(session.active_local_profile().unwrap().nickname, "test2");

        session
            .execute(
                "profile.switch",
                &json!({ "target": "local", "profile_id": primary_id }),
            )
            .unwrap();
        assert_eq!(session.active_local_profile().unwrap().nickname, "Primary");
        assert!(session.notebooks.iter().any(|n| n.title == "Primary Note"));

        session
            .execute(
                "profile.rename",
                &json!({ "profileId": test2_id, "nickname": "Work" }),
            )
            .unwrap();
        assert!(
            session
                .local_profiles()
                .iter()
                .any(|p| p.local_profile_id == test2_id && p.nickname == "Work")
        );

        session
            .execute("profile.delete", &json!({ "profileId": test2_id }))
            .unwrap();
        assert!(
            !session
                .local_profiles()
                .iter()
                .any(|p| p.local_profile_id == test2_id)
        );
        let _ = test_id;
    }

    #[test]
    fn local_discovery_report_counts_without_creating_missing_profile_dirs() {
        let dir = temp_dir("discovery_no_mkdir");
        let mut session = Session::open_desktop_library(dir.0.clone());
        session
            .execute(
                "profile.completeFirstAccess",
                &json!({ "nickname": "Primary" }),
            )
            .unwrap();
        let primary_count = session.notebooks.len();

        let uninitialized = session.create_local_profile("Unused", false).unwrap();
        let uninitialized_dir =
            papercraft_sync::profile_library_dir(&dir.0, &uninitialized.local_profile_id);
        assert!(!uninitialized_dir.exists());

        let secondary = session.create_local_profile("School", true).unwrap();
        session
            .execute("notebook.create", &json!({ "title": "Calculus" }))
            .unwrap();
        let school_count = session.notebooks.len();

        let report = session.local_discovery_report();
        assert_eq!(report.unsynced_local_accounts, 3);
        assert_eq!(report.total_local_notebooks, primary_count + school_count);
        assert!(
            !uninitialized_dir.exists(),
            "local_discovery_report must never create missing profile directories on disk"
        );
        assert_ne!(secondary.local_profile_id, uninitialized.local_profile_id);
    }

    #[test]
    fn merge_studio_prefs_propagates_pen_smoothings_on_overwrite() {
        let mut target = papercraft_doc::UserStudioConfig::default();
        let mut incoming = papercraft_doc::UserStudioConfig::default();
        incoming.pen_smoothings[papercraft_doc::PenKind::FountainPen.index()] =
            papercraft_doc::StabilizationPreset::CurvedBezier
                .to_smoothing_config_with_strength(0.75);
        super::merge_studio_prefs(&mut target, incoming.clone(), false);
        assert_eq!(
            target.pen_smoothings[papercraft_doc::PenKind::FountainPen.index()].strength_percent(),
            50
        );
        super::merge_studio_prefs(&mut target, incoming, true);
        assert_eq!(
            target.pen_smoothings[papercraft_doc::PenKind::FountainPen.index()].preset(),
            papercraft_doc::StabilizationPreset::CurvedBezier
        );
        assert_eq!(
            target.pen_smoothings[papercraft_doc::PenKind::FountainPen.index()].strength_percent(),
            75
        );
    }
}
