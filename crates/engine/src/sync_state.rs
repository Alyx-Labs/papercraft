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
    ActiveAccountSession, ActiveWorkspaceTarget, DocumentMergeBase, EntitySyncStatus,
    FolderSyncMeta, GLOBAL_SYNC_FILE, GlobalSyncState, LocalNotebookSnapshot, LocalProfile,
    MergeDecision, MergeKind, OwnerId, PageElementFingerprints, RememberedServerAccount,
    RemoteNotebookUpdate, SyncBatch, SyncConnectionState, SyncCycleOutcome, SyncManifest,
    SyncStatusSummary, account_library_dir, compute_document_page_elements, default_device_name,
    load_global_state, load_manifest, merge_documents_three_way, outcome_from_full_pull,
    profile_library_dir, sanitize_account_dir_segment, sanitize_client_id,
    sanitize_profile_nickname, sanitize_release_version, sanitize_remembered_accounts,
    save_global_state, save_manifest, strip_conflict_copy_suffix,
};
use papercraft_sync_protocol::{
    DEFAULT_SERVER_URL, LocalDiscoveryReport, LocalFolderChange, LocalPrefsPush,
    RemoteFolderRecord, SyncPullResponse, canonical_document_body_hash, canonical_document_hash,
    canonical_document_page_hashes, canonical_prefs_hash, normalize_server_url, now_unix_ms,
    sanitize_device_name,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::library::{
    LIBRARY_MARKER_FILE, LibraryInitMode, LibraryState, STUDIO_PREFS_FILE, TrashedNotebook,
    count_ncraft_files_in_dir, seed_starter_library_prefs, starter_documents,
};
use crate::web_storage::{
    WebPersistTracker, WebStorageDelta, WebStorageLoadedState, build_tracker_from_loaded_state,
    cloud_workspace_key, local_workspace_key, trashed_to_cached_entry,
    workspace_bundle_to_loaded_meta,
};
use crate::{MAX_NOTEBOOKS, Session};

const WEB_CACHE_VERSION: u32 = 4;
const LEGACY_WEB_CACHE_VERSION_1: u32 = 1;
const LEGACY_WEB_CACHE_VERSION_2: u32 = 2;
const LEGACY_WEB_CACHE_VERSION_3: u32 = 3;
const TRASH_SUBDIR: &str = "trash";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct CachedTrashEntry {
    #[serde(default)]
    pub(crate) trash_id: String,
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

/// Persisted browser `localStorage` payload storing offline local profiles, cached cloud
/// account workspaces, pending sync manifests, and server shortcuts.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct WebWorkspaceCache {
    pub(crate) version: u32,
    pub(crate) global: GlobalSyncState,
    #[serde(default)]
    pub(crate) guest: Option<CachedLibraryBundle>,
    #[serde(default)]
    pub(crate) local_profiles: HashMap<String, CachedLibraryBundle>,
    #[serde(default)]
    pub(crate) cloud_accounts: HashMap<String, CachedLibraryBundle>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReleaseNoticeKind {
    WelcomeBanner,
    FirstNotebookDataNotice,
    InfiniteThemeNotice,
}

#[derive(Clone, Debug)]
pub(crate) struct SessionSyncState {
    pub(crate) desktop_root: Option<PathBuf>,
    pub(crate) track_manifest: bool,
    pub(crate) web_cache_enabled: bool,
    pub(crate) web_local_bundles: HashMap<String, CachedLibraryBundle>,
    pub(crate) web_cloud_bundles: HashMap<String, CachedLibraryBundle>,
    pub(crate) web_tracker: WebPersistTracker,
    pub(crate) pending_web_delta: WebStorageDelta,
    pub(crate) global: GlobalSyncState,
    /// Active authenticated server session (persisted inside `global.cached_server_sessions` for Offline-First access).
    pub(crate) active_server_session: Option<ActiveAccountSession>,
    pub(crate) manifest: SyncManifest,
    pub(crate) slot_ids: Vec<String>,
    pub(crate) slot_dirty_hash: Vec<bool>,
    pub(crate) slot_edit_gen: Vec<u64>,
    pub(crate) in_flight_slot_gens: HashMap<String, u64>,
    pub(crate) slot_placeholder_blank: Vec<bool>,
    pub(crate) prefs_dirty_pending: bool,
    pub(crate) connection: SyncConnectionState,
    pub(crate) last_error: Option<String>,
    pub(crate) last_conflict_notice: Option<String>,
    pub(crate) last_dirty_at_ms: Option<i64>,
    pub(crate) sync_in_flight: bool,
    pub(crate) network_probed_offline: bool,
    pub(crate) session_expired: bool,
}

impl Default for SessionSyncState {
    fn default() -> Self {
        Self {
            desktop_root: None,
            track_manifest: false,
            web_cache_enabled: false,
            web_local_bundles: HashMap::new(),
            web_cloud_bundles: HashMap::new(),
            web_tracker: WebPersistTracker::default(),
            pending_web_delta: WebStorageDelta::default(),
            global: GlobalSyncState::with_defaults(default_device_name()),
            active_server_session: None,
            manifest: SyncManifest::default(),
            slot_ids: vec![String::new()],
            slot_dirty_hash: vec![false],
            slot_edit_gen: vec![0],
            in_flight_slot_gens: HashMap::new(),
            slot_placeholder_blank: vec![false],
            prefs_dirty_pending: false,
            connection: SyncConnectionState::SignedOut,
            last_error: None,
            last_conflict_notice: None,
            last_dirty_at_ms: None,
            sync_in_flight: false,
            network_probed_offline: false,
            session_expired: false,
        }
    }
}

impl Session {
    /// Returns `true` when local persistence (desktop directory or browser local storage) is enabled.
    pub fn is_sync_persistence_enabled(&self) -> bool {
        self.sync.desktop_root.is_some() || self.sync.web_cache_enabled
    }

    /// Opens (or initializes) a browser workspace from hydrated IndexedDB state, restoring
    /// either the last active Offline-First server account cache or the active local profile.
    pub fn open_web_storage(loaded: Option<WebStorageLoadedState>) -> Self {
        let Some(mut state) = loaded else {
            return Self::open_web_cache(None);
        };
        sanitize_global_state(&mut state.global);
        let tracker = build_tracker_from_loaded_state(&state);
        let mut local_profiles = HashMap::new();
        let mut cloud_accounts = HashMap::new();
        for ws in &state.workspaces {
            let bundle = workspace_bundle_to_loaded_meta(ws);
            if let Some(pid) = ws.workspace_key.strip_prefix("local:") {
                local_profiles.insert(pid.to_string(), bundle);
            } else if let Some(acct_key) = ws.workspace_key.strip_prefix("cloud:") {
                cloud_accounts.insert(acct_key.to_string(), bundle);
            }
        }
        let mut session = Self::open_web_cache_struct(WebWorkspaceCache {
            version: WEB_CACHE_VERSION,
            global: state.global,
            guest: None,
            local_profiles,
            cloud_accounts,
        });
        session.sync.web_tracker = tracker;
        session
    }

    /// Opens (or initializes) a browser workspace, restoring either the last active
    /// Offline-First server account cache or the active local profile.
    pub fn open_web_cache(raw_json: Option<&str>) -> Self {
        if let Some(cache) = raw_json
            .and_then(|raw| serde_json::from_str::<WebWorkspaceCache>(raw).ok())
            .filter(|c| {
                matches!(
                    c.version,
                    LEGACY_WEB_CACHE_VERSION_1
                        | LEGACY_WEB_CACHE_VERSION_2
                        | LEGACY_WEB_CACHE_VERSION_3
                        | WEB_CACHE_VERSION
                )
            })
        {
            return Self::open_web_cache_struct(cache);
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

    fn open_web_cache_struct(mut cache: WebWorkspaceCache) -> Self {
        sanitize_global_state(&mut cache.global);
        migrate_web_cache_if_needed(&mut cache);

        if let ActiveWorkspaceTarget::Cloud {
            ref server_url,
            ref user_id,
        } = cache.global.active_workspace
            && let Some(cached_acct) = cache
                .global
                .cached_server_session(server_url, user_id)
                .cloned()
        {
            let key = cloud_account_cache_key(&cached_acct.server_url, &cached_acct.user.user_id);
            let bundle = cache.cloud_accounts.get(&key).cloned().unwrap_or_default();
            let mut session = restore_web_bundle(&bundle);
            session.sync.global = cache.global;
            session.sync.web_cache_enabled = true;
            session.sync.track_manifest = true;
            session.sync.web_local_bundles = cache.local_profiles;
            session.sync.web_cloud_bundles = cache.cloud_accounts;
            session.sync.manifest.owner_id =
                Some(OwnerId::CloudUser(cached_acct.user.user_id.clone()));
            session.sync.active_server_session = Some(cached_acct);
            session.sync.connection = SyncConnectionState::Offline;
            session.sync_reconcile_slots();
            session.restore_active_notebook_from_manifest(false);
            return session;
        }

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
            session.sync.web_cloud_bundles = cache.cloud_accounts;
            session.sync.connection = SyncConnectionState::SignedOut;
            session.sync.manifest.owner_id = Some(OwnerId::LocalProfile(profile_id));
            session.sync_reconcile_slots();
            session.restore_active_notebook_from_manifest(false);
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
        session.sync.web_cloud_bundles = cache.cloud_accounts;
        session.sync.connection = SyncConnectionState::SignedOut;
        let pid = ensure_default_profile_in_global(&mut session.sync.global);
        session.sync.manifest.owner_id = Some(OwnerId::LocalProfile(pid));
        session.sync_reconcile_slots();
        session
    }

    /// Snapshots local profiles, cached Offline-First server workspaces, and server shortcuts.
    pub fn export_web_cache_json(&mut self) -> Result<String, String> {
        self.sync_flush_dirty_flags_to_manifest();
        self.sync_reconcile_folders();
        self.save_current_web_bundle();
        let cache = WebWorkspaceCache {
            version: WEB_CACHE_VERSION,
            global: self.sync.global.clone(),
            guest: None,
            local_profiles: self.sync.web_local_bundles.clone(),
            cloud_accounts: self.sync.web_cloud_bundles.clone(),
        };
        serde_json::to_string(&cache)
            .map_err(|err| format!("failed to serialize web workspace cache: {err}"))
    }

    pub(crate) fn save_current_web_bundle(&mut self) {
        if !self.sync.web_cache_enabled {
            return;
        }
        if let Some(ref acct) = self.sync.active_server_session {
            let current = self.capture_web_bundle();
            let key = cloud_account_cache_key(&acct.server_url, &acct.user.user_id);
            self.sync.web_cloud_bundles.insert(key, current);
            return;
        }
        let Some(OwnerId::LocalProfile(pid)) = self.sync.manifest.owner_id.clone() else {
            return;
        };
        if pid.is_empty()
            || !self
                .sync
                .global
                .local_profiles
                .iter()
                .any(|p| p.local_profile_id == pid)
        {
            return;
        }
        let current = self.capture_web_bundle();
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
            .map(trashed_to_cached_entry)
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

    /// Opens the desktop library root, restoring either an Offline-First cached server account
    /// (if previously active) or the active local profile.
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
        if save_global {
            let _ = save_global_state(&root_dir, &global);
        }

        if let ActiveWorkspaceTarget::Cloud {
            ref server_url,
            ref user_id,
        } = global.active_workspace
            && let Some(cached_acct) = global.cached_server_session(server_url, user_id).cloned()
        {
            let acct_dir = account_library_dir(&root_dir, &cached_acct.user.user_id);
            let (mut session, is_placeholder) =
                Self::open_library_mode(acct_dir, LibraryInitMode::EmptyClean);
            let uid = cached_acct.user.user_id.clone();
            session.sync.desktop_root = Some(root_dir);
            session.sync.track_manifest = true;
            session.sync.global = global;
            session.sync.active_server_session = Some(cached_acct);
            session.sync.connection = SyncConnectionState::Offline;
            session.sync_init_from_disk(is_placeholder, OwnerId::CloudUser(uid));
            return session;
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

    /// Returns `true` when `(server_url, user_id)` has a persisted Offline-First workspace cache
    /// and saved session on this device.
    pub fn has_cached_server_account(&self, server_url: &str, user_id: &str) -> bool {
        self.sync
            .global
            .has_cached_server_session(server_url, user_id)
    }

    /// Returns the persisted cached session for `(server_url, user_id)`, if present on this device.
    pub fn cached_server_session_for(
        &self,
        server_url: &str,
        user_id: &str,
    ) -> Option<ActiveAccountSession> {
        self.sync
            .global
            .cached_server_session(server_url, user_id)
            .cloned()
    }

    /// Counts pending unpushed local changes stored in `(server_url, user_id)`'s local cache.
    pub fn server_account_pending_changes_count(&self, server_url: &str, user_id: &str) -> usize {
        if self.sync.active_server_session.as_ref().is_some_and(|s| {
            s.user.user_id == user_id
                && (s.server_url.eq_ignore_ascii_case(server_url)
                    || normalize_server_url(server_url)
                        .is_ok_and(|u| u.eq_ignore_ascii_case(&s.server_url)))
        }) {
            return self.cloud_pending_changes_count();
        }
        if let Some(ref root) = self.sync.desktop_root {
            let dir = account_library_dir(root, user_id);
            if dir.exists() {
                return load_manifest(&dir).pending_changes_count();
            }
            return 0;
        }
        if self.sync.web_cache_enabled {
            let key = cloud_account_cache_key(server_url, user_id);
            if let Some(bundle) = self.sync.web_cloud_bundles.get(&key) {
                let mut count = bundle.manifest.pending_changes_count();
                for (idx, dirty) in bundle.slot_dirty_hash.iter().enumerate() {
                    if !*dirty
                        || bundle
                            .slot_placeholder_blank
                            .get(idx)
                            .copied()
                            .unwrap_or(false)
                    {
                        continue;
                    }
                    let already_counted = bundle
                        .slot_ids
                        .get(idx)
                        .and_then(|id| bundle.manifest.meta(id))
                        .is_some_and(|m| m.cloud_dirty || m.server_version == 0);
                    if !already_counted {
                        count += 1;
                    }
                }
                if bundle.prefs_dirty_pending && !bundle.manifest.prefs_dirty {
                    count += 1;
                }
                return count;
            }
        }
        0
    }

    /// Takes the most recent conflict resolution summary banner produced by the sync engine.
    pub fn take_last_conflict_notice(&mut self) -> Option<String> {
        self.sync.last_conflict_notice.take()
    }

    pub fn has_saved_local_accounts(&self) -> bool {
        self.sync.global.has_saved_local_accounts()
    }

    /// Returns `true` when at least one saved local profile or remembered server account exists.
    pub fn has_any_saved_accounts(&self) -> bool {
        self.has_saved_local_accounts() || !self.remembered_server_accounts().is_empty()
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

    /// Returns `true` when a self-hosted sync server has been configured or previously linked on this device.
    pub fn is_server_configured(&self) -> bool {
        self.sync.global.server_configured
            || self.is_signed_in()
            || !self.sync.global.remembered_server_accounts.is_empty()
    }

    /// Returns the user's configured default/preferred server URL independent of any active session.
    pub fn preferred_server_url(&self) -> &str {
        &self.sync.global.server_url
    }

    /// Returns `true` when the user opted to skip the startup account picker modal.
    pub fn skip_account_picker_on_startup(&self) -> bool {
        self.sync.global.skip_account_picker_on_startup
    }

    /// Updates whether the startup account picker modal should be skipped on next launch.
    pub fn set_skip_account_picker_on_startup(&mut self, skip: bool) {
        if self.sync.global.skip_account_picker_on_startup == skip {
            return;
        }
        self.sync.global.skip_account_picker_on_startup = skip;
        self.sync_persist_global_state();
        self.capture_active_web_delta_into_pending(true);
        self.bump_revision();
    }

    /// Returns `true` when either a cloud account or a confirmed local profile workspace is loaded.
    pub fn has_loaded_workspace(&self) -> bool {
        if !self.is_sync_persistence_enabled() {
            return true;
        }
        self.is_viewing_cloud()
            || (!self.sync.global.needs_initial_nickname
                && matches!(
                    self.sync.manifest.owner_id,
                    Some(OwnerId::LocalProfile(ref id)) if !id.is_empty()
                ))
    }

    /// Returns `true` when the startup account picker modal should appear on startup
    /// (requires persistence enabled, at least one saved local/server account, and skip unchecked).
    pub fn should_show_startup_account_picker(&self) -> bool {
        if !self.is_sync_persistence_enabled() || !self.has_any_saved_accounts() {
            return false;
        }
        self.needs_initial_nickname() || !self.sync.global.skip_account_picker_on_startup
    }

    /// Returns the number of cached/offline notebooks belonging to `(server_url, user_id)`.
    pub fn server_account_notebook_count(&self, server_url: &str, user_id: &str) -> usize {
        if self.sync.active_server_session.as_ref().is_some_and(|s| {
            s.user.user_id == user_id
                && (s.server_url.eq_ignore_ascii_case(server_url)
                    || normalize_server_url(server_url)
                        .is_ok_and(|u| u.eq_ignore_ascii_case(&s.server_url)))
        }) {
            return count_non_placeholder_slots(
                self.notebooks.len(),
                &self.sync.slot_placeholder_blank,
            );
        }
        if let Some(ref root) = self.sync.desktop_root {
            let dir = account_library_dir(root, user_id);
            if dir.exists() {
                return count_ncraft_files_in_dir(&dir);
            }
            return 0;
        }
        if self.sync.web_cache_enabled {
            let key = cloud_account_cache_key(server_url, user_id);
            if let Some(bundle) = self.sync.web_cloud_bundles.get(&key) {
                return count_non_placeholder_slots(
                    bundle.notebooks.len(),
                    &bundle.slot_placeholder_blank,
                );
            }
        }
        0
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

    fn release_notice_seen_version(&self, kind: ReleaseNoticeKind) -> Option<&str> {
        match kind {
            ReleaseNoticeKind::WelcomeBanner => {
                self.sync.global.welcome_banner_seen_version.as_deref()
            }
            ReleaseNoticeKind::FirstNotebookDataNotice => self
                .sync
                .global
                .first_notebook_data_notice_seen_version
                .as_deref(),
            ReleaseNoticeKind::InfiniteThemeNotice => self
                .sync
                .global
                .infinite_theme_notice_seen_version
                .as_deref(),
        }
    }

    fn should_show_release_notice_for_version(
        &self,
        kind: ReleaseNoticeKind,
        current_version: &str,
    ) -> bool {
        if !self.is_sync_persistence_enabled() {
            return false;
        }
        let Some(target) = sanitize_release_version(current_version) else {
            return false;
        };
        self.release_notice_seen_version(kind) != Some(target.as_str())
    }

    fn dismiss_release_notice_for_version(
        &mut self,
        kind: ReleaseNoticeKind,
        current_version: &str,
    ) {
        let Some(target) = sanitize_release_version(current_version) else {
            return;
        };
        let (seen, seen_version) = match kind {
            ReleaseNoticeKind::WelcomeBanner => (
                &mut self.sync.global.welcome_banner_seen,
                &mut self.sync.global.welcome_banner_seen_version,
            ),
            ReleaseNoticeKind::FirstNotebookDataNotice => (
                &mut self.sync.global.first_notebook_data_notice_seen,
                &mut self.sync.global.first_notebook_data_notice_seen_version,
            ),
            ReleaseNoticeKind::InfiniteThemeNotice => (
                &mut self.sync.global.infinite_theme_notice_seen,
                &mut self.sync.global.infinite_theme_notice_seen_version,
            ),
        };
        if *seen && seen_version.as_deref() == Some(target.as_str()) {
            return;
        }
        *seen = true;
        *seen_version = Some(target);
        self.sync_persist_global_state();
        self.save_current_web_bundle();
        self.capture_active_web_delta_into_pending(true);
        self.bump_revision();
    }

    /// Returns `true` when the release welcome banner has not yet been dismissed
    /// for `current_version` on a persisted Desktop or Web workspace.
    pub fn should_show_welcome_banner_for_version(&self, current_version: &str) -> bool {
        self.should_show_release_notice_for_version(
            ReleaseNoticeKind::WelcomeBanner,
            current_version,
        )
    }

    /// Returns `true` when the release welcome banner has not yet been dismissed
    /// for the running build's version on a persisted Desktop or Web workspace.
    pub fn should_show_welcome_banner(&self) -> bool {
        self.should_show_welcome_banner_for_version(env!("CARGO_PKG_VERSION"))
    }

    /// Returns the last release version for which the welcome banner was dismissed.
    pub fn welcome_banner_seen_version(&self) -> Option<&str> {
        self.release_notice_seen_version(ReleaseNoticeKind::WelcomeBanner)
    }

    /// Marks the release welcome banner as seen for `current_version` and persists global state.
    pub fn dismiss_welcome_banner_for_version(&mut self, current_version: &str) {
        self.dismiss_release_notice_for_version(ReleaseNoticeKind::WelcomeBanner, current_version);
    }

    /// Marks the release welcome banner as seen for the running build's version.
    pub fn dismiss_welcome_banner(&mut self) {
        self.dismiss_welcome_banner_for_version(env!("CARGO_PKG_VERSION"));
    }

    /// Returns `true` when the first-notebook data & privacy notice has not yet been dismissed
    /// for `current_version` on a persisted Desktop or Web workspace.
    pub fn should_show_first_notebook_data_notice_for_version(
        &self,
        current_version: &str,
    ) -> bool {
        self.should_show_release_notice_for_version(
            ReleaseNoticeKind::FirstNotebookDataNotice,
            current_version,
        )
    }

    /// Returns `true` when the first-notebook data & privacy notice has not yet been dismissed
    /// for the running build's version on a persisted Desktop or Web workspace.
    pub fn should_show_first_notebook_data_notice(&self) -> bool {
        self.should_show_first_notebook_data_notice_for_version(env!("CARGO_PKG_VERSION"))
    }

    /// Returns the last release version for which the first-notebook data & privacy notice was dismissed.
    pub fn first_notebook_data_notice_seen_version(&self) -> Option<&str> {
        self.release_notice_seen_version(ReleaseNoticeKind::FirstNotebookDataNotice)
    }

    /// Marks the first-notebook data & privacy notice as seen for `current_version` and persists global state.
    pub fn dismiss_first_notebook_data_notice_for_version(&mut self, current_version: &str) {
        self.dismiss_release_notice_for_version(
            ReleaseNoticeKind::FirstNotebookDataNotice,
            current_version,
        );
    }

    /// Marks the first-notebook data & privacy notice as seen for the running build's version.
    pub fn dismiss_first_notebook_data_notice(&mut self) {
        self.dismiss_first_notebook_data_notice_for_version(env!("CARGO_PKG_VERSION"));
    }

    /// Returns `true` when the Infinite theme notice has not yet been dismissed
    /// for `current_version` on a persisted Desktop or Web workspace.
    pub fn should_show_infinite_theme_notice_for_version(&self, current_version: &str) -> bool {
        self.should_show_release_notice_for_version(
            ReleaseNoticeKind::InfiniteThemeNotice,
            current_version,
        )
    }

    /// Returns `true` when the Infinite theme notice has not yet been dismissed
    /// for the running build's version on a persisted Desktop or Web workspace.
    pub fn should_show_infinite_theme_notice(&self) -> bool {
        self.should_show_infinite_theme_notice_for_version(env!("CARGO_PKG_VERSION"))
    }

    /// Returns the last release version for which the Infinite theme notice was dismissed.
    pub fn infinite_theme_notice_seen_version(&self) -> Option<&str> {
        self.release_notice_seen_version(ReleaseNoticeKind::InfiniteThemeNotice)
    }

    /// Marks the Infinite theme notice as seen for `current_version` and persists global state.
    pub fn dismiss_infinite_theme_notice_for_version(&mut self, current_version: &str) {
        self.dismiss_release_notice_for_version(
            ReleaseNoticeKind::InfiniteThemeNotice,
            current_version,
        );
    }

    /// Marks the Infinite theme notice as seen for the running build's version.
    pub fn dismiss_infinite_theme_notice(&mut self) {
        self.dismiss_infinite_theme_notice_for_version(env!("CARGO_PKG_VERSION"));
    }

    pub fn set_network_offline_state(&mut self, offline: bool) {
        self.sync.network_probed_offline = offline;
        if offline && self.is_viewing_cloud() && !self.sync.sync_in_flight {
            self.sync.connection = SyncConnectionState::Offline;
        }
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

    /// Estimates pending binary payload bytes (embedded PDFs, image templates, and pasted images)
    /// across dirty cloud notebooks using shared `Arc` lengths with zero heap allocations.
    #[must_use]
    pub fn dirty_sync_estimated_payload_bytes(&self) -> usize {
        if !self.is_viewing_cloud() {
            return 0;
        }
        let mut total = 0usize;
        for (idx, doc) in self.notebooks.iter().enumerate() {
            if self
                .sync
                .slot_placeholder_blank
                .get(idx)
                .copied()
                .unwrap_or(false)
            {
                continue;
            }
            let slot_dirty = self.sync.slot_dirty_hash.get(idx).copied().unwrap_or(false);
            let meta_dirty = self
                .sync
                .slot_ids
                .get(idx)
                .and_then(|id| self.sync.manifest.meta(id))
                .is_none_or(|m| m.cloud_dirty || m.server_version == 0);
            if slot_dirty || meta_dirty {
                total = total.saturating_add(estimate_doc_attachment_bytes(doc));
            }
        }
        total
    }

    /// Returns `true` when the active cloud session's token expired and needs password re-entry.
    pub fn is_session_expired(&self) -> bool {
        self.is_viewing_cloud() && self.sync.session_expired
    }

    /// Marks the active cloud session expired (`unauthorized`) while keeping all local work safe.
    pub fn record_session_expired(&mut self, message: String) {
        self.sync.session_expired = true;
        self.record_sync_failure(false, message);
    }

    pub fn record_sync_failure(&mut self, offline: bool, message: String) {
        self.sync.sync_in_flight = false;
        self.sync.in_flight_slot_gens.clear();
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

    /// Clears the active in-flight flag without marking an error when a peer tab holds the
    /// cross-tab cloud sync leader lock.
    pub fn clear_sync_in_flight_for_peer(&mut self) {
        self.sync.sync_in_flight = false;
        self.sync.in_flight_slot_gens.clear();
        if self.sync.connection == SyncConnectionState::Syncing {
            self.sync.connection = if !self.is_viewing_cloud() {
                SyncConnectionState::SignedOut
            } else if self.sync.network_probed_offline {
                SyncConnectionState::Offline
            } else {
                SyncConnectionState::Synced
            };
        }
    }

    /// Persists unacknowledged push snapshot fingerprints so a subsequent retry after a dropped
    /// HTTP ACK recognizes the server record as its own prior push.
    pub fn record_pending_push_bases(&mut self, bases: HashMap<String, DocumentMergeBase>) {
        if bases.is_empty() {
            return;
        }
        for (nb_id, base) in bases {
            if let Some(hash) = base.pending_push_hash {
                self.sync.manifest.record_pending_push_base(
                    &nb_id,
                    hash,
                    base.pending_push_page_hashes,
                    base.pending_push_page_elements,
                );
            }
        }
        self.sync_persist_manifest();
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
        self.sync.session_expired = false;
        self.sync.global.server_url = updated.server_url.clone();
        self.sync.global.server_configured = true;
        self.sync.global.remember_server_account(&updated);
        self.sync
            .global
            .upsert_cached_server_session(updated.clone());
        self.sync.active_server_session = Some(updated);
        self.sync_persist_global_state();
        self.save_current_web_bundle();
        self.bump_revision();
    }

    pub fn set_server_url_preference(&mut self, raw_url: &str) -> Result<String, String> {
        let normalized = normalize_server_url(raw_url).map_err(|e| e.to_string())?;
        self.sync.global.server_url = normalized.clone();
        self.sync.global.server_configured = true;
        self.sync_persist_global_state();
        self.capture_active_web_delta_into_pending(true);
        Ok(normalized)
    }

    /// Removes the local shortcut, saved session, and ONLY the local cache on this device
    /// for `(server_url, user_id)` for privacy, keeping all server data intact.
    pub fn forget_server_account(
        &mut self,
        server_url: &str,
        user_id: &str,
    ) -> Result<bool, String> {
        if user_id.trim().is_empty() {
            return Err("server user_id must not be empty".to_string());
        }
        let norm_url = normalize_server_url(server_url).map_err(|e| e.to_string())?;
        let clean_uid = user_id.trim().to_string();
        let was_active = self.sync.active_server_session.as_ref().is_some_and(|s| {
            s.user.user_id == clean_uid && s.server_url.eq_ignore_ascii_case(&norm_url)
        });
        let had_disk_cache = self
            .sync
            .desktop_root
            .as_ref()
            .is_some_and(|root| account_library_dir(root, &clean_uid).exists());
        let web_key = cloud_account_cache_key(&norm_url, &clean_uid);
        let had_web_cache = self.sync.web_cloud_bundles.remove(&web_key).is_some();
        if self.sync.web_cache_enabled {
            let ws_key = cloud_workspace_key(&norm_url, &clean_uid);
            self.sync.web_tracker.workspaces.remove(&ws_key);
            let mut del = WebStorageDelta {
                urgent: true,
                ..Default::default()
            };
            del.delete_workspaces.insert(ws_key);
            self.sync.pending_web_delta.merge(del);
        }

        let removed_shortcut = self
            .sync
            .global
            .forget_server_account(&norm_url, &clean_uid);
        if let Some(ref root) = self.sync.desktop_root {
            let acct_dir = account_library_dir(root, &clean_uid);
            if acct_dir.exists() {
                let _ = fs::remove_dir_all(acct_dir);
            }
        }
        let any_removed = removed_shortcut || had_disk_cache || had_web_cache;

        if was_active {
            self.exit_active_cloud_workspace_to_local();
        } else {
            self.sync_persist_global_state();
            if any_removed {
                self.bump_revision();
            }
        }
        Ok(any_removed)
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

    /// Switches into a saved local profile on this device, flushing active local or Offline-First
    /// cloud changes to disk without deleting any cached cloud account workspace.
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
        self.flush_active_workspace_to_storage();
        self.sync.active_server_session = None;
        self.sync.last_error = None;
        self.sync.last_conflict_notice = None;
        self.sync.sync_in_flight = false;
        self.sync.network_probed_offline = false;
        self.sync.session_expired = false;
        self.sync.in_flight_slot_gens.clear();
        self.sync.last_dirty_at_ms = None;
        self.sync.global.active_workspace = ActiveWorkspaceTarget::Local;

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

    /// Switches into an already cached Offline-First server account without requiring internet
    /// or re-entering a password. Fails if that account has no cached session on this device.
    pub fn switch_to_cached_server_account(
        &mut self,
        server_url: &str,
        user_id: &str,
    ) -> Result<(), String> {
        let norm_url = normalize_server_url(server_url).map_err(|e| e.to_string())?;
        let clean_uid = user_id.trim();
        let Some(cached) = self
            .sync
            .global
            .cached_server_session(&norm_url, clean_uid)
            .cloned()
        else {
            return Err(
                "This server account is not cached on this device. Sign in while online first."
                    .to_string(),
            );
        };

        if !self.is_viewing_cloud() && self.sync.global.needs_initial_nickname {
            self.remove_unconfirmed_local_placeholders();
        } else {
            self.flush_active_workspace_to_storage();
        }

        self.sync.global.server_url = cached.server_url.clone();
        self.sync.global.server_configured = true;
        self.sync.global.remember_server_account(&cached);
        self.sync.global.active_workspace = ActiveWorkspaceTarget::Cloud {
            server_url: cached.server_url.clone(),
            user_id: cached.user.user_id.clone(),
        };
        self.sync_persist_global_state();

        let mut next = self.init_cloud_session_from_storage(&cached);
        next.sync.active_server_session = Some(cached);
        next.sync.connection = SyncConnectionState::Offline;
        next.sync.network_probed_offline = self.sync.network_probed_offline;
        next.sync.sync_in_flight = false;
        next.sync.session_expired = false;
        next.sync.last_error = None;
        next.sync.last_conflict_notice = None;
        *self = next;
        self.save_current_web_bundle();
        self.bump_revision();
        Ok(())
    }

    /// Enters an authenticated server account workspace in Offline-First mode, persisting its
    /// notebooks, folders, preferences, and pending queue in dedicated per-account local storage.
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
        } else {
            self.flush_active_workspace_to_storage();
        }

        self.sync.global.server_url = auth.server_url.clone();
        self.sync.global.server_configured = true;
        self.sync.global.device_name = auth.device_name.clone();
        self.sync.global.remember_server_account(&auth);
        self.sync.global.upsert_cached_server_session(auth.clone());
        self.sync.global.active_workspace = ActiveWorkspaceTarget::Cloud {
            server_url: auth.server_url.clone(),
            user_id: auth.user.user_id.clone(),
        };
        self.sync_persist_global_state();

        let mut next = self.init_cloud_session_from_storage(&auth);
        let was_empty = next.notebooks.is_empty() && next.sync.manifest.last_server_cursor == 0;
        let had_saved_active =
            !sanitize_notebook_id(&next.sync.manifest.active_notebook_id).is_empty();
        let outcome = outcome_from_full_pull(initial_pull, None);
        if next.sync.manifest.notebooks.is_empty()
            && next.sync.manifest.last_server_cursor == 0
            && next.notebooks.is_empty()
        {
            if let Some(ref rp) = outcome.updated_prefs {
                next.prefs = rp.prefs.clone().sanitized();
                next.sync.manifest.prefs_version = rp.version;
                next.sync.manifest.prefs_hash = rp.content_hash.clone();
            } else {
                next.prefs = UserStudioConfig::default();
            }
            next.apply_pen_size_preferences();
        }

        next.sync.active_server_session = Some(auth);
        next.sync.connection = SyncConnectionState::Synced;
        next.sync.network_probed_offline = false;
        next.sync.sync_in_flight = false;
        next.sync.session_expired = false;
        next.sync.last_error = None;
        next.sync.last_conflict_notice = None;

        let _ = next.apply_sync_outcome(outcome);
        if !next.notebooks.is_empty() {
            next.restore_active_notebook_from_manifest(was_empty && !had_saved_active);
        }
        next.history.clear();
        next.flush_active_workspace_to_storage();

        *self = next;
        self.bump_revision();
    }

    fn init_cloud_session_from_storage(&self, auth: &ActiveAccountSession) -> Self {
        let keep_root = self.sync.desktop_root.clone();
        let keep_web_enabled = self.sync.web_cache_enabled;
        let keep_web_locals = self.sync.web_local_bundles.clone();
        let keep_web_clouds = self.sync.web_cloud_bundles.clone();
        let keep_web_tracker = self.sync.web_tracker.clone();
        let keep_web_delta = self.sync.pending_web_delta.clone();
        let keep_global = self.sync.global.clone();
        let uid = auth.user.user_id.clone();
        let owner = OwnerId::CloudUser(uid.clone());

        if let Some(ref root) = keep_root {
            let acct_dir = account_library_dir(root, &uid);
            let (mut next, is_placeholder) =
                Self::open_library_mode(acct_dir, LibraryInitMode::EmptyClean);
            next.sync.desktop_root = Some(root.clone());
            next.sync.web_cache_enabled = keep_web_enabled;
            next.sync.web_local_bundles = keep_web_locals;
            next.sync.web_cloud_bundles = keep_web_clouds;
            next.sync.web_tracker = keep_web_tracker;
            next.sync.pending_web_delta = keep_web_delta;
            next.sync.global = keep_global;
            next.sync.track_manifest = true;
            next.sync_init_from_disk(is_placeholder, owner);
            return next;
        }

        if keep_web_enabled {
            let key = cloud_account_cache_key(&auth.server_url, &uid);
            let mut next = if let Some(bundle) = keep_web_clouds.get(&key) {
                restore_web_bundle(bundle)
            } else {
                let mut empty = Self {
                    notebooks: Vec::new(),
                    active_notebook: 0,
                    trashed_notebooks: Vec::new(),
                    library: LibraryState::default(),
                    ..Default::default()
                };
                let mut blank = Document::default();
                blank.sync_active_to_pages();
                empty.doc = Arc::new(blank);
                empty.sync.slot_ids.clear();
                empty.sync.slot_dirty_hash.clear();
                empty.sync.slot_edit_gen.clear();
                empty.sync.slot_placeholder_blank.clear();
                empty
            };
            next.sync.desktop_root = None;
            next.sync.web_cache_enabled = true;
            next.sync.web_local_bundles = keep_web_locals;
            next.sync.web_cloud_bundles = keep_web_clouds;
            next.sync.web_tracker = keep_web_tracker;
            next.sync.pending_web_delta = keep_web_delta;
            next.sync.global = keep_global;
            next.sync.track_manifest = true;
            next.sync.manifest.owner_id = Some(owner);
            next.sync_reconcile_slots();
            return next;
        }

        let mut next = Self {
            notebooks: Vec::new(),
            active_notebook: 0,
            trashed_notebooks: Vec::new(),
            library: LibraryState::default(),
            ..Default::default()
        };
        let mut blank = Document::default();
        blank.sync_active_to_pages();
        next.doc = Arc::new(blank);
        next.sync.global = keep_global;
        next.sync.track_manifest = true;
        next.sync.manifest.owner_id = Some(owner);
        next.sync.slot_ids.clear();
        next.sync.slot_dirty_hash.clear();
        next.sync.slot_edit_gen.clear();
        next.sync.slot_placeholder_blank.clear();
        next
    }

    /// Signs out from the active server session after flushing local cache changes to disk,
    /// clearing the cached auth token while keeping local accounts selectable.
    /// Returns `true` if one or more saved local accounts exist on this device.
    pub fn sign_out_from_server_session(&mut self) -> bool {
        if let Some(acct) = self.sync.active_server_session.clone() {
            self.flush_active_workspace_to_storage();
            self.sync
                .global
                .remove_cached_server_session(&acct.server_url, &acct.user.user_id);
        }
        self.exit_active_cloud_workspace_to_local()
    }

    fn exit_active_cloud_workspace_to_local(&mut self) -> bool {
        self.sync.active_server_session = None;
        self.sync.sync_in_flight = false;
        self.sync.session_expired = false;
        self.sync.in_flight_slot_gens.clear();
        self.sync.last_error = None;
        self.sync.last_conflict_notice = None;
        self.sync.last_dirty_at_ms = None;
        self.sync.prefs_dirty_pending = false;
        self.sync.connection = SyncConnectionState::SignedOut;
        self.sync.global.active_workspace = ActiveWorkspaceTarget::Local;

        let keep_root = self.sync.desktop_root.clone();
        let keep_web_enabled = self.sync.web_cache_enabled;
        let keep_web_bundles = self.sync.web_local_bundles.clone();
        let keep_web_clouds = self.sync.web_cloud_bundles.clone();
        let keep_web_tracker = std::mem::take(&mut self.sync.web_tracker);
        let keep_web_delta = std::mem::take(&mut self.sync.pending_web_delta);
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
        next.sync.web_cloud_bundles = keep_web_clouds;
        next.sync.web_tracker = keep_web_tracker;
        next.sync.pending_web_delta = keep_web_delta;
        next.sync.global = keep_global;
        next.sync.track_manifest = true;
        next.sync.slot_ids.clear();
        next.sync.slot_dirty_hash.clear();
        next.sync.slot_edit_gen.clear();
        next.sync.in_flight_slot_gens.clear();
        next.sync.slot_placeholder_blank.clear();
        next.sync.connection = SyncConnectionState::SignedOut;

        *self = next;
        self.sync_persist_global_state();
        self.capture_active_web_delta_into_pending(true);
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
            if doc.created_at_ms == 0 {
                doc.created_at_ms = now;
            }
            if doc.updated_at_ms == 0 {
                doc.updated_at_ms = doc.created_at_ms.max(now);
            }
            if doc.meta_updated_at_ms == 0 {
                doc.meta_updated_at_ms = doc.updated_at_ms;
            }
            doc.sync_active_to_pages();
            let id = doc.id.clone();
            local_docs.push(LocalNotebookSnapshot {
                notebook_id: id,
                base_version: 0,
                cloud_dirty: true,
                // Computed asynchronously on the sync worker thread to keep UI responsiveness intact.
                content_hash: String::new(),
                doc,
                merge_base: DocumentMergeBase::default(),
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

    /// Finalizes local-to-cloud conversion after the initial server upload succeeds: migrates the
    /// local notebooks, folders, trash, and preferences into the new cloud account's persistent
    /// Offline-First local cache (keeping all data on this device for offline access) and enters
    /// the converted server account session.
    pub fn complete_local_account_conversion(
        &mut self,
        source_profile_id: &str,
        mut auth: ActiveAccountSession,
        initial_pull: SyncPullResponse,
    ) {
        auth.server_url =
            normalize_server_url(&auth.server_url).unwrap_or_else(|_| auth.server_url.clone());
        let is_active_local = !self.is_viewing_cloud()
            && self
                .active_local_profile()
                .is_some_and(|p| p.local_profile_id == source_profile_id);
        if is_active_local {
            self.flush_active_workspace_to_storage();
        }

        if let Some(ref root) = self.sync.desktop_root {
            let src_dir = profile_library_dir(root, source_profile_id);
            let dst_dir = account_library_dir(root, &auth.user.user_id);
            if src_dir.exists() {
                migrate_local_profile_dir_to_cloud_account(&src_dir, &dst_dir);
            }
        }
        if self.sync.web_cache_enabled
            && let Some(mut bundle) = self.sync.web_local_bundles.remove(source_profile_id)
        {
            bundle
                .manifest
                .reassign_owner(OwnerId::CloudUser(auth.user.user_id.clone()), false);
            let key = cloud_account_cache_key(&auth.server_url, &auth.user.user_id);
            self.sync.web_cloud_bundles.insert(key, bundle);
        }
        if self.sync.web_cache_enabled {
            let old_ws_key = local_workspace_key(source_profile_id);
            self.sync.web_tracker.workspaces.remove(&old_ws_key);
            let mut del = WebStorageDelta {
                urgent: true,
                ..Default::default()
            };
            del.delete_workspaces.insert(old_ws_key);
            self.sync.pending_web_delta.merge(del);
        }

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
        if self.sync.global.local_profiles.is_empty() {
            self.sync.global.active_local_profile_id.clear();
            self.sync.global.needs_initial_nickname = true;
        }
        if is_active_local {
            self.sync.manifest.owner_id = None;
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

    /// Flushes active workspace documents, preferences, and sync manifests for clean app shutdown.
    pub fn flush_for_exit(&mut self) {
        self.flush_active_workspace_to_storage();
    }

    pub(crate) fn sync_flush_dirty_flags_to_manifest(&mut self) {
        self.sync_active_notebook_slot();
        if !self.sync.track_manifest {
            return;
        }
        self.sync_reconcile_slots();
        let count = self.notebooks.len();
        self.ensure_sync_vec_lens(count);
        for idx in 0..count {
            if self.sync.slot_placeholder_blank[idx] {
                continue;
            }
            if self.sync.slot_dirty_hash[idx]
                && let Some(id) = self
                    .sync
                    .slot_ids
                    .get(idx)
                    .filter(|id| !id.is_empty())
                    .cloned()
            {
                self.sync.manifest.mark_notebook_dirty(&id, "");
            }
        }
        if self.sync.prefs_dirty_pending {
            let hash = canonical_prefs_hash(&self.prefs);
            self.sync.manifest.mark_prefs_dirty(&hash);
            self.sync.prefs_dirty_pending = false;
        }
    }

    fn flush_active_workspace_to_storage(&mut self) {
        if self.has_loaded_workspace() {
            self.library.promote_view_dirty_to_dirty();
            self.sync_flush_dirty_flags_to_manifest();
            self.sync_reconcile_folders();
            let _ = self.save_library();
            let _ = self.save_prefs();
            self.sync_persist_manifest();
            self.save_current_web_bundle();
        }
        self.capture_active_web_delta_into_pending(true);
    }

    fn load_local_profile_workspace(&mut self, profile_id: &str) {
        let owner = OwnerId::LocalProfile(profile_id.to_string());
        if let Some(root) = self.sync.desktop_root.clone() {
            let keep_global = self.sync.global.clone();
            let keep_web_enabled = self.sync.web_cache_enabled;
            let keep_web_locals = self.sync.web_local_bundles.clone();
            let keep_web_clouds = self.sync.web_cloud_bundles.clone();
            let keep_web_tracker = std::mem::take(&mut self.sync.web_tracker);
            let keep_web_delta = std::mem::take(&mut self.sync.pending_web_delta);
            let prof_dir = profile_library_dir(&root, profile_id);
            let (next_session, is_placeholder) =
                Self::open_library_mode(prof_dir, LibraryInitMode::EmptyClean);
            *self = next_session;
            self.sync.desktop_root = Some(root);
            self.sync.web_cache_enabled = keep_web_enabled;
            self.sync.web_local_bundles = keep_web_locals;
            self.sync.web_cloud_bundles = keep_web_clouds;
            self.sync.web_tracker = keep_web_tracker;
            self.sync.pending_web_delta = keep_web_delta;
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
            next.sync.web_cloud_bundles = self.sync.web_cloud_bundles.clone();
            next.sync.web_tracker = std::mem::take(&mut self.sync.web_tracker);
            next.sync.pending_web_delta = std::mem::take(&mut self.sync.pending_web_delta);
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
                Self::open_library_mode(src_dir, LibraryInitMode::EmptyClean);
            if !is_placeholder {
                src_sess.sync.track_manifest = true;
                src_sess.sync_reconcile_slots();
                let _ = src_sess.save_library();
                docs = src_sess.notebooks.iter().map(|d| (**d).clone()).collect();
            }
            prefs = src_sess.prefs.clone();
        } else if self.sync.web_cache_enabled
            && let Some(bundle) = self.sync.web_local_bundles.get_mut(source_profile_id)
        {
            for (idx, doc) in bundle.notebooks.iter_mut().enumerate() {
                if bundle
                    .slot_placeholder_blank
                    .get(idx)
                    .copied()
                    .unwrap_or(false)
                {
                    continue;
                }
                if doc.id.is_empty() {
                    doc.id = Uuid::new_v4().to_string();
                }
                if let Some(slot_id) = bundle.slot_ids.get_mut(idx)
                    && slot_id.is_empty()
                {
                    *slot_id = doc.id.clone();
                }
                docs.push(doc.clone());
            }
            prefs = bundle.prefs.clone();
        }

        CapturedProfileData { docs, prefs }
    }

    fn remove_local_profile_disk_and_web_data(&mut self, profile_id: &str) {
        self.sync.web_local_bundles.remove(profile_id);
        if self.sync.web_cache_enabled {
            let ws_key = local_workspace_key(profile_id);
            self.sync.web_tracker.workspaces.remove(&ws_key);
            let mut del = WebStorageDelta {
                urgent: true,
                ..Default::default()
            };
            del.delete_workspaces.insert(ws_key);
            self.sync.pending_web_delta.merge(del);
        }
        if let Some(ref root) = self.sync.desktop_root {
            let dir = profile_library_dir(root, profile_id);
            if dir.exists() {
                let _ = fs::remove_dir_all(dir);
            }
        }
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
        self.sync.in_flight_slot_gens.clear();

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
            if slot_dirty || self.sync.manifest.meta(&id).is_none() {
                self.sync.manifest.mark_notebook_dirty(&id, "");
                self.sync.slot_dirty_hash[idx] = false;
            }

            if let Some(meta) = self.sync.manifest.meta(&id)
                && (meta.cloud_dirty || meta.server_version == 0)
            {
                let edit_gen = self.sync.slot_edit_gen[idx];
                self.sync.in_flight_slot_gens.insert(id.clone(), edit_gen);
                let merge_base = DocumentMergeBase {
                    page_hashes: meta.last_synced_page_hashes.clone(),
                    page_elements: meta.last_synced_page_element_hashes.clone(),
                    pending_push_hash: meta.pending_push_hash.clone(),
                    pending_push_page_hashes: meta.pending_push_page_hashes.clone(),
                    pending_push_page_elements: meta.pending_push_page_elements.clone(),
                };
                local_docs.push(LocalNotebookSnapshot {
                    notebook_id: id,
                    base_version: meta.server_version,
                    cloud_dirty: true,
                    // Defer expensive SHA-256 hashing of strokes and embedded assets to `papercraft-sync`.
                    content_hash: String::new(),
                    doc: (**doc_arc).clone(),
                    merge_base,
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
            force_overwrite: false,
        })
    }

    /// Applies a completed server sync outcome onto the Offline-First cloud session and
    /// persists updates to disk / browser cache without ever discarding user work.
    pub fn apply_sync_outcome(&mut self, outcome: SyncCycleOutcome) -> Result<(), String> {
        if self.live_stroke.is_some() || self.live_eraser.is_some() {
            return Err("cannot apply server update during an active stroke".to_string());
        }
        if !self.is_viewing_cloud() {
            return Err("cannot apply server sync outcome while using a local profile".to_string());
        }
        let Some(active) = self.sync.active_server_session.as_ref() else {
            return Err("no active server session".to_string());
        };
        if active.user.user_id != outcome.server_user.user_id {
            return Err("ignored stale sync outcome for a different server account".to_string());
        }
        if let Some(ref refreshed) = outcome.refreshed_session {
            let norm_refreshed = normalize_server_url(&refreshed.server_url)
                .unwrap_or_else(|_| refreshed.server_url.clone());
            let norm_active = normalize_server_url(&active.server_url)
                .unwrap_or_else(|_| active.server_url.clone());
            if refreshed.user.user_id != active.user.user_id
                || !norm_refreshed.eq_ignore_ascii_case(&norm_active)
            {
                return Err(
                    "ignored stale refreshed session for a different server account".to_string(),
                );
            }
        }
        self.sync_active_notebook_slot();

        if let Some(refreshed) = outcome.refreshed_session {
            self.sync.global.server_url = refreshed.server_url.clone();
            self.sync.global.server_configured = true;
            self.sync.global.remember_server_account(&refreshed);
            self.sync
                .global
                .upsert_cached_server_session(refreshed.clone());
            self.sync.active_server_session = Some(refreshed);
            self.sync_persist_global_state();
        } else if let Some(sess) = self.sync.active_server_session.as_mut()
            && sess.user != outcome.server_user
        {
            sess.user = outcome.server_user.clone();
            let sess_clone = sess.clone();
            self.sync.global.remember_server_account(&sess_clone);
            self.sync.global.upsert_cached_server_session(sess_clone);
            self.sync_persist_global_state();
        }

        self.sync.manifest.last_server_cursor = outcome.new_cursor;
        let rebased_changed = self.ack_accepted_uploads(
            &outcome.accepted_uploads,
            &outcome.accepted_page_hashes,
            &outcome.accepted_page_elements,
            &outcome.rebased_docs,
            &outcome.rebased_ancestors,
            outcome.synced_at_ms,
        );
        self.ack_accepted_folders(&outcome.accepted_folders);

        for deleted_id in &outcome.cleared_tombstones {
            self.sync.manifest.clear_tombstone(deleted_id);
        }

        let was_empty_before_pull = self.notebooks.is_empty();
        let had_saved_active =
            !sanitize_notebook_id(&self.sync.manifest.active_notebook_id).is_empty();
        let mut updated_any_doc = rebased_changed;
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
        for conflict in &outcome.conflict_updates {
            if self.resolve_notebook_conflict(conflict, outcome.synced_at_ms) {
                updated_any_doc = true;
            }
        }
        if self.apply_remote_folders(&outcome.downloaded_folders) {
            updated_any_doc = true;
        }
        if !self.notebooks.is_empty() {
            if was_empty_before_pull && !had_saved_active {
                self.restore_active_notebook_from_manifest(true);
            } else {
                self.sync_active_notebook_id();
            }
        }

        if let Some((ver, ref sent_hash)) = outcome.ack_prefs {
            let cur_hash = canonical_prefs_hash(&self.prefs);
            self.sync.manifest.prefs_version = ver;
            self.sync.manifest.prefs_hash = sent_hash.clone();
            self.sync.manifest.prefs_dirty =
                self.sync.prefs_dirty_pending || cur_hash != *sent_hash;
        }
        if let Some(ref rp) = outcome.updated_prefs {
            if !self.sync.manifest.prefs_dirty && !self.sync.prefs_dirty_pending {
                self.apply_remote_prefs_preserving_folders(&rp.prefs);
                self.sync.manifest.prefs_version = rp.version;
                self.sync.manifest.prefs_hash = rp.content_hash.clone();
                self.sync.manifest.prefs_dirty = false;
                updated_any_doc = true;
            } else if outcome.ack_prefs.is_none() && rp.version > self.sync.manifest.prefs_version {
                if !self.sync.prefs_dirty_pending
                    && self.sync.manifest.prefs_version == 0
                    && self.prefs == UserStudioConfig::default()
                {
                    self.apply_remote_prefs_preserving_folders(&rp.prefs);
                    self.sync.manifest.prefs_version = rp.version;
                    self.sync.manifest.prefs_hash = rp.content_hash.clone();
                    self.sync.manifest.prefs_dirty = false;
                } else {
                    merge_studio_prefs(&mut self.prefs, rp.prefs.clone(), false);
                    self.apply_pen_size_preferences();
                    let cur_hash = canonical_prefs_hash(&self.prefs);
                    self.sync.manifest.prefs_version = rp.version;
                    self.sync.manifest.prefs_hash = cur_hash.clone();
                    self.sync.manifest.prefs_dirty =
                        self.sync.prefs_dirty_pending || cur_hash != rp.content_hash;
                }
                updated_any_doc = true;
            }
        }

        self.sync.manifest.last_sync_at_ms = Some(outcome.synced_at_ms);
        self.sync.network_probed_offline = false;
        self.sync.sync_in_flight = false;
        self.sync.session_expired = false;
        self.sync.connection = SyncConnectionState::Synced;
        self.sync.last_error = None;
        if self.compute_pending_changes_fast() == 0 {
            self.sync.last_dirty_at_ms = None;
        } else {
            self.sync.last_dirty_at_ms.get_or_insert_with(now_unix_ms);
        }

        self.flush_active_workspace_to_storage();
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
            self.sync.slot_edit_gen[index] = self.sync.slot_edit_gen[index].wrapping_add(1);
            self.sync.slot_placeholder_blank[index] = false;
            // Adaptive trailing debounce: defer push while strokes/edits continue interactively.
            self.sync.last_dirty_at_ms = Some(now_unix_ms());
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
        self.sync.last_dirty_at_ms = Some(now_unix_ms());
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
                self.sync.in_flight_slot_gens.remove(&id);
                self.sync.manifest.record_local_delete(&id, now_unix_ms());
                self.sync.last_dirty_at_ms = Some(now_unix_ms());
            }
        }
        if index < self.sync.slot_dirty_hash.len() {
            self.sync.slot_dirty_hash.remove(index);
        }
        if index < self.sync.slot_edit_gen.len() {
            self.sync.slot_edit_gen.remove(index);
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
            self.sync.slot_edit_gen[index] = self.sync.slot_edit_gen[index].wrapping_add(1);
            self.sync.slot_placeholder_blank[index] = false;
        }
        if index < self.notebooks.len() {
            self.sync.manifest.mark_notebook_dirty(&id, "");
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
        self.sync.last_dirty_at_ms = Some(now_unix_ms());
        self.sync_persist_manifest();
    }

    pub(crate) fn sync_active_notebook_id(&mut self) {
        if !self.sync.track_manifest {
            return;
        }
        let next_id = self
            .notebooks
            .get(self.active_notebook)
            .map(|d| d.id.as_str())
            .unwrap_or_default();
        if next_id.is_empty() {
            self.sync.manifest.active_notebook_id.clear();
        } else if self.sync.manifest.active_notebook_id != next_id {
            self.sync.manifest.active_notebook_id = next_id.to_string();
        }
    }

    /// Selects the active notebook from `manifest.active_notebook_id` if present,
    /// optionally falling back to the most recently modified notebook.
    pub(crate) fn restore_active_notebook_from_manifest(&mut self, fallback_to_most_recent: bool) {
        if self.notebooks.is_empty() {
            self.active_notebook = 0;
            return;
        }
        let saved_id = sanitize_notebook_id(&self.sync.manifest.active_notebook_id);
        let target_idx = if !saved_id.is_empty() {
            self.notebooks.iter().position(|d| d.id == saved_id)
        } else {
            None
        };
        let chosen = target_idx.unwrap_or_else(|| {
            if fallback_to_most_recent {
                most_recent_notebook_index(&self.notebooks)
            } else {
                self.active_notebook
                    .min(self.notebooks.len().saturating_sub(1))
            }
        });
        if chosen != self.active_notebook || self.doc.id != self.notebooks[chosen].id {
            self.active_notebook = chosen;
            self.doc = Arc::clone(&self.notebooks[chosen]);
            self.history.clear();
        }
        self.sync_active_notebook_id();
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
                    self.sync.slot_edit_gen[idx] = self.sync.slot_edit_gen[idx].wrapping_add(1);
                    self.sync.manifest.mark_notebook_dirty(&id, "");
                    self.sync.last_dirty_at_ms = Some(now_unix_ms());
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
        self.sync.slot_edit_gen = vec![0; count];
        self.sync.in_flight_slot_gens.clear();
        self.sync.slot_placeholder_blank = vec![is_placeholder && count == 1; count];
        self.sync.prefs_dirty_pending = false;
        self.sync.last_dirty_at_ms = None;

        if !is_placeholder {
            self.sync_reconcile_slots();
            self.restore_active_notebook_from_manifest(false);
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

    pub(crate) fn sync_reconcile_folders(&mut self) {
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
        self.sync.slot_edit_gen.resize(count, 0);
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
        if !self.sync.track_manifest {
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
        accepted_page_hashes: &HashMap<String, Vec<String>>,
        accepted_page_elements: &HashMap<String, Vec<PageElementFingerprints>>,
        rebased_docs: &HashMap<String, Document>,
        rebased_ancestors: &HashMap<String, DocumentMergeBase>,
        synced_at_ms: i64,
    ) -> bool {
        let mut updated_any = false;
        for acc in accepted {
            let sent_gen = self.sync.in_flight_slot_gens.remove(&acc.notebook_id);
            let slot_idx = self.find_slot_by_id(&acc.notebook_id);
            let mut still_dirty = match (sent_gen, slot_idx) {
                (Some(expected_gen), Some(idx)) => {
                    self.sync.slot_dirty_hash.get(idx).copied().unwrap_or(false)
                        || self.sync.slot_edit_gen.get(idx).copied() != Some(expected_gen)
                }
                (Some(_), None) => false,
                (None, Some(idx)) => {
                    canonical_document_hash(&self.notebooks[idx]) != acc.content_hash
                }
                (None, None) => false,
            };

            if let Some(rebased_doc) = rebased_docs.get(&acc.notebook_id)
                && let Some(idx) = slot_idx
            {
                let prev_hash = canonical_document_hash(&self.notebooks[idx]);
                if !still_dirty {
                    let mut installed = rebased_doc.clone();
                    installed.current_page = self.notebooks[idx]
                        .current_page
                        .min(installed.page_count().saturating_sub(1));
                    installed.sync_active_to_pages();
                    installed.load_active_from_pages();
                    self.install_remote_doc_at_slot(idx, installed, prev_hash != acc.content_hash);
                    updated_any = true;
                } else {
                    let ancestor = rebased_ancestors
                        .get(&acc.notebook_id)
                        .cloned()
                        .unwrap_or_default();
                    match merge_documents_three_way(&self.notebooks[idx], rebased_doc, &ancestor) {
                        MergeDecision::FastForward(ff) => {
                            let changed = canonical_document_hash(&ff) != prev_hash;
                            self.install_remote_doc_at_slot(idx, ff, changed);
                            still_dirty = false;
                            updated_any = true;
                        }
                        MergeDecision::Merged { doc: merged, .. } => {
                            let changed = canonical_document_hash(&merged) != prev_hash;
                            self.install_remote_doc_at_slot(idx, merged, changed);
                            still_dirty = true;
                            updated_any = true;
                        }
                        MergeDecision::Conflict => {
                            let local_work = (*self.notebooks[idx]).clone();
                            self.install_remote_doc_at_slot(
                                idx,
                                rebased_doc.clone(),
                                prev_hash != acc.content_hash,
                            );
                            let backup_title = self.append_conflict_copy_notebook(local_work);
                            self.sync.last_conflict_notice = Some(format!(
                                "Saved \"{backup_title}\" to keep offline changes made during sync."
                            ));
                            still_dirty = false;
                            updated_any = true;
                        }
                    }
                }
            }

            self.sync.manifest.mark_notebook_synced_with_dirty(
                &acc.notebook_id,
                acc.new_version,
                acc.content_hash.clone(),
                still_dirty,
                synced_at_ms,
            );
            self.sync.manifest.clear_pending_push_base(&acc.notebook_id);
            if let Some(hashes) = accepted_page_hashes.get(&acc.notebook_id) {
                self.sync
                    .manifest
                    .set_notebook_synced_page_hashes(&acc.notebook_id, hashes.clone());
            }
            if let Some(elements) = accepted_page_elements.get(&acc.notebook_id) {
                self.sync
                    .manifest
                    .set_notebook_synced_page_elements(&acc.notebook_id, elements.clone());
            } else if let Some(rebased_doc) = rebased_docs.get(&acc.notebook_id) {
                self.sync.manifest.set_notebook_synced_page_elements(
                    &acc.notebook_id,
                    compute_document_page_elements(rebased_doc),
                );
            }
            if let Some(idx) = slot_idx {
                self.sync.slot_dirty_hash[idx] = still_dirty;
            }
        }
        updated_any
    }

    pub(crate) fn install_remote_doc_at_slot(
        &mut self,
        idx: usize,
        mut doc: Document,
        content_changed: bool,
    ) {
        doc.sync_active_to_pages();
        doc.load_active_from_pages();
        self.notebooks[idx] = Arc::new(doc);
        self.library.mark_dirty(idx, self.notebooks.len());
        self.sync.slot_edit_gen[idx] = self.sync.slot_edit_gen[idx].wrapping_add(1);
        self.sync.slot_placeholder_blank[idx] = false;
        if idx == self.active_notebook {
            self.doc = Arc::clone(&self.notebooks[idx]);
            if content_changed {
                self.history.clear();
            }
        }
    }

    fn resolve_notebook_conflict(
        &mut self,
        server_update: &RemoteNotebookUpdate,
        synced_at_ms: i64,
    ) -> bool {
        if server_update.deleted {
            let Some(idx) = self.find_slot_by_id(&server_update.notebook_id) else {
                return self.apply_remote_delete(&server_update.notebook_id);
            };
            let local_changed = self.sync.slot_dirty_hash.get(idx).copied().unwrap_or(false)
                || self
                    .sync
                    .manifest
                    .meta(&server_update.notebook_id)
                    .is_some_and(|m| m.cloud_dirty);
            if !local_changed {
                return self.apply_remote_delete(&server_update.notebook_id);
            }
            // Server deleted a notebook while offline edits existed on this device: fork as a new ID
            // so the user's offline work is preserved and synced safely.
            let mut preserved = (*self.notebooks[idx]).clone();
            let next_id = Uuid::new_v4().to_string();
            let now = now_unix_ms();
            preserved.id = next_id.clone();
            preserved.updated_at_ms = now;
            preserved.meta_updated_at_ms = now;
            self.notebooks[idx] = Arc::new(preserved);
            self.library.mark_dirty(idx, self.notebooks.len());
            self.sync
                .manifest
                .notebooks
                .retain(|m| m.notebook_id != server_update.notebook_id);
            self.sync.slot_ids[idx] = next_id.clone();
            self.sync.slot_dirty_hash[idx] = true;
            self.sync.slot_edit_gen[idx] = self.sync.slot_edit_gen[idx].wrapping_add(1);
            self.sync.manifest.mark_notebook_dirty(&next_id, "");
            self.sync.last_dirty_at_ms = Some(now);
            if idx == self.active_notebook {
                self.doc = Arc::clone(&self.notebooks[idx]);
            }
            self.sync.last_conflict_notice = Some(format!(
                "Preserved local offline edits to \"{}\" after remote deletion.",
                self.notebooks[idx].title
            ));
            return true;
        }
        self.apply_remote_upsert(server_update, synced_at_ms)
    }

    /// Reconciles document timestamps when local and remote hashes already match.
    fn reconcile_matching_hash_upsert(
        &mut self,
        idx: usize,
        local_changed: bool,
        doc: &Document,
        update: &RemoteNotebookUpdate,
        (remote_page_hashes, remote_page_elements): (Vec<String>, Vec<PageElementFingerprints>),
        synced_at_ms: i64,
    ) -> bool {
        let mut timestamp_changed = false;
        let mut reconciled = (*self.notebooks[idx]).clone();
        if local_changed {
            if reconciled.created_at_ms == 0 && doc.created_at_ms > 0 {
                reconciled.created_at_ms = doc.created_at_ms;
                timestamp_changed = true;
            }
            if doc.updated_at_ms > reconciled.updated_at_ms {
                reconciled.updated_at_ms = doc.updated_at_ms;
                timestamp_changed = true;
            }
            if doc.meta_updated_at_ms > reconciled.meta_updated_at_ms {
                reconciled.meta_updated_at_ms = doc.meta_updated_at_ms;
                timestamp_changed = true;
            }
        } else {
            if doc.created_at_ms > 0 && reconciled.created_at_ms != doc.created_at_ms {
                reconciled.created_at_ms = doc.created_at_ms;
                timestamp_changed = true;
            }
            if doc.updated_at_ms > 0 && reconciled.updated_at_ms != doc.updated_at_ms {
                reconciled.updated_at_ms = doc.updated_at_ms;
                timestamp_changed = true;
            }
            if doc.meta_updated_at_ms > 0 && reconciled.meta_updated_at_ms != doc.meta_updated_at_ms
            {
                reconciled.meta_updated_at_ms = doc.meta_updated_at_ms;
                timestamp_changed = true;
            }
        }
        let local_ahead_of_remote = local_changed
            && (reconciled.updated_at_ms > doc.updated_at_ms
                || reconciled.meta_updated_at_ms > doc.meta_updated_at_ms
                || (doc.created_at_ms == 0 && reconciled.created_at_ms > 0));
        if timestamp_changed {
            reconciled.sync_active_to_pages();
            self.notebooks[idx] = Arc::new(reconciled);
            self.library.mark_dirty(idx, self.notebooks.len());
            if idx == self.active_notebook {
                self.doc = Arc::clone(&self.notebooks[idx]);
            }
        }
        self.sync.slot_dirty_hash[idx] = local_ahead_of_remote;
        self.sync.slot_placeholder_blank[idx] = false;
        self.sync.manifest.mark_notebook_synced_with_dirty(
            &update.notebook_id,
            update.version,
            update.content_hash.clone(),
            local_ahead_of_remote,
            synced_at_ms,
        );
        self.sync
            .manifest
            .clear_pending_push_base(&update.notebook_id);
        self.sync
            .manifest
            .set_notebook_synced_page_hashes(&update.notebook_id, remote_page_hashes);
        self.sync
            .manifest
            .set_notebook_synced_page_elements(&update.notebook_id, remote_page_elements);
        if local_ahead_of_remote {
            self.sync.last_dirty_at_ms = Some(now_unix_ms());
        }
        timestamp_changed
    }

    fn apply_remote_upsert(&mut self, update: &RemoteNotebookUpdate, synced_at_ms: i64) -> bool {
        let Some(mut doc) = update.document.clone() else {
            return false;
        };
        doc.id = update.notebook_id.clone();
        if doc.updated_at_ms == 0 && update.updated_at_ms > 0 {
            doc.updated_at_ms = update.updated_at_ms;
        }
        if doc.meta_updated_at_ms == 0 && update.meta_updated_at_ms > 0 {
            doc.meta_updated_at_ms = update.meta_updated_at_ms;
        }
        if doc.created_at_ms == 0 && doc.updated_at_ms > 0 {
            doc.created_at_ms = doc.updated_at_ms;
        }
        doc.sync_active_to_pages();
        doc.load_active_from_pages();
        let remote_page_hashes = if update.page_hashes.is_empty() {
            canonical_document_page_hashes(&doc)
        } else {
            update.page_hashes.clone()
        };
        let remote_page_elements = compute_document_page_elements(&doc);

        if let Some(idx) = self.find_slot_by_id(&update.notebook_id) {
            let cur_hash = canonical_document_hash(&self.notebooks[idx]);
            let local_changed = self.sync.slot_dirty_hash[idx]
                || self
                    .sync
                    .manifest
                    .meta(&update.notebook_id)
                    .is_some_and(|m| m.cloud_dirty && cur_hash != m.last_synced_hash);

            if local_changed && cur_hash != update.content_hash {
                let server_unchanged =
                    self.sync
                        .manifest
                        .meta(&update.notebook_id)
                        .is_some_and(|m| {
                            m.server_version > 0
                                && m.server_version == update.version
                                && m.last_synced_hash == update.content_hash
                        });
                if server_unchanged {
                    self.sync
                        .manifest
                        .mark_notebook_dirty(&update.notebook_id, "");
                    self.sync.slot_dirty_hash[idx] = true;
                    self.sync.last_dirty_at_ms = Some(now_unix_ms());
                    return false;
                }
                let base = self
                    .sync
                    .manifest
                    .meta(&update.notebook_id)
                    .map(|m| DocumentMergeBase {
                        page_hashes: m.last_synced_page_hashes.clone(),
                        page_elements: m.last_synced_page_element_hashes.clone(),
                        pending_push_hash: m.pending_push_hash.clone(),
                        pending_push_page_hashes: m.pending_push_page_hashes.clone(),
                        pending_push_page_elements: m.pending_push_page_elements.clone(),
                    })
                    .unwrap_or_default();
                let remote_orig_updated = doc.updated_at_ms;
                let remote_orig_meta = doc.meta_updated_at_ms;
                let remote_orig_created = doc.created_at_ms;
                match merge_documents_three_way(&self.notebooks[idx], &doc, &base) {
                    MergeDecision::FastForward(_) => {
                        return self.reconcile_matching_hash_upsert(
                            idx,
                            local_changed,
                            &doc,
                            update,
                            (remote_page_hashes, remote_page_elements),
                            synced_at_ms,
                        );
                    }
                    MergeDecision::Merged {
                        doc: mut merged,
                        kind,
                    } => {
                        merged.id = update.notebook_id.clone();
                        merged.sync_active_to_pages();
                        merged.load_active_from_pages();
                        let merged_hash = canonical_document_hash(&merged);
                        let content_changed = merged_hash != cur_hash;
                        let dirty = match kind {
                            MergeKind::MetadataOnly => {
                                merged_hash != update.content_hash
                                    || merged.updated_at_ms > remote_orig_updated
                                    || merged.meta_updated_at_ms > remote_orig_meta
                                    || (remote_orig_created == 0 && merged.created_at_ms > 0)
                            }
                            MergeKind::RebasedPendingPush
                            | MergeKind::PageLevel
                            | MergeKind::ElementLevel => true,
                        };
                        let merged_title = merged.title.clone();
                        self.install_remote_doc_at_slot(idx, merged, content_changed);
                        self.sync.slot_dirty_hash[idx] = dirty;
                        self.sync.manifest.mark_notebook_synced_with_dirty(
                            &update.notebook_id,
                            update.version,
                            update.content_hash.clone(),
                            dirty,
                            synced_at_ms,
                        );
                        self.sync
                            .manifest
                            .clear_pending_push_base(&update.notebook_id);
                        self.sync.manifest.set_notebook_synced_page_hashes(
                            &update.notebook_id,
                            remote_page_hashes,
                        );
                        self.sync.manifest.set_notebook_synced_page_elements(
                            &update.notebook_id,
                            remote_page_elements,
                        );
                        if dirty {
                            self.sync.last_dirty_at_ms = Some(now_unix_ms());
                        }
                        if matches!(kind, MergeKind::PageLevel | MergeKind::ElementLevel) {
                            self.sync.last_conflict_notice =
                                Some(format!("Merged offline changes in \"{merged_title}\"."));
                        }
                        return true;
                    }
                    MergeDecision::Conflict => {
                        // Irreconcilable same-element or structural collision: apply server record
                        // AND keep all local work in a single non-cascading Conflict Copy.
                        let local_work = (*self.notebooks[idx]).clone();
                        self.install_remote_doc_at_slot(idx, doc, true);
                        self.sync.slot_dirty_hash[idx] = false;
                        self.sync.manifest.mark_notebook_synced_with_dirty(
                            &update.notebook_id,
                            update.version,
                            update.content_hash.clone(),
                            false,
                            synced_at_ms,
                        );
                        self.sync
                            .manifest
                            .clear_pending_push_base(&update.notebook_id);
                        self.sync.manifest.set_notebook_synced_page_hashes(
                            &update.notebook_id,
                            remote_page_hashes,
                        );
                        self.sync.manifest.set_notebook_synced_page_elements(
                            &update.notebook_id,
                            remote_page_elements,
                        );
                        let backup_title = self.append_conflict_copy_notebook(local_work);
                        self.sync.last_conflict_notice = Some(format!(
                            "Saved \"{backup_title}\" to keep offline changes made on the same element."
                        ));
                        return true;
                    }
                }
            }

            if cur_hash == update.content_hash {
                return self.reconcile_matching_hash_upsert(
                    idx,
                    local_changed,
                    &doc,
                    update,
                    (remote_page_hashes, remote_page_elements),
                    synced_at_ms,
                );
            }

            self.install_remote_doc_at_slot(idx, doc, cur_hash != update.content_hash);
            self.sync.slot_dirty_hash[idx] = false;
            self.sync.manifest.mark_notebook_synced_with_dirty(
                &update.notebook_id,
                update.version,
                update.content_hash.clone(),
                false,
                synced_at_ms,
            );
            self.sync
                .manifest
                .clear_pending_push_base(&update.notebook_id);
            self.sync
                .manifest
                .set_notebook_synced_page_hashes(&update.notebook_id, remote_page_hashes);
            self.sync
                .manifest
                .set_notebook_synced_page_elements(&update.notebook_id, remote_page_elements);
            return true;
        }

        if let Some(p_idx) = self.sync.slot_placeholder_blank.iter().position(|&b| b) {
            self.sync.slot_ids[p_idx] = update.notebook_id.clone();
            self.sync.slot_dirty_hash[p_idx] = false;
            self.install_remote_doc_at_slot(p_idx, doc, true);
            self.sync.manifest.mark_notebook_synced_with_dirty(
                &update.notebook_id,
                update.version,
                update.content_hash.clone(),
                false,
                synced_at_ms,
            );
            self.sync
                .manifest
                .clear_pending_push_base(&update.notebook_id);
            self.sync
                .manifest
                .set_notebook_synced_page_hashes(&update.notebook_id, remote_page_hashes);
            self.sync
                .manifest
                .set_notebook_synced_page_elements(&update.notebook_id, remote_page_elements);
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
        self.library.push_slot(None, true, self.notebooks.len());
        self.sync.slot_ids.push(update.notebook_id.clone());
        self.sync.slot_dirty_hash.push(false);
        self.sync.slot_edit_gen.push(0);
        self.sync.slot_placeholder_blank.push(false);
        if self.notebooks.len() == 1 {
            self.active_notebook = 0;
            self.doc = Arc::clone(&self.notebooks[0]);
            self.history.clear();
        }
        self.sync.manifest.mark_notebook_synced_with_dirty(
            &update.notebook_id,
            update.version,
            update.content_hash.clone(),
            false,
            synced_at_ms,
        );
        self.sync
            .manifest
            .clear_pending_push_base(&update.notebook_id);
        self.sync
            .manifest
            .set_notebook_synced_page_hashes(&update.notebook_id, remote_page_hashes);
        self.sync
            .manifest
            .set_notebook_synced_page_elements(&update.notebook_id, remote_page_elements);
        true
    }

    pub(crate) fn append_conflict_copy_notebook(&mut self, mut copy_doc: Document) -> String {
        let now = now_unix_ms();
        copy_doc.sync_active_to_pages();
        copy_doc.load_active_from_pages();
        let copy_body_hash = canonical_document_body_hash(&copy_doc);
        let stripped_base = strip_conflict_copy_suffix(&copy_doc.title);
        if let Some(existing) = self.notebooks.iter().find(|nb| {
            strip_conflict_copy_suffix(&nb.title).eq_ignore_ascii_case(&stripped_base)
                && canonical_document_body_hash(nb) == copy_body_hash
        }) {
            return existing.title.clone();
        }

        let copy_id = Uuid::new_v4().to_string();
        let copy_title = format_conflict_copy_title(&copy_doc.title, &self.notebooks);
        copy_doc.id = copy_id.clone();
        copy_doc.title = copy_title.clone();
        copy_doc.updated_at_ms = now;
        copy_doc.meta_updated_at_ms = now;
        copy_doc.sync_active_to_pages();
        copy_doc.load_active_from_pages();

        if self.notebooks.len() >= MAX_NOTEBOOKS {
            let slot_idx = self.notebooks.len().saturating_sub(1);
            self.notebooks[slot_idx] = Arc::new(copy_doc);
            self.library.mark_dirty(slot_idx, self.notebooks.len());
            self.ensure_sync_vec_lens(self.notebooks.len());
            self.sync.slot_ids[slot_idx] = copy_id.clone();
            self.sync.slot_dirty_hash[slot_idx] = true;
            self.sync.slot_edit_gen[slot_idx] = self.sync.slot_edit_gen[slot_idx].wrapping_add(1);
            self.sync.slot_placeholder_blank[slot_idx] = false;
        } else {
            self.notebooks.push(Arc::new(copy_doc));
            self.library.push_slot(None, true, self.notebooks.len());
            self.sync.slot_ids.push(copy_id.clone());
            self.sync.slot_dirty_hash.push(true);
            self.sync.slot_edit_gen.push(1);
            self.sync.slot_placeholder_blank.push(false);
        }
        self.sync.manifest.mark_notebook_dirty(&copy_id, "");
        self.sync.last_dirty_at_ms = Some(now);
        copy_title
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
        let removed_id = self.sync.slot_ids.remove(idx);
        self.sync.in_flight_slot_gens.remove(&removed_id);
        self.sync.slot_dirty_hash.remove(idx);
        self.sync.slot_edit_gen.remove(idx);
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
        target.infinite_theme = incoming.infinite_theme;
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

pub(crate) fn ensure_default_profile_in_global(global: &mut GlobalSyncState) -> String {
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

pub(crate) fn cloud_account_cache_key(server_url: &str, user_id: &str) -> String {
    let norm = normalize_server_url(server_url).unwrap_or_else(|_| server_url.trim().to_string());
    format!("{norm}|{}", sanitize_account_dir_segment(user_id))
}

fn copy_dir_recursive_merge(src: &Path, dst: &Path) {
    let _ = fs::create_dir_all(dst);
    let Ok(entries) = fs::read_dir(src) else {
        return;
    };
    for entry in entries.flatten() {
        let src_path = entry.path();
        let Some(name) = src_path.file_name() else {
            continue;
        };
        if name == papercraft_sync::SYNC_MANIFEST_FILE {
            continue;
        }
        let dst_path = dst.join(name);
        if src_path.is_dir() {
            copy_dir_recursive_merge(&src_path, &dst_path);
        } else if src_path.is_file() {
            let _ = fs::copy(&src_path, &dst_path);
        }
    }
}

fn migrate_local_profile_dir_to_cloud_account(src_dir: &Path, dst_dir: &Path) {
    if let Some(parent) = dst_dir.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if !dst_dir.exists() && fs::rename(src_dir, dst_dir).is_ok() {
        let stale_manifest = dst_dir.join(papercraft_sync::SYNC_MANIFEST_FILE);
        if stale_manifest.exists() {
            let _ = fs::remove_file(stale_manifest);
        }
        return;
    }
    copy_dir_recursive_merge(src_dir, dst_dir);
    let _ = fs::remove_dir_all(src_dir);
}

fn format_conflict_copy_title(original_title: &str, existing: &[Arc<Document>]) -> String {
    let base = strip_conflict_copy_suffix(original_title);
    let first = sanitize_notebook_title(&format!("{base} (Conflict Copy)"));
    if !existing
        .iter()
        .any(|d| d.title.eq_ignore_ascii_case(&first))
    {
        return first;
    }
    for n in 2..=99 {
        let candidate = sanitize_notebook_title(&format!("{base} (Conflict Copy {n})"));
        if !existing
            .iter()
            .any(|d| d.title.eq_ignore_ascii_case(&candidate))
        {
            return candidate;
        }
    }
    first
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

pub(crate) fn migrate_web_cache_if_needed(cache: &mut WebWorkspaceCache) {
    if cache.local_profiles.is_empty()
        && !cache.global.needs_initial_nickname
        && let Some(guest_bundle) = cache.guest.take()
    {
        let pid = ensure_default_profile_in_global(&mut cache.global);
        cache.local_profiles.insert(pid, guest_bundle);
    }
    cache.version = WEB_CACHE_VERSION;
}

pub(crate) fn sanitize_global_state(global: &mut GlobalSyncState) {
    global.server_url =
        normalize_server_url(&global.server_url).unwrap_or_else(|_| DEFAULT_SERVER_URL.to_string());
    global.device_name = sanitize_device_name(&global.device_name);
    global.client_id = sanitize_client_id(&global.client_id);
    if global.client_id.is_empty() {
        global.client_id = Uuid::new_v4().to_string();
    }
    global.sanitize_release_notices();
    let _ = global.migrate_legacy_session();
    global.remembered_server_accounts =
        sanitize_remembered_accounts(std::mem::take(&mut global.remembered_server_accounts));
    for prof in &mut global.local_profiles {
        prof.local_profile_id = sanitize_account_dir_segment(&prof.local_profile_id);
        prof.nickname = sanitize_profile_nickname(&prof.nickname);
    }
    if !global.server_configured
        && (!global.cached_server_sessions.is_empty()
            || !global.remembered_server_accounts.is_empty()
            || !global.server_url.eq_ignore_ascii_case(DEFAULT_SERVER_URL))
    {
        global.server_configured = true;
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
                trash_id: if entry.trash_id.trim().is_empty() {
                    Uuid::new_v4().to_string()
                } else {
                    entry.trash_id.clone()
                },
                doc: Arc::new(d),
                deleted_at_ms: entry.deleted_at_ms,
                path: None,
            }
        })
        .collect();
    session.cleanup_expired_trash();

    session.sync.manifest = bundle.manifest.clone();
    session.sync.manifest.migrate_if_needed();
    session.sync.slot_ids = bundle.slot_ids.clone();
    session.sync.slot_dirty_hash = bundle.slot_dirty_hash.clone();
    session.sync.slot_placeholder_blank = bundle.slot_placeholder_blank.clone();
    session.sync.prefs_dirty_pending = bundle.prefs_dirty_pending;
    session.sync.last_dirty_at_ms = bundle.last_dirty_at_ms;
    session.ensure_sync_vec_lens(count);
    session
}

fn estimate_doc_attachment_bytes(doc: &Document) -> usize {
    let mut total = 0usize;
    for pdf in &doc.pdf_documents {
        total = total.saturating_add(pdf.bytes.len());
    }
    for tmpl in &doc.image_templates {
        total = total.saturating_add(tmpl.png_bytes.len());
    }
    if doc.pages.is_empty() {
        for img in &doc.images {
            total = total.saturating_add(img.png_bytes.len());
        }
    } else {
        for (p_idx, page) in doc.pages.iter().enumerate() {
            let images = if p_idx == doc.current_page {
                &doc.images
            } else {
                &page.images
            };
            for img in images {
                total = total.saturating_add(img.png_bytes.len());
            }
        }
    }
    total
}

fn most_recent_notebook_index(notebooks: &[Arc<Document>]) -> usize {
    notebooks
        .iter()
        .enumerate()
        .max_by(|(idx_a, da), (idx_b, db)| {
            da.updated_at_ms
                .cmp(&db.updated_at_ms)
                .then_with(|| da.created_at_ms.cmp(&db.created_at_ms))
                .then_with(|| da.meta_updated_at_ms.cmp(&db.meta_updated_at_ms))
                .then_with(|| {
                    db.title
                        .to_ascii_lowercase()
                        .cmp(&da.title.to_ascii_lowercase())
                })
                .then_with(|| db.id.cmp(&da.id))
                .then_with(|| idx_b.cmp(idx_a))
        })
        .map_or(0, |(idx, _)| idx)
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
        account_library_dir,
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
            .map(|(i, snap)| {
                let hash = if snap.content_hash.is_empty() {
                    canonical_document_hash(&snap.doc)
                } else {
                    snap.content_hash.clone()
                };
                papercraft_sync_protocol::AcceptedNotebookSync {
                    notebook_id: snap.notebook_id.clone(),
                    new_version: snap.base_version + 1,
                    cursor: (i + 1) as u64,
                    deleted: false,
                    content_hash: hash,
                    body_hash: canonical_document_body_hash(&snap.doc),
                    meta_hash: canonical_document_meta_hash(&snap.doc),
                }
            })
            .collect();
        let count = accepted.len();
        let accepted_page_hashes: std::collections::HashMap<String, Vec<String>> = batch
            .local_docs
            .iter()
            .map(|snap| {
                (
                    snap.notebook_id.clone(),
                    papercraft_sync_protocol::canonical_document_page_hashes(&snap.doc),
                )
            })
            .collect();
        let accepted_page_elements: std::collections::HashMap<
            String,
            Vec<papercraft_sync::PageElementFingerprints>,
        > = batch
            .local_docs
            .iter()
            .map(|snap| {
                (
                    snap.notebook_id.clone(),
                    papercraft_sync::compute_document_page_elements(&snap.doc),
                )
            })
            .collect();
        let server_user = session
            .active_account_session()
            .map(|s| s.user.clone())
            .unwrap_or_else(|| fake_account("user-a", "alice@example.com", "alice").user);
        session
            .apply_sync_outcome(SyncCycleOutcome {
                new_cursor: count as u64,
                refreshed_session: None,
                server_user,
                accepted_uploads: accepted,
                accepted_page_hashes,
                accepted_page_elements,
                rebased_docs: std::collections::HashMap::new(),
                rebased_ancestors: std::collections::HashMap::new(),
                accepted_folders: Vec::new(),
                cleared_tombstones: Vec::new(),
                downloaded_updates: Vec::new(),
                conflict_updates: Vec::new(),
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
    fn offline_first_cloud_session_caches_locally_across_restart_and_forget_wipes_cache() {
        let dir = temp_dir("offline_first_desktop");
        let mut session = Session::open_desktop_library(dir.0.clone());
        session
            .execute(
                "profile.completeFirstAccess",
                &json!({ "nickname": "Personal" }),
            )
            .unwrap();
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
        assert_eq!(
            session.library_dir(),
            Some(account_library_dir(&dir.0, "user-a").as_path())
        );
        assert!(has_notebook_title(&session, "Cloud Lecture Notes"));
        assert!(!has_notebook_title(&session, "Offline Sketchbook"));
        assert_eq!(session.cloud_pending_changes_count(), 0);
        assert_eq!(session.notebook_sync_status(0), EntitySyncStatus::Synced);

        // Creating a notebook while offline enters local queue and persists under accounts/<user_id>.
        session
            .execute(
                "notebook.create",
                &json!({ "title": "Queued Offline Note" }),
            )
            .unwrap();
        let batch = session.prepare_sync_batch().unwrap();
        assert!(!batch.force_overwrite);
        assert_eq!(batch.local_docs.len(), 1);
        let acct_dir = account_library_dir(&dir.0, "user-a");
        assert!(acct_dir.is_dir());

        // Reopening desktop while offline restores the Offline-First cloud session and pending queue!
        drop(session);
        let mut reopened = Session::open_desktop_library(dir.0.clone());
        assert!(reopened.is_signed_in());
        assert!(reopened.is_viewing_cloud());
        assert!(has_notebook_title(&reopened, "Cloud Lecture Notes"));
        assert!(has_notebook_title(&reopened, "Queued Offline Note"));
        assert!(
            reopened.server_account_pending_changes_count("http://127.0.0.1:8787", "user-a") >= 1
        );

        // Switching to local keeps the cloud session cached for offline reopening later.
        let pid = reopened
            .saved_local_profiles()
            .first()
            .unwrap()
            .local_profile_id
            .clone();
        reopened.switch_to_local_profile(&pid).unwrap();
        assert!(!reopened.is_viewing_cloud());
        assert!(reopened.has_cached_server_account("http://127.0.0.1:8787", "user-a"));
        assert!(
            reopened.server_account_pending_changes_count("http://127.0.0.1:8787", "user-a") >= 1
        );

        // Forgetting the server account deletes ONLY its local cache directory and shortcut.
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
        assert!(!reopened.has_cached_server_account("http://127.0.0.1:8787", "user-a"));
        assert!(!acct_dir.exists());
    }

    #[test]
    fn switching_between_local_and_cached_server_preserves_both_workspaces() {
        let dir = temp_dir("offline_switch");
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
            .execute("notebook.create", &json!({ "title": "Cached Cloud Note" }))
            .unwrap();
        assert!(session.is_viewing_cloud());

        // Switching to a local account persists the cloud workspace and keeps it cached.
        session.switch_to_local_profile(&personal_id).unwrap();
        assert!(!session.is_signed_in());
        assert!(!session.is_viewing_cloud());
        assert!(has_notebook_title(&session, "Personal Idea"));
        assert!(!has_notebook_title(&session, "Cached Cloud Note"));
        assert_eq!(session.remembered_server_accounts().len(), 1);
        assert!(session.has_cached_server_account("http://127.0.0.1:8787", "user-sw"));

        // `profile.switch target=cloud` re-enters the cached server account without needing network!
        session
            .execute(
                "profile.switch",
                &json!({
                    "target": "cloud",
                    "serverUrl": "http://127.0.0.1:8787",
                    "userId": "user-sw"
                }),
            )
            .unwrap();
        assert!(session.is_signed_in());
        assert!(session.is_viewing_cloud());
        assert!(has_notebook_title(&session, "Cached Cloud Note"));
        assert!(!has_notebook_title(&session, "Personal Idea"));
    }

    #[test]
    fn page_level_three_way_merge_combines_disjoint_pages_and_creates_copy_on_same_page_collision()
    {
        let mut session = Session::default();
        let acct = fake_account("user-b", "bob@example.com", "bob");
        session.enter_online_server_session(acct.clone(), empty_pull_for(&acct, 0));

        let page0 = papercraft_doc::NotebookPage::new(
            1404,
            1872,
            papercraft_doc::PaperTemplate::RuledNarrow,
            papercraft_doc::PaperColor::PureWhite,
        );
        let page1 = papercraft_doc::NotebookPage::new(
            1404,
            1872,
            papercraft_doc::PaperTemplate::RuledNarrow,
            papercraft_doc::PaperColor::PureWhite,
        );
        let mut base = Document {
            id: "nb-merge".to_string(),
            title: "Shared Notes".to_string(),
            cover: NotebookCover::Terracotta,
            pages: vec![page0, page1],
            current_page: 0,
            meta_updated_at_ms: 1000,
            ..Default::default()
        };
        base.load_active_from_pages();
        let base_hash = canonical_document_hash(&base);
        let base_page_hashes = papercraft_sync_protocol::canonical_document_page_hashes(&base);

        // Pull version 1 with 2 pages.
        session
            .apply_sync_outcome(SyncCycleOutcome {
                new_cursor: 1,
                refreshed_session: None,
                server_user: acct.user.clone(),
                accepted_uploads: Vec::new(),
                accepted_page_hashes: Default::default(),
                accepted_page_elements: Default::default(),
                rebased_docs: Default::default(),
                rebased_ancestors: Default::default(),
                accepted_folders: Vec::new(),
                cleared_tombstones: Vec::new(),
                downloaded_updates: vec![RemoteNotebookUpdate {
                    notebook_id: "nb-merge".to_string(),
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
                    page_hashes: base_page_hashes,
                }],
                conflict_updates: Vec::new(),
                downloaded_folders: Vec::new(),
                updated_prefs: None,
                ack_prefs: None,
                pushed_count: 0,
                pulled_count: 1,
                synced_at_ms: 1000,
            })
            .unwrap();

        // Device A (this session) draws offline on Page 0.
        session.commit_raw_stroke(
            PenSettings::ball_pen(),
            vec![
                StrokeSample::from_xy_pressure(20.0, 20.0, 0.5),
                StrokeSample::from_xy_pressure(60.0, 60.0, 0.7),
            ],
        );

        // Device B (remote conflict) edited Page 1 instead!
        let mut remote_diff_page = base.clone();
        let rsid = remote_diff_page.alloc_stroke_id();
        let rpts = vec![
            StrokeSample::from_xy_pressure(200.0, 200.0, 0.5),
            StrokeSample::from_xy_pressure(240.0, 240.0, 0.8),
        ];
        remote_diff_page.pages[1]
            .strokes
            .push(Arc::new(papercraft_doc::InkStroke::new(
                rsid,
                PenSettings::fountain_pen(),
                rpts.clone(),
                rpts,
            )));
        remote_diff_page.load_active_from_pages();
        let remote_diff_hash = canonical_document_hash(&remote_diff_page);
        let remote_diff_pages =
            papercraft_sync_protocol::canonical_document_page_hashes(&remote_diff_page);

        session
            .apply_sync_outcome(SyncCycleOutcome {
                new_cursor: 2,
                refreshed_session: None,
                server_user: acct.user.clone(),
                accepted_uploads: Vec::new(),
                accepted_page_hashes: Default::default(),
                accepted_page_elements: Default::default(),
                rebased_docs: Default::default(),
                rebased_ancestors: Default::default(),
                accepted_folders: Vec::new(),
                cleared_tombstones: Vec::new(),
                downloaded_updates: Vec::new(),
                conflict_updates: vec![RemoteNotebookUpdate {
                    notebook_id: "nb-merge".to_string(),
                    version: 2,
                    cursor: 2,
                    deleted: false,
                    updated_at_ms: 2000,
                    deleted_at_ms: None,
                    content_hash: remote_diff_hash,
                    body_hash: canonical_document_body_hash(&remote_diff_page),
                    meta_hash: canonical_document_meta_hash(&remote_diff_page),
                    meta_updated_at_ms: 2000,
                    document: Some(remote_diff_page.clone()),
                    page_hashes: remote_diff_pages.clone(),
                }],
                downloaded_folders: Vec::new(),
                updated_prefs: None,
                ack_prefs: None,
                pushed_count: 0,
                pulled_count: 1,
                synced_at_ms: 2500,
            })
            .unwrap();

        // Both Page 0 and Page 1 edits survived cleanly with zero Conflict Copy!
        assert_eq!(session.notebooks.len(), 1);
        assert_eq!(session.notebooks[0].pages[0].strokes.len(), 1);
        assert_eq!(session.notebooks[0].pages[1].strokes.len(), 1);
        assert!(session.take_last_conflict_notice().is_some());

        // Concurrent independent strokes on Page 0 ALSO merge at the element level cleanly!
        let mut remote_same_page = remote_diff_page;
        let csid = remote_same_page.alloc_stroke_id();
        let cpts = vec![
            StrokeSample::from_xy_pressure(300.0, 300.0, 0.6),
            StrokeSample::from_xy_pressure(340.0, 340.0, 0.8),
        ];
        remote_same_page.pages[0]
            .strokes
            .push(Arc::new(papercraft_doc::InkStroke::new(
                csid,
                PenSettings::fountain_pen(),
                cpts.clone(),
                cpts,
            )));
        remote_same_page.load_active_from_pages();
        let remote_same_hash = canonical_document_hash(&remote_same_page);
        let remote_same_pages =
            papercraft_sync_protocol::canonical_document_page_hashes(&remote_same_page);

        session
            .apply_sync_outcome(SyncCycleOutcome {
                new_cursor: 3,
                refreshed_session: None,
                server_user: acct.user.clone(),
                accepted_uploads: Vec::new(),
                accepted_page_hashes: Default::default(),
                accepted_page_elements: Default::default(),
                rebased_docs: Default::default(),
                rebased_ancestors: Default::default(),
                accepted_folders: Vec::new(),
                cleared_tombstones: Vec::new(),
                downloaded_updates: Vec::new(),
                conflict_updates: vec![RemoteNotebookUpdate {
                    notebook_id: "nb-merge".to_string(),
                    version: 3,
                    cursor: 3,
                    deleted: false,
                    updated_at_ms: 3000,
                    deleted_at_ms: None,
                    content_hash: remote_same_hash,
                    body_hash: canonical_document_body_hash(&remote_same_page),
                    meta_hash: canonical_document_meta_hash(&remote_same_page),
                    meta_updated_at_ms: 3000,
                    document: Some(remote_same_page.clone()),
                    page_hashes: remote_same_pages,
                }],
                downloaded_folders: Vec::new(),
                updated_prefs: None,
                ack_prefs: None,
                pushed_count: 0,
                pulled_count: 1,
                synced_at_ms: 3500,
            })
            .unwrap();

        assert_eq!(session.notebooks.len(), 1);
        assert_eq!(session.notebooks[0].pages[0].strokes.len(), 2);
        assert_eq!(session.notebooks[0].pages[1].strokes.len(), 1);

        // Now simulate a true irreconcilable collision on Page 0 properties (conflicting templates).
        {
            let doc = Arc::make_mut(&mut session.notebooks[0]);
            doc.pages[0].paper_template = papercraft_doc::PaperTemplate::Grid;
            doc.load_active_from_pages();
        }
        session.doc = Arc::clone(&session.notebooks[0]);

        let mut remote_prop_conflict = remote_same_page;
        remote_prop_conflict.pages[0].paper_template = papercraft_doc::PaperTemplate::Dotted;
        remote_prop_conflict.load_active_from_pages();
        let remote_prop_hash = canonical_document_hash(&remote_prop_conflict);
        let remote_prop_pages =
            papercraft_sync_protocol::canonical_document_page_hashes(&remote_prop_conflict);

        session
            .apply_sync_outcome(SyncCycleOutcome {
                new_cursor: 4,
                refreshed_session: None,
                server_user: acct.user,
                accepted_uploads: Vec::new(),
                accepted_page_hashes: Default::default(),
                accepted_page_elements: Default::default(),
                rebased_docs: Default::default(),
                rebased_ancestors: Default::default(),
                accepted_folders: Vec::new(),
                cleared_tombstones: Vec::new(),
                downloaded_updates: Vec::new(),
                conflict_updates: vec![RemoteNotebookUpdate {
                    notebook_id: "nb-merge".to_string(),
                    version: 4,
                    cursor: 4,
                    deleted: false,
                    updated_at_ms: 4000,
                    deleted_at_ms: None,
                    content_hash: remote_prop_hash,
                    body_hash: canonical_document_body_hash(&remote_prop_conflict),
                    meta_hash: canonical_document_meta_hash(&remote_prop_conflict),
                    meta_updated_at_ms: 4000,
                    document: Some(remote_prop_conflict),
                    page_hashes: remote_prop_pages,
                }],
                downloaded_folders: Vec::new(),
                updated_prefs: None,
                ack_prefs: None,
                pushed_count: 0,
                pulled_count: 1,
                synced_at_ms: 4500,
            })
            .unwrap();

        assert_eq!(session.notebooks.len(), 2);
        assert!(has_notebook_title(&session, "Shared Notes (Conflict Copy)"));
    }

    #[test]
    fn web_cache_persists_offline_first_cloud_workspace_and_forget_clears_only_that_account() {
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
            title: "Synced Cloud Note".to_string(),
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
                title: "Synced Cloud Note".to_string(),
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
            .execute("notebook.create", &json!({ "title": "Queued Web Note" }))
            .unwrap();

        let cached_json = session.export_web_cache_json().unwrap();
        assert!(cached_json.contains("Synced Cloud Note"));
        assert!(cached_json.contains("Queued Web Note"));
        assert!(cached_json.contains("web@example.com"));

        // Reloading browser restores Offline-First cloud session and pending queue!
        let mut reloaded = Session::open_web_cache(Some(&cached_json));
        assert!(reloaded.is_signed_in());
        assert!(reloaded.is_viewing_cloud());
        assert!(has_notebook_title(&reloaded, "Synced Cloud Note"));
        assert!(has_notebook_title(&reloaded, "Queued Web Note"));
        assert_eq!(reloaded.remembered_server_accounts().len(), 1);
        assert!(
            reloaded.server_account_pending_changes_count("http://127.0.0.1:8787", "web-u1") >= 1
        );

        // Forgetting the server account removes ONLY its cache and restores local profile.
        assert!(
            reloaded
                .execute(
                    "profile.forgetServerAccount",
                    &json!({ "serverUrl": "http://127.0.0.1:8787", "userId": "web-u1" })
                )
                .unwrap()["forgotten"]
                .as_bool()
                .unwrap()
        );
        assert!(!reloaded.is_viewing_cloud());
        assert!(!reloaded.has_cached_server_account("http://127.0.0.1:8787", "web-u1"));
        assert!(reloaded.remembered_server_accounts().is_empty());
    }

    #[test]
    fn local_account_conversion_uploads_everything_and_keeps_offline_first_cache() {
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
        let cloud_dir = account_library_dir(&dir.0, "user-bruno");
        assert!(
            cloud_dir.is_dir(),
            "local notebooks must remain on device as the Offline-First server cache"
        );
        assert_eq!(session.library_dir(), Some(cloud_dir.as_path()));
        assert_eq!(session.prefs.theme, UiTheme::Dark);

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
    fn first_notebook_data_notice_shows_once_per_release_on_persisted_desktop_and_web_sessions() {
        let ephemeral = Session::default();
        assert!(!ephemeral.should_show_first_notebook_data_notice());

        let dir = temp_dir("first_notebook_data_notice");
        let mut desktop = Session::open_desktop_library(dir.0.clone());
        assert!(desktop.should_show_first_notebook_data_notice());
        desktop
            .execute("profile.dismissFirstNotebookDataNotice", &json!({}))
            .unwrap();
        assert!(!desktop.should_show_first_notebook_data_notice());
        assert_eq!(
            desktop.first_notebook_data_notice_seen_version(),
            Some(env!("CARGO_PKG_VERSION"))
        );

        let reopened_desktop = Session::open_desktop_library(dir.0.clone());
        assert!(!reopened_desktop.should_show_first_notebook_data_notice());
        assert!(reopened_desktop.should_show_first_notebook_data_notice_for_version("99.0.0"));

        let mut legacy = papercraft_sync::load_global_state(&dir.0);
        legacy.first_notebook_data_notice_seen = true;
        legacy.first_notebook_data_notice_seen_version = None;
        papercraft_sync::save_global_state(&dir.0, &legacy).unwrap();
        let updated_from_legacy = Session::open_desktop_library(dir.0.clone());
        assert!(updated_from_legacy.should_show_first_notebook_data_notice());

        let mut web = Session::open_web_cache(None);
        assert!(web.should_show_first_notebook_data_notice());
        web.execute("profile.dismissFirstNotebookDataNotice", &json!({}))
            .unwrap();
        assert!(!web.should_show_first_notebook_data_notice());
        assert_eq!(
            web.first_notebook_data_notice_seen_version(),
            Some(env!("CARGO_PKG_VERSION"))
        );
        let saved_web = web.export_web_cache_json().unwrap();
        let reopened_web = Session::open_web_cache(Some(&saved_web));
        assert!(!reopened_web.should_show_first_notebook_data_notice());
        assert!(reopened_web.should_show_first_notebook_data_notice_for_version("99.0.0"));
    }

    #[test]
    fn infinite_theme_notice_shows_once_per_release_on_persisted_desktop_and_web_sessions() {
        let ephemeral = Session::default();
        assert!(!ephemeral.should_show_infinite_theme_notice());

        let dir = temp_dir("infinite_theme_notice");
        let mut desktop = Session::open_desktop_library(dir.0.clone());
        assert!(desktop.should_show_infinite_theme_notice());
        desktop
            .execute("profile.dismissInfiniteThemeNotice", &json!({}))
            .unwrap();
        assert!(!desktop.should_show_infinite_theme_notice());
        assert_eq!(
            desktop.infinite_theme_notice_seen_version(),
            Some(env!("CARGO_PKG_VERSION"))
        );

        let reopened_desktop = Session::open_desktop_library(dir.0.clone());
        assert!(!reopened_desktop.should_show_infinite_theme_notice());
        assert!(reopened_desktop.should_show_infinite_theme_notice_for_version("99.0.0"));

        let mut legacy = papercraft_sync::load_global_state(&dir.0);
        legacy.infinite_theme_notice_seen = true;
        legacy.infinite_theme_notice_seen_version = None;
        papercraft_sync::save_global_state(&dir.0, &legacy).unwrap();
        let updated_from_legacy = Session::open_desktop_library(dir.0.clone());
        assert!(updated_from_legacy.should_show_infinite_theme_notice());

        let mut web = Session::open_web_cache(None);
        assert!(web.should_show_infinite_theme_notice());
        web.execute("profile.dismissInfiniteThemeNotice", &json!({}))
            .unwrap();
        assert!(!web.should_show_infinite_theme_notice());
        assert_eq!(
            web.infinite_theme_notice_seen_version(),
            Some(env!("CARGO_PKG_VERSION"))
        );
        let saved_web = web.export_web_cache_json().unwrap();
        let reopened_web = Session::open_web_cache(Some(&saved_web));
        assert!(!reopened_web.should_show_infinite_theme_notice());
        assert!(reopened_web.should_show_infinite_theme_notice_for_version("99.0.0"));
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

    #[test]
    fn off_ui_hashing_and_mid_upload_edits_stay_accurate() {
        let mut session = Session::default();
        let acct = fake_account("user-hash", "cloud@example.com", "hashuser");
        let doc = Document {
            id: "nb-1".to_string(),
            title: "Cloud Note".to_string(),
            ..Default::default()
        };
        let pull = SyncPullResponse {
            server_cursor: 1,
            user: acct.user.clone(),
            notebooks: vec![RemoteNotebookRecord {
                notebook_id: "nb-1".to_string(),
                version: 1,
                cursor: 1,
                deleted: false,
                updated_at_ms: 1000,
                deleted_at_ms: None,
                title: "Cloud Note".to_string(),
                content_hash: canonical_document_hash(&doc),
                body_hash: canonical_document_body_hash(&doc),
                meta_hash: canonical_document_meta_hash(&doc),
                meta_updated_at_ms: 1000,
                document: Some(doc),
            }],
            folders: Vec::new(),
            studio_prefs: None,
        };
        session.enter_online_server_session(acct, pull);
        assert_eq!(session.cloud_pending_changes_count(), 0);
        assert_eq!(session.dirty_sync_estimated_payload_bytes(), 0);

        // Attach a 5 MiB PDF payload to verify byte estimation before upload.
        let mut big_doc = (*session.notebooks[0]).clone();
        big_doc.pdf_documents.push(papercraft_doc::EmbeddedPdf {
            name: "Lecture.pdf".to_string(),
            bytes: std::sync::Arc::new(vec![0u8; 5 * 1024 * 1024]),
        });
        session.notebooks[0] = std::sync::Arc::new(big_doc);
        session.doc = std::sync::Arc::clone(&session.notebooks[0]);
        session.sync_mark_slot_dirty(0);

        assert_eq!(
            session.dirty_sync_estimated_payload_bytes(),
            5 * 1024 * 1024
        );

        let batch = session.prepare_sync_batch().unwrap();
        assert_eq!(batch.local_docs.len(), 1);
        assert!(
            batch.local_docs[0].content_hash.is_empty(),
            "prepare_sync_batch must defer SHA-256 computation to the background worker"
        );

        // Simulate user drawing another stroke while the previous push is still in flight.
        session
            .execute(
                "stroke.draw",
                &json!({
                    "points": [
                        { "x": 10.0, "y": 10.0, "pressure": 0.5 },
                        { "x": 20.0, "y": 20.0, "pressure": 0.5 }
                    ]
                }),
            )
            .unwrap();

        // ACK the older in-flight snapshot: the newer mid-upload edit must keep the notebook dirty!
        apply_fake_upload_acks(&mut session, &batch, 2000);
        assert_eq!(session.cloud_pending_changes_count(), 1);
        assert!(session.last_dirty_at_ms().is_some());

        // Second push uploads the mid-flight edit cleanly and clears dirty state.
        let batch2 = session.prepare_sync_batch().unwrap();
        assert_eq!(batch2.local_docs.len(), 1);
        apply_fake_upload_acks(&mut session, &batch2, 3000);
        assert_eq!(session.cloud_pending_changes_count(), 0);
        assert_eq!(session.last_dirty_at_ms(), None);
        assert_eq!(session.dirty_sync_estimated_payload_bytes(), 0);
    }

    #[test]
    fn offline_edits_to_synced_cloud_notebook_persist_dirty_queue_across_switch_and_restart() {
        // Desktop persistence check
        let root = std::env::temp_dir().join(format!(
            "papercraft-offline-dirty-desk-{}",
            uuid::Uuid::new_v4()
        ));
        let mut session = Session::open_desktop_library(root.clone());
        let local_prof = session.complete_first_access_nickname("Alex").unwrap();
        let acct = fake_account("u-queue", "queue@example.com", "queue");
        let synced_doc = Document {
            id: "nb-synced".to_string(),
            title: "Existing Cloud".to_string(),
            ..Default::default()
        };
        let pull = SyncPullResponse {
            server_cursor: 1,
            user: acct.user.clone(),
            notebooks: vec![RemoteNotebookRecord {
                notebook_id: "nb-synced".to_string(),
                version: 1,
                cursor: 1,
                deleted: false,
                updated_at_ms: 1000,
                deleted_at_ms: None,
                title: "Existing Cloud".to_string(),
                content_hash: canonical_document_hash(&synced_doc),
                body_hash: canonical_document_body_hash(&synced_doc),
                meta_hash: canonical_document_meta_hash(&synced_doc),
                meta_updated_at_ms: 1000,
                document: Some(synced_doc),
            }],
            folders: Vec::new(),
            studio_prefs: None,
        };
        session.enter_online_server_session(acct, pull);
        assert_eq!(session.cloud_pending_changes_count(), 0);

        // Edit the already-synced notebook while offline and switch away before any batch push runs.
        session
            .execute(
                "stroke.draw",
                &json!({
                    "points": [
                        { "x": 4.0, "y": 4.0, "pressure": 0.5 },
                        { "x": 14.0, "y": 14.0, "pressure": 0.5 }
                    ]
                }),
            )
            .unwrap();
        assert_eq!(session.cloud_pending_changes_count(), 1);
        session.flush_for_exit();
        session
            .switch_to_local_profile(&local_prof.local_profile_id)
            .unwrap();
        drop(session);

        let mut reopened = Session::open_desktop_library(root.clone());
        assert_eq!(
            reopened.server_account_pending_changes_count("http://127.0.0.1:8787", "u-queue"),
            1
        );
        reopened
            .switch_to_cached_server_account("http://127.0.0.1:8787", "u-queue")
            .unwrap();
        assert_eq!(reopened.cloud_pending_changes_count(), 1);
        let batch = reopened.prepare_sync_batch().unwrap();
        assert_eq!(batch.local_docs.len(), 1);
        let _ = std::fs::remove_dir_all(&root);

        // Web persistence check
        let mut web = Session::open_web_cache(None);
        let web_local = web.complete_first_access_nickname("WebUser").unwrap();
        let web_acct = fake_account("u-webq", "webq@example.com", "webq");
        let web_doc = Document {
            id: "nb-web-synced".to_string(),
            title: "Web Cloud".to_string(),
            ..Default::default()
        };
        let web_pull = SyncPullResponse {
            server_cursor: 1,
            user: web_acct.user.clone(),
            notebooks: vec![RemoteNotebookRecord {
                notebook_id: "nb-web-synced".to_string(),
                version: 1,
                cursor: 1,
                deleted: false,
                updated_at_ms: 1000,
                deleted_at_ms: None,
                title: "Web Cloud".to_string(),
                content_hash: canonical_document_hash(&web_doc),
                body_hash: canonical_document_body_hash(&web_doc),
                meta_hash: canonical_document_meta_hash(&web_doc),
                meta_updated_at_ms: 1000,
                document: Some(web_doc),
            }],
            folders: Vec::new(),
            studio_prefs: None,
        };
        web.enter_online_server_session(web_acct, web_pull);
        web.execute(
            "stroke.draw",
            &json!({
                "points": [
                    { "x": 8.0, "y": 8.0, "pressure": 0.5 },
                    { "x": 18.0, "y": 18.0, "pressure": 0.5 }
                ]
            }),
        )
        .unwrap();
        let exported = web.export_web_cache_json().unwrap();
        web.switch_to_local_profile(&web_local.local_profile_id)
            .unwrap();
        assert_eq!(
            web.server_account_pending_changes_count("http://127.0.0.1:8787", "u-webq"),
            1
        );

        let web_reopened = Session::open_web_cache(Some(&exported));
        assert!(web_reopened.is_viewing_cloud());
        assert_eq!(web_reopened.cloud_pending_changes_count(), 1);
    }

    #[test]
    fn reauthenticating_expired_session_keeps_offline_edits_without_false_conflicts() {
        let mut session = Session::open_web_cache(None);
        let acct = fake_account("u-exp", "expired@example.com", "expired");
        let synced_doc = Document {
            id: "nb-exp".to_string(),
            title: "Lecture Notes".to_string(),
            pages: vec![
                papercraft_doc::NotebookPage::new(
                    1000,
                    1400,
                    papercraft_doc::PaperTemplate::Blank,
                    papercraft_doc::PaperColor::default(),
                ),
                papercraft_doc::NotebookPage::new(
                    1000,
                    1400,
                    papercraft_doc::PaperTemplate::RuledNarrow,
                    papercraft_doc::PaperColor::default(),
                ),
            ],
            ..Default::default()
        };
        let remote_record = RemoteNotebookRecord {
            notebook_id: "nb-exp".to_string(),
            version: 1,
            cursor: 1,
            deleted: false,
            updated_at_ms: 1000,
            deleted_at_ms: None,
            title: "Lecture Notes".to_string(),
            content_hash: canonical_document_hash(&synced_doc),
            body_hash: canonical_document_body_hash(&synced_doc),
            meta_hash: canonical_document_meta_hash(&synced_doc),
            meta_updated_at_ms: 1000,
            document: Some(synced_doc),
        };
        let pull = SyncPullResponse {
            server_cursor: 1,
            user: acct.user.clone(),
            notebooks: vec![remote_record.clone()],
            folders: Vec::new(),
            studio_prefs: None,
        };
        session.enter_online_server_session(acct.clone(), pull);
        assert_eq!(session.cloud_pending_changes_count(), 0);

        // Token expires and user continues editing offline (including deleting a page & attaching a PDF).
        session.record_session_expired("Session expired".to_string());
        assert!(session.is_session_expired());

        let mut edited = (*session.notebooks[0]).clone();
        edited.title = "Edited Offline".to_string();
        edited.pages.pop();
        edited.pdf_documents.push(papercraft_doc::EmbeddedPdf {
            name: "Slides.pdf".to_string(),
            bytes: std::sync::Arc::new(b"%PDF-1.4\n".to_vec()),
        });
        edited.sync_active_to_pages();
        session.notebooks[0] = std::sync::Arc::new(edited);
        session.doc = std::sync::Arc::clone(&session.notebooks[0]);
        session.sync_mark_slot_dirty(0);

        // User enters password again: server returns the unchanged v1 record in initial_pull.
        let reauth_pull = SyncPullResponse {
            server_cursor: 1,
            user: acct.user.clone(),
            notebooks: vec![remote_record],
            folders: Vec::new(),
            studio_prefs: None,
        };
        session.enter_online_server_session(acct, reauth_pull);

        assert!(!session.is_session_expired());
        assert_eq!(
            session.notebooks.len(),
            1,
            "re-auth must NOT create a spurious Conflict Copy when the server version did not change"
        );
        assert_eq!(session.notebooks[0].title, "Edited Offline");
        assert_eq!(session.notebooks[0].pages.len(), 1);
        assert_eq!(session.notebooks[0].pdf_documents.len(), 1);
        assert_eq!(session.cloud_pending_changes_count(), 1);
        assert!(session.last_dirty_at_ms().is_some());
        let batch = session.prepare_sync_batch().unwrap();
        assert_eq!(batch.local_docs.len(), 1);
        assert_eq!(batch.local_docs[0].base_version, 1);
    }

    #[test]
    fn web_sign_out_then_switch_to_local_preserves_local_notebooks() {
        let mut session = Session::open_web_cache(None);
        session
            .execute(
                "profile.completeFirstAccess",
                &json!({ "nickname": "OfflineNote" }),
            )
            .unwrap();
        session
            .execute("notebook.create", &json!({ "title": "My Local Journal" }))
            .unwrap();
        let local_id = session
            .active_local_profile()
            .unwrap()
            .local_profile_id
            .clone();

        let acct = fake_account("u-signout", "cloud@example.com", "clouduser");
        let cloud_doc = Document {
            id: "nb-cloud-only".to_string(),
            title: "Cloud Only Doc".to_string(),
            ..Default::default()
        };
        let pull = SyncPullResponse {
            server_cursor: 1,
            user: acct.user.clone(),
            notebooks: vec![RemoteNotebookRecord {
                notebook_id: "nb-cloud-only".to_string(),
                version: 1,
                cursor: 1,
                deleted: false,
                updated_at_ms: 1000,
                deleted_at_ms: None,
                title: "Cloud Only Doc".to_string(),
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
        assert!(has_notebook_title(&session, "Cloud Only Doc"));
        assert!(!has_notebook_title(&session, "My Local Journal"));

        assert!(session.sign_out_from_server_session());
        assert!(!session.has_loaded_workspace());

        // Switching back into the existing local account must still have My Local Journal intact.
        session.switch_to_local_profile(&local_id).unwrap();
        assert!(session.has_loaded_workspace());
        assert!(has_notebook_title(&session, "My Local Journal"));
        assert!(!has_notebook_title(&session, "Cloud Only Doc"));
    }

    #[test]
    fn direct_cloud_to_cloud_switch_preserves_account_a_pending_and_isolates_web_deltas() {
        let mut session = Session::open_web_cache(None);
        let acct_a = fake_account("u-a", "alice@example.com", "alice");
        session.enter_online_server_session(acct_a.clone(), empty_pull_for(&acct_a, 0));
        session
            .execute("notebook.create", &json!({ "title": "Alice Unsynced" }))
            .unwrap();
        assert_eq!(session.cloud_pending_changes_count(), 1);

        let acct_b = fake_account("u-b", "bob@example.com", "bob");
        let bob_doc = Document {
            id: "nb-bob".to_string(),
            title: "Bob Synced".to_string(),
            ..Default::default()
        };
        let pull_b = SyncPullResponse {
            server_cursor: 2,
            user: acct_b.user.clone(),
            notebooks: vec![RemoteNotebookRecord {
                notebook_id: "nb-bob".to_string(),
                version: 1,
                cursor: 2,
                deleted: false,
                updated_at_ms: 1000,
                deleted_at_ms: None,
                title: "Bob Synced".to_string(),
                content_hash: canonical_document_hash(&bob_doc),
                body_hash: canonical_document_body_hash(&bob_doc),
                meta_hash: canonical_document_meta_hash(&bob_doc),
                meta_updated_at_ms: 1000,
                document: Some(bob_doc),
            }],
            folders: Vec::new(),
            studio_prefs: None,
        };
        session.enter_online_server_session(acct_b, pull_b);

        assert_eq!(session.notebooks.len(), 1);
        assert_eq!(session.notebooks[0].title, "Bob Synced");
        assert_eq!(
            session.server_account_notebook_count(&acct_a.server_url, "u-a"),
            1
        );
        assert_eq!(
            session.server_account_pending_changes_count(&acct_a.server_url, "u-a"),
            1
        );

        let exported = session.export_web_cache_json().unwrap();
        let reopened = Session::open_web_cache(Some(&exported));
        assert_eq!(
            reopened.server_account_notebook_count(&acct_a.server_url, "u-a"),
            1
        );
        assert_eq!(
            reopened.server_account_pending_changes_count(&acct_a.server_url, "u-a"),
            1
        );
        assert_eq!(
            reopened
                .active_account_session()
                .map(|s| s.user.user_id.as_str()),
            Some("u-b")
        );
    }

    #[test]
    fn apply_sync_outcome_rejects_mismatched_user_id() {
        let mut session = Session::open_web_cache(None);
        let acct_b = fake_account("u-b", "bob@example.com", "bob");
        let pull_b = empty_pull_for(&acct_b, 0);
        session.enter_online_server_session(acct_b, pull_b);

        let alice_doc = Document {
            id: "nb-alice".to_string(),
            title: "Stale Alice Doc".to_string(),
            ..Default::default()
        };
        let stale_outcome = SyncCycleOutcome {
            new_cursor: 10,
            refreshed_session: None,
            server_user: fake_account("u-a", "alice@example.com", "alice").user,
            accepted_uploads: Vec::new(),
            accepted_page_hashes: std::collections::HashMap::new(),
            accepted_page_elements: std::collections::HashMap::new(),
            rebased_docs: std::collections::HashMap::new(),
            rebased_ancestors: std::collections::HashMap::new(),
            accepted_folders: Vec::new(),
            cleared_tombstones: Vec::new(),
            downloaded_updates: vec![papercraft_sync::RemoteNotebookUpdate {
                notebook_id: "nb-alice".to_string(),
                version: 1,
                cursor: 10,
                deleted: false,
                updated_at_ms: 1000,
                deleted_at_ms: None,
                content_hash: canonical_document_hash(&alice_doc),
                body_hash: canonical_document_body_hash(&alice_doc),
                meta_hash: canonical_document_meta_hash(&alice_doc),
                meta_updated_at_ms: 1000,
                document: Some(alice_doc),
                page_hashes: Vec::new(),
            }],
            conflict_updates: Vec::new(),
            downloaded_folders: Vec::new(),
            updated_prefs: None,
            ack_prefs: None,
            pushed_count: 0,
            pulled_count: 1,
            synced_at_ms: 2000,
        };
        let err = session.apply_sync_outcome(stale_outcome).unwrap_err();
        assert!(err.contains("different server account"), "{err}");
        assert!(!has_notebook_title(&session, "Stale Alice Doc"));
    }

    #[test]
    fn startup_picker_preference_command_persists_across_reopen() {
        let dir = temp_dir("startup_picker_pref");
        let mut session = Session::open_desktop_library(dir.0.clone());
        assert!(!session.skip_account_picker_on_startup());
        assert!(session.needs_initial_nickname());
        assert!(
            !session.has_any_saved_accounts() && !session.should_show_startup_account_picker(),
            "clean first-time install with 0 saved accounts must use the simple nickname prompt, not the account picker"
        );

        session
            .execute(
                "profile.completeFirstAccess",
                &json!({ "nickname": "Alex" }),
            )
            .unwrap();
        assert!(session.has_any_saved_accounts());
        assert!(session.should_show_startup_account_picker());

        session
            .execute("profile.setStartupPicker", &json!({ "skip": true }))
            .unwrap();
        assert!(session.skip_account_picker_on_startup());
        assert!(!session.should_show_startup_account_picker());

        let reopened = Session::open_desktop_library(dir.0.clone());
        assert!(reopened.skip_account_picker_on_startup());
        assert!(!reopened.should_show_startup_account_picker());
    }

    #[test]
    fn pdf_import_sets_timestamps_and_marks_new_cloud_slot_dirty() {
        let mut session = Session::open_web_cache(None);
        let acct = fake_account("u-pdf", "pdf@example.com", "pdfuser");
        session.enter_online_server_session(acct.clone(), empty_pull_for(&acct, 0));

        // Minimal 1-page valid PDF payload.
        let mut pdf = Vec::new();
        pdf.extend_from_slice(b"%PDF-1.4\n");
        let o1 = pdf.len();
        pdf.extend_from_slice(b"1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n");
        let o2 = pdf.len();
        pdf.extend_from_slice(b"2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n");
        let o3 = pdf.len();
        pdf.extend_from_slice(
            b"3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 600 800] >>\nendobj\n",
        );
        let xref = pdf.len();
        pdf.extend_from_slice(
            format!(
                "xref\n0 4\n0000000000 65535 f \n{o1:010} 00000 n \n{o2:010} 00000 n \n{o3:010} 00000 n \n"
            )
            .as_bytes(),
        );
        pdf.extend_from_slice(
            format!("trailer\n<< /Size 4 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
        );

        let prepared = crate::prepare_pdf_import_from_bytes(
            pdf,
            Some("English for Everyone.pdf".to_string()),
            crate::PdfImportMode::NewNotebook,
        )
        .expect("prepare minimal pdf");
        session
            .apply_prepared_pdf_import(prepared, false, false)
            .expect("apply async pdf import");

        assert_eq!(session.notebooks.len(), 1);
        assert!(session.doc.created_at_ms > 0);
        assert!(session.doc.updated_at_ms > 0);
        assert!(session.doc.meta_updated_at_ms > 0);
        assert_eq!(session.cloud_pending_changes_count(), 1);
        assert!(session.last_dirty_at_ms().is_some());
    }

    #[test]
    fn prepare_local_conversion_preserves_timestamps_and_backfills_zero() {
        let dir = temp_dir("conversion_preserve_ts");
        let mut session = Session::open_desktop_library(dir.0.clone());
        session
            .execute(
                "profile.completeFirstAccess",
                &json!({ "nickname": "Alex" }),
            )
            .unwrap();
        let pid = session
            .active_local_profile()
            .expect("active local profile")
            .local_profile_id
            .clone();

        let mut older = Document {
            id: "nb-daily".to_string(),
            title: "Daily Notebook".to_string(),
            created_at_ms: 1_000,
            updated_at_ms: 2_000,
            meta_updated_at_ms: 2_000,
            ..Default::default()
        };
        older.sync_active_to_pages();
        let mut newer = Document {
            id: "nb-english".to_string(),
            title: "English Grammar".to_string(),
            created_at_ms: 3_000,
            updated_at_ms: 8_000,
            meta_updated_at_ms: 8_000,
            ..Default::default()
        };
        newer.sync_active_to_pages();
        let mut zeroed = Document {
            id: "nb-zero".to_string(),
            title: "Zero Timestamp".to_string(),
            created_at_ms: 0,
            updated_at_ms: 0,
            meta_updated_at_ms: 0,
            ..Default::default()
        };
        zeroed.sync_active_to_pages();

        session.notebooks = vec![Arc::new(older), Arc::new(newer), Arc::new(zeroed)];
        session.doc = Arc::clone(&session.notebooks[0]);
        session.library.sync_len(3);
        session.sync_reconcile_slots();
        session.flush_active_workspace_to_storage();

        let (_target_pid, conv_batch) = session
            .prepare_local_account_conversion_batch(&pid)
            .expect("conversion batch");
        let find_doc = |id: &str| {
            conv_batch
                .local_docs
                .iter()
                .find(|snap| snap.notebook_id == id)
                .map(|snap| &snap.doc)
                .expect("converted doc")
        };
        assert_eq!(find_doc("nb-daily").updated_at_ms, 2_000);
        assert_eq!(find_doc("nb-english").updated_at_ms, 8_000);
        assert!(find_doc("nb-zero").updated_at_ms > 8_000);
        assert!(find_doc("nb-zero").created_at_ms > 0);
    }

    #[test]
    fn fresh_cloud_login_picks_most_recent_notebook_instead_of_oldest_cursor() {
        let mut session = Session::open_web_cache(None);
        let acct = fake_account("u-recent", "recent@example.com", "recent");

        let mut old_doc = Document {
            id: "nb-old".to_string(),
            title: "Daily Notebook".to_string(),
            created_at_ms: 1_000,
            updated_at_ms: 1_000,
            meta_updated_at_ms: 1_000,
            ..Default::default()
        };
        old_doc.sync_active_to_pages();

        let mut recent_doc = Document {
            id: "nb-recent".to_string(),
            title: "English Grammar".to_string(),
            created_at_ms: 5_000,
            updated_at_ms: 9_000,
            meta_updated_at_ms: 9_000,
            ..Default::default()
        };
        recent_doc.sync_active_to_pages();

        let pull = SyncPullResponse {
            server_cursor: 2,
            user: acct.user.clone(),
            notebooks: vec![
                RemoteNotebookRecord {
                    notebook_id: old_doc.id.clone(),
                    version: 1,
                    cursor: 1,
                    deleted: false,
                    updated_at_ms: 9_999,
                    deleted_at_ms: None,
                    title: old_doc.title.clone(),
                    content_hash: canonical_document_hash(&old_doc),
                    body_hash: canonical_document_body_hash(&old_doc),
                    meta_hash: canonical_document_meta_hash(&old_doc),
                    meta_updated_at_ms: 1_000,
                    document: Some(old_doc),
                },
                RemoteNotebookRecord {
                    notebook_id: recent_doc.id.clone(),
                    version: 1,
                    cursor: 2,
                    deleted: false,
                    updated_at_ms: 9_999,
                    deleted_at_ms: None,
                    title: recent_doc.title.clone(),
                    content_hash: canonical_document_hash(&recent_doc),
                    body_hash: canonical_document_body_hash(&recent_doc),
                    meta_hash: canonical_document_meta_hash(&recent_doc),
                    meta_updated_at_ms: 9_000,
                    document: Some(recent_doc),
                },
            ],
            folders: Vec::new(),
            studio_prefs: None,
        };

        session.enter_online_server_session(acct, pull);
        assert_eq!(session.notebooks.len(), 2);
        assert_eq!(
            session.notebooks[session.active_notebook].id, "nb-recent",
            "fresh cloud login must select most recently modified notebook for Recent badge"
        );
    }

    #[test]
    fn apply_remote_upsert_reconciles_timestamps_on_matching_hash_and_migrates_v2_manifest() {
        let dir = temp_dir("manifest_v2_reconcile");
        let acct = fake_account("u-reconcile", "rec@example.com", "reconcile");
        let lib_dir = papercraft_sync::account_library_dir(&dir.0, &acct.user.user_id);
        std::fs::create_dir_all(&lib_dir).unwrap();

        let mut legacy_manifest = papercraft_sync::SyncManifest {
            schema_version: 2,
            owner_id: Some(papercraft_sync::OwnerId::CloudUser(
                acct.user.user_id.clone(),
            )),
            last_server_cursor: 55,
            ..Default::default()
        };
        legacy_manifest.schema_version = 2;
        papercraft_sync::save_manifest(&lib_dir, &legacy_manifest).unwrap();
        let migrated = papercraft_sync::load_manifest(&lib_dir);
        assert_eq!(
            migrated.schema_version,
            papercraft_sync::types::SYNC_MANIFEST_SCHEMA_VERSION
        );
        assert_eq!(migrated.last_server_cursor, 0);

        let mut session = Session::open_web_cache(None);
        let mut local_doc = Document {
            id: "nb-sync-ts".to_string(),
            title: "Synced Notebook".to_string(),
            created_at_ms: 0,
            updated_at_ms: 9_999,
            meta_updated_at_ms: 9_999,
            ..Default::default()
        };
        local_doc.sync_active_to_pages();
        let shared_hash = canonical_document_hash(&local_doc);
        let shared_body = canonical_document_body_hash(&local_doc);
        let shared_meta = canonical_document_meta_hash(&local_doc);

        session.enter_online_server_session(
            acct.clone(),
            SyncPullResponse {
                server_cursor: 1,
                user: acct.user.clone(),
                notebooks: vec![RemoteNotebookRecord {
                    notebook_id: "nb-sync-ts".to_string(),
                    version: 1,
                    cursor: 1,
                    deleted: false,
                    updated_at_ms: 9_999,
                    deleted_at_ms: None,
                    title: local_doc.title.clone(),
                    content_hash: shared_hash.clone(),
                    body_hash: shared_body.clone(),
                    meta_hash: shared_meta.clone(),
                    meta_updated_at_ms: 9_999,
                    document: Some(local_doc),
                }],
                folders: Vec::new(),
                studio_prefs: None,
            },
        );
        assert_eq!(session.notebooks[0].updated_at_ms, 9_999);

        let mut remote_authoritative = (*session.notebooks[0]).clone();
        remote_authoritative.created_at_ms = 1_500;
        remote_authoritative.updated_at_ms = 3_000;
        remote_authoritative.meta_updated_at_ms = 3_000;
        remote_authoritative.sync_active_to_pages();

        let outcome = SyncCycleOutcome {
            new_cursor: 2,
            refreshed_session: None,
            server_user: acct.user,
            accepted_uploads: Vec::new(),
            accepted_page_hashes: std::collections::HashMap::new(),
            accepted_page_elements: std::collections::HashMap::new(),
            rebased_docs: std::collections::HashMap::new(),
            rebased_ancestors: std::collections::HashMap::new(),
            accepted_folders: Vec::new(),
            cleared_tombstones: Vec::new(),
            pulled_count: 1,
            downloaded_updates: vec![papercraft_sync::RemoteNotebookUpdate {
                notebook_id: "nb-sync-ts".to_string(),
                version: 2,
                cursor: 2,
                deleted: false,
                updated_at_ms: 3_000,
                deleted_at_ms: None,
                content_hash: shared_hash,
                body_hash: shared_body,
                meta_hash: shared_meta,
                meta_updated_at_ms: 3_000,
                document: Some(remote_authoritative),
                page_hashes: Vec::new(),
            }],
            conflict_updates: Vec::new(),
            downloaded_folders: Vec::new(),
            updated_prefs: None,
            ack_prefs: None,
            pushed_count: 0,
            synced_at_ms: 4_000,
        };
        session.apply_sync_outcome(outcome).unwrap();
        assert_eq!(session.notebooks[0].created_at_ms, 1_500);
        assert_eq!(session.notebooks[0].updated_at_ms, 3_000);
        assert_eq!(session.cloud_pending_changes_count(), 0);
    }

    #[test]
    fn apply_sync_outcome_resolves_prefs_conflict_without_deadlock() {
        let mut session = Session::open_web_cache(None);
        let acct = fake_account("u-prefs", "prefs@example.com", "prefs");
        let initial_prefs = papercraft_doc::UserStudioConfig {
            library_sort_by: papercraft_doc::LibrarySortBy::ModifiedDate,
            library_sort_ascending: false,
            ..Default::default()
        };
        let initial_hash = papercraft_sync_protocol::canonical_prefs_hash(&initial_prefs);

        session.enter_online_server_session(
            acct.clone(),
            SyncPullResponse {
                server_cursor: 1,
                user: acct.user.clone(),
                notebooks: Vec::new(),
                folders: Vec::new(),
                studio_prefs: Some(papercraft_sync_protocol::RemotePrefsRecord {
                    version: 1,
                    updated_at_ms: 1_000,
                    content_hash: initial_hash,
                    prefs: initial_prefs,
                }),
            },
        );

        // User edits sort preference locally, so prefs becomes dirty at base_version 1.
        session
            .execute(
                "library.setViewPrefs",
                &json!({ "sort_by": "name", "sort_ascending": true }),
            )
            .unwrap();
        let batch_before = session.prepare_sync_batch().unwrap();
        assert_eq!(batch_before.prefs_push.as_ref().unwrap().base_version, 1);

        // Another device already pushed prefs version 2 on server; server returns prefs_conflict.
        let mut remote_v2 = papercraft_doc::UserStudioConfig::default();
        remote_v2.library_folders.push("Work".to_string());
        let remote_v2_hash = papercraft_sync_protocol::canonical_prefs_hash(&remote_v2);

        let conflict_outcome = SyncCycleOutcome {
            new_cursor: 2,
            refreshed_session: None,
            server_user: acct.user,
            accepted_uploads: Vec::new(),
            accepted_page_hashes: std::collections::HashMap::new(),
            accepted_page_elements: std::collections::HashMap::new(),
            rebased_docs: std::collections::HashMap::new(),
            rebased_ancestors: std::collections::HashMap::new(),
            accepted_folders: Vec::new(),
            cleared_tombstones: Vec::new(),
            pulled_count: 0,
            downloaded_updates: Vec::new(),
            conflict_updates: Vec::new(),
            downloaded_folders: Vec::new(),
            updated_prefs: Some(papercraft_sync_protocol::RemotePrefsRecord {
                version: 2,
                updated_at_ms: 2_000,
                content_hash: remote_v2_hash,
                prefs: remote_v2,
            }),
            ack_prefs: None,
            pushed_count: 0,
            synced_at_ms: 2_100,
        };
        session.apply_sync_outcome(conflict_outcome).unwrap();

        // Scalar edit preserved, folder merged, and base version rebased to 2 so next push succeeds.
        assert_eq!(
            session.prefs.library_sort_by,
            papercraft_doc::LibrarySortBy::Name
        );
        assert!(session.prefs.library_sort_ascending);
        assert!(session.prefs.library_folders.iter().any(|f| f == "Work"));
        let batch_after = session.prepare_sync_batch().unwrap();
        assert_eq!(batch_after.prefs_push.as_ref().unwrap().base_version, 2);
    }

    #[test]
    fn desktop_reopen_sorts_by_doc_updated_at_and_restores_active_notebook_id() {
        let dir = temp_dir("desktop_active_nb_id");
        let mut session = Session::open_desktop_library(dir.0.clone());
        session
            .execute(
                "profile.completeFirstAccess",
                &json!({ "nickname": "Alex" }),
            )
            .unwrap();

        session
            .execute("notebook.create", &json!({ "title": "Older Active" }))
            .unwrap();
        let older_id = session.notebooks[session.active_notebook].id.clone();

        session
            .execute("notebook.create", &json!({ "title": "Newer Inactive" }))
            .unwrap();
        let newer_id = session.notebooks[session.active_notebook].id.clone();
        assert_eq!(session.notebooks.len(), 2);

        // Give the two notebooks distinct document timestamps without relying on filesystem mtime.
        for nb in &mut session.notebooks {
            let mut updated = (**nb).clone();
            if updated.id == older_id {
                updated.created_at_ms = 1_000;
                updated.updated_at_ms = 2_000;
                updated.meta_updated_at_ms = 2_000;
            } else {
                updated.created_at_ms = 3_000;
                updated.updated_at_ms = 9_000;
                updated.meta_updated_at_ms = 9_000;
            }
            updated.sync_active_to_pages();
            *nb = Arc::new(updated);
        }
        session.doc = Arc::clone(&session.notebooks[session.active_notebook]);
        session.library.mark_dirty(0, 2);
        session.library.mark_dirty(1, 2);

        let older_idx = session
            .notebooks
            .iter()
            .position(|d| d.id == older_id)
            .unwrap();
        session
            .execute("notebook.select", &json!({ "index": older_idx }))
            .unwrap();
        session.flush_active_workspace_to_storage();

        let reopened = Session::open_desktop_library(dir.0.clone());
        assert_eq!(reopened.notebooks.len(), 2);
        assert_eq!(
            reopened.notebooks[0].id, newer_id,
            "disk load should order primarily by document updated_at_ms"
        );
        assert_eq!(
            reopened.notebooks[reopened.active_notebook].id, older_id,
            "reopening must restore the persisted active_notebook_id for the Recent badge"
        );
    }
}
