use crate::merge::{DocumentMergeBase, PageElementFingerprints};
use papercraft_doc::Document;
use papercraft_sync_protocol::{
    AcceptedFolderSync, AcceptedNotebookSync, DEFAULT_SERVER_URL, LocalFolderChange,
    LocalPrefsPush, ProtocolError, RemoteFolderRecord, RemotePrefsRecord, UserProfile,
    normalize_server_url, now_unix_ms,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const GLOBAL_SYNC_FILE: &str = "active_session.json";
pub const SYNC_MANIFEST_FILE: &str = "sync_manifest.json";
pub const ACCOUNTS_DIR: &str = "accounts";
pub const PROFILES_DIR: &str = "profiles";
pub const MAX_PROFILE_NICKNAME_LEN: usize = 48;
pub const MAX_REMEMBERED_SERVER_ACCOUNTS: usize = 32;
pub const SYNC_MANIFEST_SCHEMA_VERSION: u32 = 3;

/// Strict ownership boundary for local profiles and Offline-First server accounts.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id")]
pub enum OwnerId {
    LocalProfile(String),
    CloudUser(String),
}

impl OwnerId {
    pub fn as_key(&self) -> String {
        match self {
            Self::LocalProfile(id) => format!("local:{id}"),
            Self::CloudUser(id) => format!("cloud:{id}"),
        }
    }

    pub fn is_cloud(&self) -> bool {
        matches!(self, Self::CloudUser(_))
    }
}

/// Client-side local workspace profile stored exclusively on this device.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocalProfile {
    pub local_profile_id: String,
    pub nickname: String,
    pub created_at_ms: i64,
    #[serde(default)]
    pub last_used_at_ms: i64,
}

/// Device-local metadata shortcut for a server account stored alongside Offline-First caches.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RememberedServerAccount {
    pub user_id: String,
    pub server_url: String,
    pub username: String,
    #[serde(default)]
    pub email: Option<String>,
    pub display_name: String,
    #[serde(default)]
    pub last_used_at_ms: i64,
}

impl RememberedServerAccount {
    pub fn primary_label(&self) -> String {
        let disp = self.display_name.trim();
        if !disp.is_empty() {
            return disp.to_string();
        }
        if let Some(ref email) = self.email
            && !email.trim().is_empty()
        {
            return email.trim().to_string();
        }
        format!("@{}", self.username)
    }

    pub fn login_identifier(&self) -> &str {
        if let Some(ref email) = self.email
            && !email.trim().is_empty()
        {
            return email.trim();
        }
        &self.username
    }

    /// Returns the account's normalized non-empty email for email-only authentication.
    pub fn login_email(&self) -> Option<&str> {
        self.email
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
    }
}

/// Explicit per-entity live status surfaced in UI badges.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum EntitySyncStatus {
    #[default]
    LocalOnly,
    PendingPush,
    Synced,
    SyncError,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActiveAccountSession {
    pub server_url: String,
    pub user: UserProfile,
    pub access_token: String,
    pub refresh_token: String,
    pub access_expires_at_ms: i64,
    pub refresh_expires_at_ms: i64,
    pub device_name: String,
    pub last_auth_ms: i64,
}

/// Identifies which Offline-First workspace is currently active on this device.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActiveWorkspaceTarget {
    #[default]
    Local,
    Cloud {
        server_url: String,
        user_id: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalSyncState {
    pub server_url: String,
    pub device_name: String,
    /// Legacy persisted session field retained deserialize-only so upgrades migrate it into
    /// `cached_server_sessions` and `remembered_server_accounts`.
    #[serde(default, skip_serializing, rename = "session")]
    pub legacy_session: Option<ActiveAccountSession>,
    #[serde(default)]
    pub cached_server_sessions: Vec<ActiveAccountSession>,
    #[serde(default)]
    pub active_workspace: ActiveWorkspaceTarget,
    #[serde(default)]
    pub remembered_server_accounts: Vec<RememberedServerAccount>,
    #[serde(default)]
    pub local_profiles: Vec<LocalProfile>,
    #[serde(default)]
    pub active_local_profile_id: String,
    #[serde(default)]
    pub needs_initial_nickname: bool,
    #[serde(default)]
    pub welcome_banner_seen: bool,
    #[serde(default)]
    pub welcome_banner_seen_version: Option<String>,
    #[serde(default)]
    pub first_notebook_data_notice_seen: bool,
    #[serde(default)]
    pub skip_account_picker_on_startup: bool,
    #[serde(default)]
    pub client_id: String,
}

impl GlobalSyncState {
    pub fn with_defaults(device_name: String) -> Self {
        Self {
            server_url: DEFAULT_SERVER_URL.to_string(),
            device_name,
            legacy_session: None,
            cached_server_sessions: Vec::new(),
            active_workspace: ActiveWorkspaceTarget::Local,
            remembered_server_accounts: Vec::new(),
            local_profiles: Vec::new(),
            active_local_profile_id: String::new(),
            needs_initial_nickname: false,
            welcome_banner_seen: false,
            welcome_banner_seen_version: None,
            first_notebook_data_notice_seen: false,
            skip_account_picker_on_startup: false,
            client_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    /// Migrates any legacy single-session field into Offline-First cached sessions and shortcuts.
    pub fn migrate_legacy_session(&mut self) -> bool {
        let Some(old) = self.legacy_session.take() else {
            return false;
        };
        if !old.user.user_id.trim().is_empty() && !old.user.username.trim().is_empty() {
            self.remember_server_account(&old);
            self.upsert_cached_server_session(old);
        }
        true
    }

    /// Saves or refreshes an authenticated server session on this device for Offline-First access.
    pub fn upsert_cached_server_session(&mut self, mut auth: ActiveAccountSession) {
        let norm_url =
            normalize_server_url(&auth.server_url).unwrap_or_else(|_| auth.server_url.clone());
        let user_id = auth.user.user_id.trim().to_string();
        let username = auth.user.username.trim().to_string();
        if user_id.is_empty() || username.is_empty() {
            return;
        }
        auth.server_url = norm_url.clone();
        auth.user.user_id = user_id.clone();
        auth.user.username = username;
        self.cached_server_sessions.retain(|entry| {
            !(entry.user.user_id == user_id && entry.server_url.eq_ignore_ascii_case(&norm_url))
        });
        self.cached_server_sessions.insert(0, auth);
        self.cached_server_sessions
            .truncate(MAX_REMEMBERED_SERVER_ACCOUNTS);
    }

    /// Looks up a previously signed-in server session cached on this device.
    pub fn cached_server_session(
        &self,
        server_url: &str,
        user_id: &str,
    ) -> Option<&ActiveAccountSession> {
        let norm_url =
            normalize_server_url(server_url).unwrap_or_else(|_| server_url.trim().to_string());
        let clean_uid = user_id.trim();
        self.cached_server_sessions.iter().find(|entry| {
            entry.user.user_id == clean_uid && entry.server_url.eq_ignore_ascii_case(&norm_url)
        })
    }

    /// Returns `true` when `(server_url, user_id)` has a cached local workspace session on this device.
    pub fn has_cached_server_session(&self, server_url: &str, user_id: &str) -> bool {
        self.cached_server_session(server_url, user_id).is_some()
    }

    /// Removes the cached session tokens for `(server_url, user_id)` when signing out explicitly or forgetting.
    pub fn remove_cached_server_session(
        &mut self,
        server_url: &str,
        user_id: &str,
    ) -> Option<ActiveAccountSession> {
        let norm_url =
            normalize_server_url(server_url).unwrap_or_else(|_| server_url.trim().to_string());
        let clean_uid = user_id.trim();
        let pos = self.cached_server_sessions.iter().position(|entry| {
            entry.user.user_id == clean_uid && entry.server_url.eq_ignore_ascii_case(&norm_url)
        })?;
        Some(self.cached_server_sessions.remove(pos))
    }

    /// Records or refreshes a server account shortcut alongside its Offline-First workspace cache.
    pub fn remember_server_account(&mut self, auth: &ActiveAccountSession) {
        let norm_url =
            normalize_server_url(&auth.server_url).unwrap_or_else(|_| auth.server_url.clone());
        let user_id = auth.user.user_id.trim().to_string();
        let username = auth.user.username.trim().to_string();
        if user_id.is_empty() || username.is_empty() {
            return;
        }
        let email = auth
            .user
            .email
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let display_name = auth.user.display_name.trim().to_string();
        self.remembered_server_accounts.retain(|entry| {
            !(entry.user_id == user_id && entry.server_url.eq_ignore_ascii_case(&norm_url))
        });
        self.remembered_server_accounts.insert(
            0,
            RememberedServerAccount {
                user_id,
                server_url: norm_url,
                username,
                email,
                display_name,
                last_used_at_ms: now_unix_ms(),
            },
        );
        self.remembered_server_accounts
            .truncate(MAX_REMEMBERED_SERVER_ACCOUNTS);
    }

    /// Removes the local shortcut and cached session for `(server_url, user_id)` without altering server data.
    pub fn forget_server_account(&mut self, server_url: &str, user_id: &str) -> bool {
        let norm_url =
            normalize_server_url(server_url).unwrap_or_else(|_| server_url.trim().to_string());
        let clean_uid = user_id.trim();
        let before = self.remembered_server_accounts.len();
        self.remembered_server_accounts.retain(|entry| {
            !(entry.user_id == clean_uid && entry.server_url.eq_ignore_ascii_case(&norm_url))
        });
        let removed_cached = self
            .remove_cached_server_session(&norm_url, clean_uid)
            .is_some();
        if let ActiveWorkspaceTarget::Cloud {
            server_url: ref cur_url,
            user_id: ref cur_uid,
        } = self.active_workspace
            && cur_uid == clean_uid
            && cur_url.eq_ignore_ascii_case(&norm_url)
        {
            self.active_workspace = ActiveWorkspaceTarget::Local;
        }
        self.remembered_server_accounts.len() != before || removed_cached
    }

    pub fn has_saved_local_accounts(&self) -> bool {
        !self.needs_initial_nickname && !self.local_profiles.is_empty()
    }

    pub fn saved_local_profiles(&self) -> &[LocalProfile] {
        if self.needs_initial_nickname {
            &[]
        } else {
            &self.local_profiles
        }
    }

    pub fn accessible_local_profiles(&self) -> Vec<&LocalProfile> {
        self.saved_local_profiles().iter().collect()
    }

    pub fn active_local_profile(&self) -> Option<&LocalProfile> {
        self.local_profiles
            .iter()
            .find(|p| p.local_profile_id == self.active_local_profile_id)
            .or_else(|| self.local_profiles.first())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotebookSyncMeta {
    pub notebook_id: String,
    #[serde(default)]
    pub owner_id: Option<OwnerId>,
    #[serde(default)]
    pub status: EntitySyncStatus,
    #[serde(default)]
    pub local_file: Option<String>,
    pub server_version: u64,
    pub last_synced_hash: String,
    #[serde(default)]
    pub last_synced_body_hash: String,
    #[serde(default)]
    pub last_synced_meta_hash: String,
    #[serde(default)]
    pub last_synced_page_hashes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub last_synced_page_element_hashes: Vec<PageElementFingerprints>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_push_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_push_page_hashes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_push_page_elements: Vec<PageElementFingerprints>,
    pub cloud_dirty: bool,
    pub last_synced_at_ms: i64,
    #[serde(default)]
    pub updated_at_ms: i64,
    #[serde(default)]
    pub deleted_at_ms: Option<i64>,
    #[serde(default)]
    pub last_error: Option<String>,
}

impl NotebookSyncMeta {
    pub fn effective_status(&self, is_cloud: bool) -> EntitySyncStatus {
        if !is_cloud {
            return EntitySyncStatus::LocalOnly;
        }
        if self.status == EntitySyncStatus::SyncError && self.last_error.is_some() {
            return EntitySyncStatus::SyncError;
        }
        if self.cloud_dirty
            || self.server_version == 0
            || self.status == EntitySyncStatus::PendingPush
        {
            return EntitySyncStatus::PendingPush;
        }
        EntitySyncStatus::Synced
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderSyncMeta {
    pub folder_id: String,
    #[serde(default)]
    pub owner_id: Option<OwnerId>,
    pub path: String,
    #[serde(default)]
    pub color_hex: Option<String>,
    #[serde(default)]
    pub status: EntitySyncStatus,
    pub server_version: u64,
    pub cloud_dirty: bool,
    pub updated_at_ms: i64,
    #[serde(default)]
    pub deleted_at_ms: Option<i64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotebookTombstone {
    pub notebook_id: String,
    pub base_version: u64,
    pub deleted_at_ms: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncManifest {
    pub schema_version: u32,
    #[serde(default)]
    pub owner_id: Option<OwnerId>,
    pub last_server_cursor: u64,
    pub prefs_version: u64,
    #[serde(default)]
    pub prefs_hash: String,
    #[serde(default)]
    pub prefs_dirty: bool,
    #[serde(default)]
    pub notebooks: Vec<NotebookSyncMeta>,
    #[serde(default)]
    pub folders: Vec<FolderSyncMeta>,
    #[serde(default)]
    pub tombstones: Vec<NotebookTombstone>,
    #[serde(default)]
    pub last_sync_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub active_notebook_id: String,
}

impl Default for SyncManifest {
    fn default() -> Self {
        Self {
            schema_version: SYNC_MANIFEST_SCHEMA_VERSION,
            owner_id: None,
            last_server_cursor: 0,
            prefs_version: 0,
            prefs_hash: String::new(),
            prefs_dirty: false,
            notebooks: Vec::new(),
            folders: Vec::new(),
            tombstones: Vec::new(),
            last_sync_at_ms: None,
            active_notebook_id: String::new(),
        }
    }
}

impl SyncManifest {
    /// Upgrades older persisted manifests so cloud accounts trigger one full-pull
    /// reconciliation pass to backfill timestamps across devices.
    pub fn migrate_if_needed(&mut self) -> bool {
        if self.schema_version >= SYNC_MANIFEST_SCHEMA_VERSION {
            return false;
        }
        if self.owner_id.as_ref().is_some_and(OwnerId::is_cloud) {
            self.last_server_cursor = 0;
        }
        self.schema_version = SYNC_MANIFEST_SCHEMA_VERSION;
        true
    }
    pub fn meta(&self, notebook_id: &str) -> Option<&NotebookSyncMeta> {
        self.notebooks
            .iter()
            .find(|item| item.notebook_id == notebook_id)
    }

    pub fn id_for_local_file(&self, file_name: &str) -> Option<&str> {
        self.notebooks
            .iter()
            .find(|item| item.local_file.as_deref() == Some(file_name))
            .map(|item| item.notebook_id.as_str())
    }

    pub fn bind_local_file(&mut self, notebook_id: &str, file_name: &str) {
        if let Some(existing) = self
            .notebooks
            .iter_mut()
            .find(|item| item.notebook_id == notebook_id)
        {
            existing.local_file = Some(file_name.to_string());
        }
    }

    pub fn ensure_notebook(&mut self, notebook_id: &str, current_hash: &str) {
        self.mark_notebook_dirty(notebook_id, current_hash);
    }

    pub fn mark_notebook_dirty(&mut self, notebook_id: &str, current_hash: &str) {
        let is_cloud = self.owner_id.as_ref().is_some_and(OwnerId::is_cloud);
        let now = now_unix_ms();
        if let Some(existing) = self
            .notebooks
            .iter_mut()
            .find(|item| item.notebook_id == notebook_id)
        {
            let dirty = existing.server_version == 0
                || current_hash.is_empty()
                || existing.last_synced_hash != current_hash;
            existing.cloud_dirty = dirty;
            existing.updated_at_ms = existing.updated_at_ms.max(now);
            existing.last_error = None;
            existing.status = if !is_cloud {
                EntitySyncStatus::LocalOnly
            } else if dirty {
                EntitySyncStatus::PendingPush
            } else {
                EntitySyncStatus::Synced
            };
        } else {
            self.notebooks.push(NotebookSyncMeta {
                notebook_id: notebook_id.to_string(),
                owner_id: self.owner_id.clone(),
                status: if is_cloud {
                    EntitySyncStatus::PendingPush
                } else {
                    EntitySyncStatus::LocalOnly
                },
                local_file: None,
                server_version: 0,
                last_synced_hash: String::new(),
                last_synced_body_hash: String::new(),
                last_synced_meta_hash: String::new(),
                last_synced_page_hashes: Vec::new(),
                last_synced_page_element_hashes: Vec::new(),
                pending_push_hash: None,
                pending_push_page_hashes: Vec::new(),
                pending_push_page_elements: Vec::new(),
                cloud_dirty: true,
                last_synced_at_ms: 0,
                updated_at_ms: now,
                deleted_at_ms: None,
                last_error: None,
            });
        }
    }

    pub fn record_local_delete(&mut self, notebook_id: &str, deleted_at_ms: i64) {
        let existing_version = self
            .notebooks
            .iter()
            .find(|item| item.notebook_id == notebook_id)
            .map_or(0, |item| item.server_version);
        self.notebooks
            .retain(|item| item.notebook_id != notebook_id);

        if existing_version == 0 {
            return;
        }
        if let Some(tombstone) = self
            .tombstones
            .iter_mut()
            .find(|item| item.notebook_id == notebook_id)
        {
            tombstone.base_version = tombstone.base_version.max(existing_version);
            tombstone.deleted_at_ms = deleted_at_ms;
        } else {
            self.tombstones.push(NotebookTombstone {
                notebook_id: notebook_id.to_string(),
                base_version: existing_version,
                deleted_at_ms,
            });
        }
    }

    pub fn mark_prefs_dirty(&mut self, current_hash: &str) {
        self.prefs_dirty = self.prefs_version == 0 || self.prefs_hash != current_hash;
    }

    pub fn mark_notebook_synced(
        &mut self,
        notebook_id: &str,
        new_version: u64,
        synced_hash: String,
        current_hash: &str,
        synced_at_ms: i64,
    ) {
        let still_dirty = current_hash != synced_hash;
        self.mark_notebook_synced_with_dirty(
            notebook_id,
            new_version,
            synced_hash,
            still_dirty,
            synced_at_ms,
        );
    }

    pub fn mark_notebook_synced_with_dirty(
        &mut self,
        notebook_id: &str,
        new_version: u64,
        synced_hash: String,
        still_dirty: bool,
        synced_at_ms: i64,
    ) {
        let is_cloud = self.owner_id.as_ref().is_some_and(OwnerId::is_cloud);
        let next_status = if !is_cloud {
            EntitySyncStatus::LocalOnly
        } else if still_dirty {
            EntitySyncStatus::PendingPush
        } else {
            EntitySyncStatus::Synced
        };
        if let Some(meta) = self
            .notebooks
            .iter_mut()
            .find(|item| item.notebook_id == notebook_id)
        {
            meta.owner_id = self.owner_id.clone();
            meta.server_version = new_version;
            meta.last_synced_hash = synced_hash;
            meta.cloud_dirty = still_dirty;
            meta.last_synced_at_ms = synced_at_ms;
            meta.deleted_at_ms = None;
            meta.last_error = None;
            meta.status = next_status;
        } else {
            self.notebooks.push(NotebookSyncMeta {
                notebook_id: notebook_id.to_string(),
                owner_id: self.owner_id.clone(),
                status: next_status,
                local_file: None,
                server_version: new_version,
                last_synced_hash: synced_hash,
                last_synced_body_hash: String::new(),
                last_synced_meta_hash: String::new(),
                last_synced_page_hashes: Vec::new(),
                last_synced_page_element_hashes: Vec::new(),
                pending_push_hash: None,
                pending_push_page_hashes: Vec::new(),
                pending_push_page_elements: Vec::new(),
                cloud_dirty: still_dirty,
                last_synced_at_ms: synced_at_ms,
                updated_at_ms: synced_at_ms,
                deleted_at_ms: None,
                last_error: None,
            });
        }
    }

    /// Updates the stored per-page base hashes used for 3-way conflict resolution.
    pub fn set_notebook_synced_page_hashes(&mut self, notebook_id: &str, page_hashes: Vec<String>) {
        if let Some(meta) = self
            .notebooks
            .iter_mut()
            .find(|item| item.notebook_id == notebook_id)
        {
            meta.last_synced_page_hashes = page_hashes;
        }
    }

    /// Updates the stored per-page element fingerprints used for element-level 3-way merges.
    pub fn set_notebook_synced_page_elements(
        &mut self,
        notebook_id: &str,
        page_elements: Vec<PageElementFingerprints>,
    ) {
        if let Some(meta) = self
            .notebooks
            .iter_mut()
            .find(|item| item.notebook_id == notebook_id)
        {
            meta.last_synced_page_element_hashes = page_elements;
        }
    }

    /// Records the exact snapshot fingerprints being uploaded so an interrupted HTTP ACK can
    /// recognize and rebase cleanly onto our own prior push on retry.
    pub fn record_pending_push_base(
        &mut self,
        notebook_id: &str,
        content_hash: String,
        page_hashes: Vec<String>,
        page_elements: Vec<PageElementFingerprints>,
    ) {
        if let Some(meta) = self
            .notebooks
            .iter_mut()
            .find(|item| item.notebook_id == notebook_id)
        {
            meta.pending_push_hash = Some(content_hash);
            meta.pending_push_page_hashes = page_hashes;
            meta.pending_push_page_elements = page_elements;
        }
    }

    /// Clears recorded pending-push fingerprints once the upload is acknowledged or superseded.
    pub fn clear_pending_push_base(&mut self, notebook_id: &str) {
        if let Some(meta) = self
            .notebooks
            .iter_mut()
            .find(|item| item.notebook_id == notebook_id)
        {
            meta.pending_push_hash = None;
            meta.pending_push_page_hashes.clear();
            meta.pending_push_page_elements.clear();
        }
    }

    pub fn reassign_owner(&mut self, next_owner: OwnerId, mark_pending: bool) {
        let is_cloud = next_owner.is_cloud();
        self.owner_id = Some(next_owner.clone());
        for nb in &mut self.notebooks {
            nb.owner_id = Some(next_owner.clone());
            if mark_pending && is_cloud {
                nb.server_version = 0;
                nb.last_synced_hash.clear();
                nb.last_synced_body_hash.clear();
                nb.last_synced_meta_hash.clear();
                nb.last_synced_page_hashes.clear();
                nb.last_synced_page_element_hashes.clear();
                nb.pending_push_hash = None;
                nb.pending_push_page_hashes.clear();
                nb.pending_push_page_elements.clear();
                nb.cloud_dirty = true;
                nb.last_synced_at_ms = 0;
                nb.status = EntitySyncStatus::PendingPush;
                nb.last_error = None;
            } else if !is_cloud {
                nb.status = EntitySyncStatus::LocalOnly;
            }
        }
        for folder in &mut self.folders {
            folder.owner_id = Some(next_owner.clone());
            if mark_pending && is_cloud {
                folder.server_version = 0;
                folder.cloud_dirty = true;
                folder.status = EntitySyncStatus::PendingPush;
            } else if !is_cloud {
                folder.status = EntitySyncStatus::LocalOnly;
            }
        }
        if mark_pending && is_cloud {
            self.last_server_cursor = 0;
            self.prefs_version = 0;
            self.prefs_hash.clear();
            self.prefs_dirty = true;
        }
    }

    pub fn clear_tombstone(&mut self, notebook_id: &str) {
        self.tombstones
            .retain(|item| item.notebook_id != notebook_id);
    }

    pub fn pending_changes_count(&self) -> usize {
        let nb_dirty = self
            .notebooks
            .iter()
            .filter(|m| m.cloud_dirty || m.server_version == 0)
            .count();
        let folder_dirty = self
            .folders
            .iter()
            .filter(|f| f.cloud_dirty || f.server_version == 0 || f.deleted_at_ms.is_some())
            .count();
        nb_dirty + folder_dirty + self.tombstones.len() + usize::from(self.prefs_dirty)
    }

    pub fn reset_for_fresh_sync(&mut self) {
        self.last_server_cursor = 0;
        self.last_sync_at_ms = Some(now_unix_ms());
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SyncConnectionState {
    #[default]
    SignedOut,
    Offline,
    Syncing,
    Synced,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncStatusSummary {
    pub signed_in: bool,
    pub viewing_cloud: bool,
    pub server_url: String,
    pub device_name: String,
    pub username: Option<String>,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub active_local_profile_id: Option<String>,
    pub active_local_profile_name: Option<String>,
    pub connection: SyncConnectionState,
    pub pending_changes: usize,
    pub last_sync_at_ms: Option<i64>,
    pub last_error: Option<String>,
}

impl SyncStatusSummary {
    pub fn workspace_badge_text(&self) -> String {
        if self.viewing_cloud {
            self.email
                .clone()
                .or_else(|| self.username.as_ref().map(|u| format!("@{u}")))
                .unwrap_or_else(|| "Cloud".to_string())
        } else {
            self.active_local_profile_name
                .clone()
                .unwrap_or_else(|| "Personal".to_string())
        }
    }
}

#[derive(Clone, Debug)]
pub struct LocalNotebookSnapshot {
    pub notebook_id: String,
    pub base_version: u64,
    pub cloud_dirty: bool,
    pub content_hash: String,
    pub doc: Document,
    pub merge_base: DocumentMergeBase,
}

#[derive(Clone, Debug)]
pub struct SyncBatch {
    pub client_cursor: u64,
    pub device_name: String,
    pub local_docs: Vec<LocalNotebookSnapshot>,
    pub tombstones: Vec<NotebookTombstone>,
    pub folder_changes: Vec<LocalFolderChange>,
    pub prefs_push: Option<LocalPrefsPush>,
    pub force_overwrite: bool,
}

#[derive(Clone, Debug)]
pub struct RemoteNotebookUpdate {
    pub notebook_id: String,
    pub version: u64,
    pub cursor: u64,
    pub deleted: bool,
    pub updated_at_ms: i64,
    pub deleted_at_ms: Option<i64>,
    pub content_hash: String,
    pub body_hash: String,
    pub meta_hash: String,
    pub meta_updated_at_ms: i64,
    pub document: Option<Document>,
    pub page_hashes: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct SyncCycleOutcome {
    pub new_cursor: u64,
    pub refreshed_session: Option<ActiveAccountSession>,
    pub server_user: UserProfile,
    pub accepted_uploads: Vec<AcceptedNotebookSync>,
    pub accepted_page_hashes: std::collections::HashMap<String, Vec<String>>,
    pub accepted_page_elements: std::collections::HashMap<String, Vec<PageElementFingerprints>>,
    pub rebased_docs: std::collections::HashMap<String, Document>,
    pub rebased_ancestors: std::collections::HashMap<String, DocumentMergeBase>,
    pub accepted_folders: Vec<AcceptedFolderSync>,
    pub cleared_tombstones: Vec<String>,
    pub downloaded_updates: Vec<RemoteNotebookUpdate>,
    pub conflict_updates: Vec<RemoteNotebookUpdate>,
    pub downloaded_folders: Vec<RemoteFolderRecord>,
    pub updated_prefs: Option<RemotePrefsRecord>,
    pub ack_prefs: Option<(u64, String)>,
    pub pushed_count: usize,
    pub pulled_count: usize,
    pub synced_at_ms: i64,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum SyncError {
    #[error("offline or unreachable server: {0}")]
    Network(String),
    #[error("session expired or unauthorized: {0}")]
    Unauthorized(String),
    #[error("server error ({status} {code}): {message}")]
    Server {
        status: u16,
        code: String,
        message: String,
    },
    #[error("{0}")]
    Protocol(#[from] ProtocolError),
    #[error("local storage error: {0}")]
    Io(String),
    #[error("invalid server response: {0}")]
    InvalidResponse(String),
}
