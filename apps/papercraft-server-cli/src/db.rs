use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;

use argon2::{
    Argon2,
    password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash},
};
use papercraft_doc::{Document, UserStudioConfig, sanitize_folder_path, sanitize_notebook_title};
use papercraft_format::{load_document, load_document_bytes, save_document_bytes};
use papercraft_sync_protocol::{
    AcceptedFolderSync, AcceptedNotebookSync, AuthResponse, CheckEmailResponse,
    ConflictNotebookSync, LocalFolderChange, LocalNotebookChange, LocalPrefsPush,
    MAX_PUSH_BATCH_SIZE, ProtocolError, PruneToLocalRequest, PruneToLocalResponse,
    RemoteFolderRecord, RemoteNotebookRecord, RemotePrefsRecord, SyncPullResponse, SyncPushRequest,
    SyncPushResponse, UserProfile, bytes_to_hex, canonical_document_body_hash,
    canonical_document_hash, canonical_document_meta_hash, canonical_prefs_hash,
    derive_username_from_email, normalize_email, normalize_username, now_unix_ms,
    sanitize_device_name, sanitize_display_name, sha256_hex, validate_folder_id,
    validate_notebook_id, validate_password,
};
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

const USERS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("users");
const USERS_BY_NAME_TABLE: TableDefinition<&str, &str> = TableDefinition::new("users_by_name");
const USERS_BY_EMAIL_TABLE: TableDefinition<&str, &str> = TableDefinition::new("users_by_email");
const TRASHED_USERS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("trashed_users");
const ACCESS_TOKENS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("access_tokens");
const REFRESH_TOKENS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("refresh_tokens");
const NOTEBOOKS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("notebooks");
/// Compressed binary `.pcraft` v2 payloads keyed identically to `NOTEBOOKS_TABLE` rows.
const NOTEBOOK_BLOBS_TABLE: TableDefinition<&str, &[u8]> = TableDefinition::new("notebook_blobs");
const FOLDERS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("folders");
const USER_PREFS_TABLE: TableDefinition<&str, &str> = TableDefinition::new("user_prefs");
const USER_CURSORS_TABLE: TableDefinition<&str, u64> = TableDefinition::new("user_cursors");

pub const ACCOUNT_TRASH_RETENTION_DAYS: i64 = 30;
pub const ACCOUNT_TRASH_RETENTION_MS: i64 = ACCOUNT_TRASH_RETENTION_DAYS * 24 * 60 * 60 * 1000;
const MAX_TOKENS_PER_USER_PRUNE: usize = 64;

#[derive(Debug, Error)]
pub enum DbError {
    #[error("{0}")]
    Validation(String),
    #[error("registration is disabled on this server")]
    RegistrationClosed,
    #[error("username `{0}` is already registered")]
    UsernameTaken(String),
    #[error("email `{0}` is already registered")]
    EmailTaken(String),
    #[error("account is in server trash and scheduled for permanent deletion")]
    AccountInTrash,
    #[error("email `{0}` belongs to an account in server trash")]
    EmailInTrash(String),
    #[error("username `{0}` belongs to an account in server trash")]
    UsernameInTrash(String),
    #[error("trashed account `{0}` has expired and was permanently deleted")]
    TrashedUserExpired(String),
    #[error("invalid email or password")]
    InvalidCredentials,
    #[error("current password is incorrect")]
    IncorrectCurrentPassword,
    #[error("password reset required on next login")]
    PasswordResetRequired,
    #[error("temporary password verified; choose a new password")]
    PasswordChangeRequired,
    #[error("session token is missing, expired, or revoked")]
    Unauthorized,
    #[error("user `{0}` was not found")]
    UserNotFound(String),
    #[error(
        "notebook `{title}` payload ({actual_bytes} bytes) exceeds server limit ({limit_bytes} bytes)"
    )]
    PayloadTooLarge {
        title: String,
        actual_bytes: usize,
        limit_bytes: usize,
    },
    #[error("database failure: {0}")]
    Storage(String),
}

impl From<ProtocolError> for DbError {
    fn from(err: ProtocolError) -> Self {
        Self::Validation(err.to_string())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TokenTtls {
    pub access_ttl_ms: i64,
    pub refresh_ttl_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct UserRow {
    user_id: String,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    email_lower: Option<String>,
    username: String,
    username_lower: String,
    display_name: String,
    password_hash: String,
    created_at_ms: i64,
    updated_at_ms: i64,
    #[serde(default)]
    must_change_password: bool,
}

impl UserRow {
    fn to_profile(&self) -> UserProfile {
        UserProfile {
            user_id: self.user_id.clone(),
            email: self.email.clone(),
            username: self.username.clone(),
            display_name: self.display_name.clone(),
            created_at_ms: self.created_at_ms,
            must_change_password: self.must_change_password,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct AccessTokenRow {
    token_hash: String,
    user_id: String,
    created_at_ms: i64,
    expires_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct RefreshTokenRow {
    token_hash: String,
    user_id: String,
    device_name: String,
    created_at_ms: i64,
    expires_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredNotebookRow {
    notebook_id: String,
    user_id: String,
    version: u64,
    cursor: u64,
    deleted: bool,
    updated_at_ms: i64,
    #[serde(default)]
    deleted_at_ms: Option<i64>,
    title: String,
    content_hash: String,
    #[serde(default)]
    body_hash: String,
    #[serde(default)]
    meta_hash: String,
    #[serde(default)]
    meta_updated_at_ms: i64,
    /// Legacy inline JSON payload (pre-binary storage); new rows keep payloads in `NOTEBOOK_BLOBS_TABLE`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    ncraft_json: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredFolderRow {
    folder_id: String,
    user_id: String,
    path: String,
    #[serde(default)]
    color_hex: Option<String>,
    version: u64,
    cursor: u64,
    deleted: bool,
    updated_at_ms: i64,
    #[serde(default)]
    deleted_at_ms: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredPrefsRow {
    version: u64,
    updated_at_ms: i64,
    content_hash: String,
    prefs: UserStudioConfig,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TrashedUserRow {
    user: UserRow,
    deleted_at_ms: i64,
    expires_at_ms: i64,
}

fn is_trashed_user_expired(row: &TrashedUserRow, now_ms: i64) -> bool {
    now_ms >= row.expires_at_ms
        || now_ms.saturating_sub(row.deleted_at_ms) >= ACCOUNT_TRASH_RETENTION_MS
}

fn format_account_label(email: Option<&str>, username: &str) -> String {
    match email {
        Some(email) if !email.trim().is_empty() => email.to_string(),
        _ => format!("@{username}"),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbStats {
    pub user_count: usize,
    pub trashed_user_count: usize,
    pub active_notebook_count: usize,
    pub deleted_tombstone_count: usize,
    pub active_session_count: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserSummary {
    pub user_id: String,
    pub email: Option<String>,
    pub username: String,
    pub display_name: String,
    pub notebook_count: usize,
    pub created_at_ms: i64,
    pub must_change_password: bool,
}

/// Summary of a user account currently held in the 30-day server Trash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashedUserSummary {
    pub user_id: String,
    pub email: Option<String>,
    pub username: String,
    pub display_name: String,
    pub notebook_count: usize,
    pub created_at_ms: i64,
    pub deleted_at_ms: i64,
    pub expires_at_ms: i64,
}

impl TrashedUserSummary {
    pub fn account_label(&self) -> String {
        format_account_label(self.email.as_deref(), &self.username)
    }
}

/// Summary returned after moving an account into the 30-day server Trash.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrashedUserActionSummary {
    pub user_id: String,
    pub username: String,
    pub email: Option<String>,
    pub display_name: String,
    pub notebook_count: usize,
    pub deleted_at_ms: i64,
    pub expires_at_ms: i64,
}

impl TrashedUserActionSummary {
    pub fn account_label(&self) -> String {
        format_account_label(self.email.as_deref(), &self.username)
    }
}

/// Summary returned after restoring a trashed user account back to active status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RestoredUserSummary {
    pub user_id: String,
    pub username: String,
    pub email: Option<String>,
    pub display_name: String,
    pub restored_notebooks: usize,
}

impl RestoredUserSummary {
    pub fn account_label(&self) -> String {
        format_account_label(self.email.as_deref(), &self.username)
    }
}

/// Outcome returned when generating a one-time random temporary password for an account.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasswordResetOutcome {
    pub user_id: String,
    pub username: String,
    pub email: Option<String>,
    pub display_name: String,
    pub temporary_password: String,
}

impl PasswordResetOutcome {
    pub fn account_label(&self) -> String {
        format_account_label(self.email.as_deref(), &self.username)
    }
}

/// Summary returned after permanently deleting a user account and all of its synced data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeletedUserSummary {
    pub user_id: String,
    pub username: String,
    pub email: Option<String>,
    pub display_name: String,
    pub deleted_notebooks: usize,
}

impl DeletedUserSummary {
    pub fn account_label(&self) -> String {
        format_account_label(self.email.as_deref(), &self.username)
    }
}

#[derive(Clone)]
pub struct ServerDb {
    db: Arc<Database>,
}

impl ServerDb {
    pub fn open(db_path: &Path) -> Result<Self, DbError> {
        let db = Database::create(db_path).map_err(|e| {
            DbError::Storage(format!("failed to open `{}`: {e}", db_path.display()))
        })?;
        let tx = db
            .begin_write()
            .map_err(|e| DbError::Storage(format!("failed to begin init transaction: {e}")))?;
        {
            let _ = tx.open_table(USERS_TABLE).map_err(map_table_err)?;
            let _ = tx.open_table(USERS_BY_NAME_TABLE).map_err(map_table_err)?;
            let _ = tx.open_table(USERS_BY_EMAIL_TABLE).map_err(map_table_err)?;
            let _ = tx.open_table(TRASHED_USERS_TABLE).map_err(map_table_err)?;
            let _ = tx.open_table(ACCESS_TOKENS_TABLE).map_err(map_table_err)?;
            let _ = tx.open_table(REFRESH_TOKENS_TABLE).map_err(map_table_err)?;
            let _ = tx.open_table(NOTEBOOKS_TABLE).map_err(map_table_err)?;
            let _ = tx.open_table(NOTEBOOK_BLOBS_TABLE).map_err(map_table_err)?;
            let _ = tx.open_table(FOLDERS_TABLE).map_err(map_table_err)?;
            let _ = tx.open_table(USER_PREFS_TABLE).map_err(map_table_err)?;
            let _ = tx.open_table(USER_CURSORS_TABLE).map_err(map_table_err)?;
        }
        tx.commit().map_err(map_commit_err)?;
        let server_db = Self { db: Arc::new(db) };
        let _ = server_db.cleanup_expired_trashed_users_at(now_unix_ms());
        Ok(server_db)
    }

    pub fn register_user(
        &self,
        username_raw: &str,
        password: &str,
        display_name_raw: Option<&str>,
        device_name_raw: Option<&str>,
        registration_open: bool,
        ttls: TokenTtls,
    ) -> Result<AuthResponse, DbError> {
        self.register_user_with_email(
            None,
            username_raw,
            password,
            display_name_raw,
            device_name_raw,
            registration_open,
            ttls,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn register_user_with_email(
        &self,
        email_raw: Option<&str>,
        username_raw: &str,
        password: &str,
        display_name_raw: Option<&str>,
        device_name_raw: Option<&str>,
        registration_open: bool,
        ttls: TokenTtls,
    ) -> Result<AuthResponse, DbError> {
        if !registration_open {
            return Err(DbError::RegistrationClosed);
        }
        let email_pair = match email_raw.map(str::trim).filter(|s| !s.is_empty()) {
            Some(raw) => Some(normalize_email(raw)?),
            None if username_raw.contains('@') => Some(normalize_email(username_raw)?),
            None => None,
        };
        let explicit_username = username_raw.trim();
        let (base_username, require_unique_username) = match (&email_pair, explicit_username) {
            (Some((email_disp, _)), "") => (derive_username_from_email(email_disp), false),
            (Some((email_disp, _)), raw) if raw.contains('@') => {
                (derive_username_from_email(email_disp), false)
            }
            (_, raw) => {
                let (u, _) = normalize_username(raw)?;
                (u, true)
            }
        };
        validate_password(password)?;
        let fallback_display = email_pair
            .as_ref()
            .map_or(base_username.as_str(), |(e, _)| e.as_str());
        let display_name = sanitize_display_name(display_name_raw.unwrap_or(""), fallback_display);
        let device_name = sanitize_device_name(device_name_raw.unwrap_or(""));
        let password_hash = hash_password(password)?;

        let now = now_unix_ms();
        self.cleanup_expired_trashed_users_at(now)?;
        let user_id = Uuid::new_v4().to_string();

        let tx = self.db.begin_write().map_err(map_tx_err)?;
        let auth = {
            let mut by_email = tx.open_table(USERS_BY_EMAIL_TABLE).map_err(map_table_err)?;
            if let Some((ref email_disp, ref email_lower)) = email_pair
                && by_email
                    .get(email_lower.as_str())
                    .map_err(map_storage_err)?
                    .is_some()
            {
                return Err(DbError::EmailTaken(email_disp.clone()));
            }
            let trashed_table = tx.open_table(TRASHED_USERS_TABLE).map_err(map_table_err)?;
            let mut trashed_usernames = HashSet::new();
            for item in trashed_table.iter().map_err(map_storage_err)? {
                let (_, v) = item.map_err(map_storage_err)?;
                let trashed: TrashedUserRow = from_json(v.value())?;
                if is_trashed_user_expired(&trashed, now) {
                    continue;
                }
                if let Some((ref email_disp, ref email_lower)) = email_pair
                    && trashed.user.email_lower.as_deref() == Some(email_lower.as_str())
                {
                    return Err(DbError::EmailInTrash(email_disp.clone()));
                }
                trashed_usernames.insert(trashed.user.username_lower);
            }
            drop(trashed_table);

            let mut by_name = tx.open_table(USERS_BY_NAME_TABLE).map_err(map_table_err)?;
            let (username, username_lower) = allocate_unique_username(
                &by_name,
                &trashed_usernames,
                &base_username,
                &user_id,
                require_unique_username,
            )?;
            let user_row = UserRow {
                user_id: user_id.clone(),
                email: email_pair.as_ref().map(|(e, _)| e.clone()),
                email_lower: email_pair.as_ref().map(|(_, l)| l.clone()),
                username,
                username_lower: username_lower.clone(),
                display_name,
                password_hash,
                created_at_ms: now,
                updated_at_ms: now,
                must_change_password: false,
            };

            let mut users = tx.open_table(USERS_TABLE).map_err(map_table_err)?;
            let mut cursors = tx.open_table(USER_CURSORS_TABLE).map_err(map_table_err)?;
            let mut access = tx.open_table(ACCESS_TOKENS_TABLE).map_err(map_table_err)?;
            let mut refresh = tx.open_table(REFRESH_TOKENS_TABLE).map_err(map_table_err)?;

            let encoded_user = to_json(&user_row)?;
            users
                .insert(user_id.as_str(), encoded_user.as_str())
                .map_err(map_storage_err)?;
            by_name
                .insert(username_lower.as_str(), user_id.as_str())
                .map_err(map_storage_err)?;
            if let Some(ref email_lower) = user_row.email_lower {
                by_email
                    .insert(email_lower.as_str(), user_id.as_str())
                    .map_err(map_storage_err)?;
            }
            cursors
                .insert(user_id.as_str(), 0u64)
                .map_err(map_storage_err)?;

            issue_tokens(
                &user_row,
                &device_name,
                now,
                ttls,
                &mut access,
                &mut refresh,
            )?
        };
        tx.commit().map_err(map_commit_err)?;
        Ok(auth)
    }

    pub fn check_email_exists(
        &self,
        email_raw: &str,
        registration_open: bool,
    ) -> Result<CheckEmailResponse, DbError> {
        let (_, email_lower) = normalize_email(email_raw)?;
        let now = now_unix_ms();
        let rx = self.db.begin_read().map_err(map_tx_err)?;
        let by_email = rx.open_table(USERS_BY_EMAIL_TABLE).map_err(map_table_err)?;
        let mut exists = by_email
            .get(email_lower.as_str())
            .map_err(map_storage_err)?
            .is_some();
        if !exists {
            let trashed = rx.open_table(TRASHED_USERS_TABLE).map_err(map_table_err)?;
            for item in trashed.iter().map_err(map_storage_err)? {
                let (_, v) = item.map_err(map_storage_err)?;
                let row: TrashedUserRow = from_json(v.value())?;
                if !is_trashed_user_expired(&row, now)
                    && row.user.email_lower.as_deref() == Some(email_lower.as_str())
                {
                    exists = true;
                    break;
                }
            }
        }
        Ok(CheckEmailResponse {
            exists,
            registration_open,
        })
    }

    pub fn change_email(
        &self,
        user_id: &str,
        current_password: &str,
        new_email_raw: &str,
    ) -> Result<UserProfile, DbError> {
        let (new_email, new_lower) = normalize_email(new_email_raw)?;
        let now = now_unix_ms();
        self.cleanup_expired_trashed_users_at(now)?;
        let tx = self.db.begin_write().map_err(map_tx_err)?;
        let profile = {
            let mut users = tx.open_table(USERS_TABLE).map_err(map_table_err)?;
            let mut by_email = tx.open_table(USERS_BY_EMAIL_TABLE).map_err(map_table_err)?;
            let trashed_table = tx.open_table(TRASHED_USERS_TABLE).map_err(map_table_err)?;
            let Some(user_guard) = users.get(user_id).map_err(map_storage_err)? else {
                return Err(DbError::Unauthorized);
            };
            let mut row: UserRow = from_json(user_guard.value())?;
            drop(user_guard);

            verify_password(current_password, &row.password_hash)
                .map_err(|_| DbError::IncorrectCurrentPassword)?;
            if row.email_lower.as_deref() == Some(new_lower.as_str()) {
                return Ok(row.to_profile());
            }
            if by_email
                .get(new_lower.as_str())
                .map_err(map_storage_err)?
                .is_some()
            {
                return Err(DbError::EmailTaken(new_email));
            }
            for item in trashed_table.iter().map_err(map_storage_err)? {
                let (_, v) = item.map_err(map_storage_err)?;
                let trashed: TrashedUserRow = from_json(v.value())?;
                if !is_trashed_user_expired(&trashed, now)
                    && trashed.user.email_lower.as_deref() == Some(new_lower.as_str())
                {
                    return Err(DbError::EmailInTrash(new_email));
                }
            }
            drop(trashed_table);
            if let Some(old_lower) = row.email_lower.take() {
                let _ = by_email
                    .remove(old_lower.as_str())
                    .map_err(map_storage_err)?;
            }
            by_email
                .insert(new_lower.as_str(), user_id)
                .map_err(map_storage_err)?;
            row.email = Some(new_email);
            row.email_lower = Some(new_lower);
            row.updated_at_ms = now;
            let encoded = to_json(&row)?;
            users
                .insert(user_id, encoded.as_str())
                .map_err(map_storage_err)?;
            row.to_profile()
        };
        tx.commit().map_err(map_commit_err)?;
        Ok(profile)
    }

    pub fn login_user(
        &self,
        identifier_raw: &str,
        password: &str,
        device_name_raw: Option<&str>,
        ttls: TokenTtls,
    ) -> Result<AuthResponse, DbError> {
        let (_, email_lower) = normalize_email(identifier_raw).map_err(|_| {
            DbError::Validation("Please enter a valid email address to sign in.".to_string())
        })?;
        if password.is_empty() || password.len() > papercraft_sync_protocol::MAX_PASSWORD_LEN {
            return Err(DbError::InvalidCredentials);
        }
        let device_name = sanitize_device_name(device_name_raw.unwrap_or(""));
        let now = now_unix_ms();

        let (active_user, trashed_match) = {
            let rx = self.db.begin_read().map_err(map_tx_err)?;
            let users = rx.open_table(USERS_TABLE).map_err(map_table_err)?;
            if let Some(user_id) = lookup_user_id_by_email(&rx, &email_lower)? {
                let row = users
                    .get(user_id.as_str())
                    .map_err(map_storage_err)?
                    .map(|g| from_json::<UserRow>(g.value()))
                    .transpose()?;
                (row, None)
            } else {
                let trashed = lookup_trashed_user_by_email(&rx, &email_lower)?;
                (None, trashed)
            }
        };

        let user_row = match active_user {
            Some(row) => row,
            None => {
                if let Some(trashed) = trashed_match {
                    if is_trashed_user_expired(&trashed, now) {
                        let _ = self.cleanup_expired_trashed_users_at(now);
                    } else if password_matches_user(&trashed.user, password) {
                        return Err(DbError::AccountInTrash);
                    }
                }
                return Err(DbError::InvalidCredentials);
            }
        };

        if user_row.must_change_password {
            let trimmed_pw = password.trim();
            let verified = verify_password(password, &user_row.password_hash).is_ok()
                || (!trimmed_pw.is_empty()
                    && trimmed_pw != password
                    && verify_password(trimmed_pw, &user_row.password_hash).is_ok());
            return if verified {
                Err(DbError::PasswordChangeRequired)
            } else {
                Err(DbError::PasswordResetRequired)
            };
        }

        verify_password(password, &user_row.password_hash)?;

        let tx = self.db.begin_write().map_err(map_tx_err)?;
        let auth = {
            let mut access = tx.open_table(ACCESS_TOKENS_TABLE).map_err(map_table_err)?;
            let mut refresh = tx.open_table(REFRESH_TOKENS_TABLE).map_err(map_table_err)?;
            prune_expired_tokens(&mut access, &mut refresh, &user_row.user_id, now)?;
            issue_tokens(
                &user_row,
                &device_name,
                now,
                ttls,
                &mut access,
                &mut refresh,
            )?
        };
        tx.commit().map_err(map_commit_err)?;
        Ok(auth)
    }

    pub fn complete_password_reset(
        &self,
        identifier_raw: &str,
        recovery_code: &str,
        new_password: &str,
        device_name_raw: Option<&str>,
        ttls: TokenTtls,
    ) -> Result<AuthResponse, DbError> {
        let (_, email_lower) = normalize_email(identifier_raw).map_err(|_| {
            DbError::Validation(
                "Please enter a valid email address to recover your account.".to_string(),
            )
        })?;
        validate_password(new_password)?;
        let code = recovery_code.trim();
        if code.is_empty() || code.len() > papercraft_sync_protocol::MAX_PASSWORD_LEN {
            return Err(DbError::Validation(
                "Please enter the temporary recovery code generated on the server.".to_string(),
            ));
        }
        let device_name = sanitize_device_name(device_name_raw.unwrap_or(""));
        let now = now_unix_ms();

        let (active_user, trashed_match) = {
            let rx = self.db.begin_read().map_err(map_tx_err)?;
            let users = rx.open_table(USERS_TABLE).map_err(map_table_err)?;
            if let Some(user_id) = lookup_user_id_by_email(&rx, &email_lower)? {
                let row = users
                    .get(user_id.as_str())
                    .map_err(map_storage_err)?
                    .map(|g| from_json::<UserRow>(g.value()))
                    .transpose()?;
                (row, None)
            } else {
                let trashed = lookup_trashed_user_by_email(&rx, &email_lower)?;
                (None, trashed)
            }
        };

        let user_row = match active_user {
            Some(row) => row,
            None => {
                if let Some(trashed) = trashed_match {
                    if is_trashed_user_expired(&trashed, now) {
                        let _ = self.cleanup_expired_trashed_users_at(now);
                    } else if trashed.user.must_change_password
                        && verify_password(code, &trashed.user.password_hash).is_ok()
                    {
                        return Err(DbError::AccountInTrash);
                    }
                }
                return Err(DbError::InvalidCredentials);
            }
        };

        if !user_row.must_change_password {
            return Err(DbError::Validation(
                "This account does not have a pending password reset. Sign in with your regular password instead."
                    .to_string(),
            ));
        }

        verify_password(code, &user_row.password_hash).map_err(|_| {
            DbError::Validation(
                "Invalid recovery code. Check the code generated on the server and try again."
                    .to_string(),
            )
        })?;

        if new_password == code {
            return Err(DbError::Validation(
                "New password must be different from the temporary recovery code.".to_string(),
            ));
        }

        let new_hash = hash_password(new_password)?;
        let now = now_unix_ms();

        let tx = self.db.begin_write().map_err(map_tx_err)?;
        let auth = {
            let mut users = tx.open_table(USERS_TABLE).map_err(map_table_err)?;
            let mut access = tx.open_table(ACCESS_TOKENS_TABLE).map_err(map_table_err)?;
            let mut refresh = tx.open_table(REFRESH_TOKENS_TABLE).map_err(map_table_err)?;

            let Some(row_guard) = users
                .get(user_row.user_id.as_str())
                .map_err(map_storage_err)?
            else {
                return Err(DbError::InvalidCredentials);
            };
            let mut row: UserRow = from_json(row_guard.value())?;
            drop(row_guard);
            if !row.must_change_password {
                return Err(DbError::Validation(
                    "This account no longer has a pending password reset.".to_string(),
                ));
            }
            row.password_hash = new_hash;
            row.must_change_password = false;
            row.updated_at_ms = now;
            let encoded = to_json(&row)?;
            users
                .insert(row.user_id.as_str(), encoded.as_str())
                .map_err(map_storage_err)?;

            revoke_all_user_tokens(&mut access, &mut refresh, &row.user_id)?;
            issue_tokens(&row, &device_name, now, ttls, &mut access, &mut refresh)?
        };
        tx.commit().map_err(map_commit_err)?;
        Ok(auth)
    }

    pub fn update_profile(
        &self,
        user_id: &str,
        new_username_raw: Option<&str>,
        new_display_name_raw: Option<&str>,
    ) -> Result<UserProfile, DbError> {
        if new_username_raw.is_none() && new_display_name_raw.is_none() {
            return Err(DbError::Validation(
                "No profile changes were provided.".to_string(),
            ));
        }
        let parsed_username = match new_username_raw {
            Some(raw) => Some(normalize_username(raw)?),
            None => None,
        };
        let now = now_unix_ms();
        self.cleanup_expired_trashed_users_at(now)?;

        let tx = self.db.begin_write().map_err(map_tx_err)?;
        let profile = {
            let mut users = tx.open_table(USERS_TABLE).map_err(map_table_err)?;
            let mut by_name = tx.open_table(USERS_BY_NAME_TABLE).map_err(map_table_err)?;
            let trashed_table = tx.open_table(TRASHED_USERS_TABLE).map_err(map_table_err)?;
            let Some(user_guard) = users.get(user_id).map_err(map_storage_err)? else {
                return Err(DbError::Unauthorized);
            };
            let mut row: UserRow = from_json(user_guard.value())?;
            drop(user_guard);

            if let Some((clean_u, new_lower)) = parsed_username {
                if new_lower != row.username_lower {
                    if by_name
                        .get(new_lower.as_str())
                        .map_err(map_storage_err)?
                        .is_some()
                    {
                        return Err(DbError::UsernameTaken(clean_u));
                    }
                    for item in trashed_table.iter().map_err(map_storage_err)? {
                        let (_, v) = item.map_err(map_storage_err)?;
                        let trashed: TrashedUserRow = from_json(v.value())?;
                        if !is_trashed_user_expired(&trashed, now)
                            && trashed.user.username_lower == new_lower
                        {
                            return Err(DbError::UsernameInTrash(clean_u));
                        }
                    }
                    let _ = by_name
                        .remove(row.username_lower.as_str())
                        .map_err(map_storage_err)?;
                    by_name
                        .insert(new_lower.as_str(), user_id)
                        .map_err(map_storage_err)?;
                }
                row.username = clean_u;
                row.username_lower = new_lower;
            }
            drop(trashed_table);

            if let Some(raw_display) = new_display_name_raw {
                row.display_name = sanitize_display_name(raw_display, &row.username);
            }

            row.updated_at_ms = now;
            let encoded = to_json(&row)?;
            users
                .insert(user_id, encoded.as_str())
                .map_err(map_storage_err)?;
            row.to_profile()
        };
        tx.commit().map_err(map_commit_err)?;
        Ok(profile)
    }

    pub fn change_password(
        &self,
        user_id: &str,
        current_password: &str,
        new_password: &str,
    ) -> Result<UserProfile, DbError> {
        validate_password(new_password)?;
        if current_password.is_empty()
            || current_password.len() > papercraft_sync_protocol::MAX_PASSWORD_LEN
        {
            return Err(DbError::IncorrectCurrentPassword);
        }

        let current_hash = {
            let rx = self.db.begin_read().map_err(map_tx_err)?;
            let users = rx.open_table(USERS_TABLE).map_err(map_table_err)?;
            let Some(user_guard) = users.get(user_id).map_err(map_storage_err)? else {
                return Err(DbError::Unauthorized);
            };
            let row: UserRow = from_json(user_guard.value())?;
            row.password_hash
        };

        verify_password(current_password, &current_hash)
            .map_err(|_| DbError::IncorrectCurrentPassword)?;
        if new_password == current_password {
            return Err(DbError::Validation(
                "New password must be different from your current password.".to_string(),
            ));
        }

        let new_hash = hash_password(new_password)?;
        let now = now_unix_ms();

        let tx = self.db.begin_write().map_err(map_tx_err)?;
        let profile = {
            let mut users = tx.open_table(USERS_TABLE).map_err(map_table_err)?;
            let Some(user_guard) = users.get(user_id).map_err(map_storage_err)? else {
                return Err(DbError::Unauthorized);
            };
            let mut row: UserRow = from_json(user_guard.value())?;
            drop(user_guard);

            row.password_hash = new_hash;
            row.must_change_password = false;
            row.updated_at_ms = now;
            let encoded = to_json(&row)?;
            users
                .insert(user_id, encoded.as_str())
                .map_err(map_storage_err)?;
            row.to_profile()
        };
        tx.commit().map_err(map_commit_err)?;
        Ok(profile)
    }

    pub fn refresh_session(
        &self,
        refresh_token_raw: &str,
        ttls: TokenTtls,
    ) -> Result<AuthResponse, DbError> {
        let trimmed = refresh_token_raw.trim();
        if trimmed.is_empty() || trimmed.len() > 256 {
            return Err(DbError::Unauthorized);
        }
        let token_hash = sha256_hex(trimmed.as_bytes());
        let now = now_unix_ms();

        let tx = self.db.begin_write().map_err(map_tx_err)?;
        let auth = {
            let users = tx.open_table(USERS_TABLE).map_err(map_table_err)?;
            let mut access = tx.open_table(ACCESS_TOKENS_TABLE).map_err(map_table_err)?;
            let mut refresh = tx.open_table(REFRESH_TOKENS_TABLE).map_err(map_table_err)?;

            let old_row: RefreshTokenRow = {
                let removed = refresh
                    .remove(token_hash.as_str())
                    .map_err(map_storage_err)?;
                let Some(old_guard) = removed else {
                    return Err(DbError::Unauthorized);
                };
                from_json(old_guard.value())?
            };
            if old_row.expires_at_ms <= now {
                return Err(DbError::Unauthorized);
            }
            let Some(user_guard) = users
                .get(old_row.user_id.as_str())
                .map_err(map_storage_err)?
            else {
                return Err(DbError::Unauthorized);
            };
            let user_row: UserRow = from_json(user_guard.value())?;
            if user_row.must_change_password {
                return Err(DbError::Unauthorized);
            }
            prune_expired_tokens(&mut access, &mut refresh, &user_row.user_id, now)?;
            issue_tokens(
                &user_row,
                &old_row.device_name,
                now,
                ttls,
                &mut access,
                &mut refresh,
            )?
        };
        tx.commit().map_err(map_commit_err)?;
        Ok(auth)
    }

    pub fn logout(
        &self,
        access_token_raw: Option<&str>,
        refresh_token_raw: Option<&str>,
    ) -> Result<(), DbError> {
        let tx = self.db.begin_write().map_err(map_tx_err)?;
        {
            if let Some(tok) = access_token_raw.map(str::trim).filter(|s| !s.is_empty()) {
                let hash = sha256_hex(tok.as_bytes());
                let mut access = tx.open_table(ACCESS_TOKENS_TABLE).map_err(map_table_err)?;
                let _ = access.remove(hash.as_str()).map_err(map_storage_err)?;
            }
            if let Some(tok) = refresh_token_raw.map(str::trim).filter(|s| !s.is_empty()) {
                let hash = sha256_hex(tok.as_bytes());
                let mut refresh = tx.open_table(REFRESH_TOKENS_TABLE).map_err(map_table_err)?;
                let _ = refresh.remove(hash.as_str()).map_err(map_storage_err)?;
            }
        }
        tx.commit().map_err(map_commit_err)?;
        Ok(())
    }

    pub fn authenticate_access_token(&self, bearer_raw: &str) -> Result<UserProfile, DbError> {
        let trimmed = bearer_raw.trim();
        if trimmed.is_empty() || trimmed.len() > 256 {
            return Err(DbError::Unauthorized);
        }
        let token_hash = sha256_hex(trimmed.as_bytes());
        let now = now_unix_ms();

        let rx = self.db.begin_read().map_err(map_tx_err)?;
        let access = rx.open_table(ACCESS_TOKENS_TABLE).map_err(map_table_err)?;
        let Some(tok_guard) = access.get(token_hash.as_str()).map_err(map_storage_err)? else {
            return Err(DbError::Unauthorized);
        };
        let tok_row: AccessTokenRow = from_json(tok_guard.value())?;
        if tok_row.expires_at_ms <= now {
            return Err(DbError::Unauthorized);
        }
        let users = rx.open_table(USERS_TABLE).map_err(map_table_err)?;
        let Some(user_guard) = users
            .get(tok_row.user_id.as_str())
            .map_err(map_storage_err)?
        else {
            return Err(DbError::Unauthorized);
        };
        let user_row: UserRow = from_json(user_guard.value())?;
        if user_row.must_change_password {
            return Err(DbError::Unauthorized);
        }
        Ok(user_row.to_profile())
    }

    pub fn pull_changes(
        &self,
        user_id: &str,
        since_cursor: u64,
    ) -> Result<SyncPullResponse, DbError> {
        let rx = self.db.begin_read().map_err(map_tx_err)?;
        let users = rx.open_table(USERS_TABLE).map_err(map_table_err)?;
        let Some(user_guard) = users.get(user_id).map_err(map_storage_err)? else {
            return Err(DbError::Unauthorized);
        };
        let user_row: UserRow = from_json(user_guard.value())?;

        let cursors = rx.open_table(USER_CURSORS_TABLE).map_err(map_table_err)?;
        let server_cursor = cursors
            .get(user_id)
            .map_err(map_storage_err)?
            .map_or(0, |g| g.value());

        let notebooks_table = rx.open_table(NOTEBOOKS_TABLE).map_err(map_table_err)?;
        let blobs_table = rx.open_table(NOTEBOOK_BLOBS_TABLE).map_err(map_table_err)?;
        let prefix = user_notebook_prefix(user_id);
        let mut notebooks = Vec::new();
        for item in notebooks_table.iter().map_err(map_storage_err)? {
            let (k, v) = item.map_err(map_storage_err)?;
            if !k.value().starts_with(&prefix) {
                continue;
            }
            let row: StoredNotebookRow = from_json(v.value())?;
            if row.cursor <= since_cursor {
                continue;
            }
            // Initial full sync (`since_cursor == 0`) skips historical tombstones.
            if since_cursor == 0 && row.deleted {
                continue;
            }
            let blob = if row.deleted {
                None
            } else {
                blobs_table
                    .get(k.value())
                    .map_err(map_storage_err)?
                    .map(|g| g.value().to_vec())
            };
            notebooks.push(stored_row_to_remote(&row, blob.as_deref())?);
        }
        notebooks.sort_by_key(|r| r.cursor);

        let folders = load_pulled_folders(&rx, user_id, since_cursor)?;

        let prefs_table = rx.open_table(USER_PREFS_TABLE).map_err(map_table_err)?;
        let studio_prefs = match prefs_table.get(user_id).map_err(map_storage_err)? {
            Some(guard) => {
                let row: StoredPrefsRow = from_json(guard.value())?;
                Some(RemotePrefsRecord {
                    version: row.version,
                    updated_at_ms: row.updated_at_ms,
                    content_hash: row.content_hash,
                    prefs: row.prefs,
                })
            }
            None => None,
        };

        Ok(SyncPullResponse {
            server_cursor,
            user: user_row.to_profile(),
            notebooks,
            folders,
            studio_prefs,
        })
    }

    pub fn push_changes(
        &self,
        user_id: &str,
        req: SyncPushRequest,
        max_notebook_bytes: usize,
    ) -> Result<SyncPushResponse, DbError> {
        if req.changes.len() > MAX_PUSH_BATCH_SIZE {
            return Err(DbError::Validation(format!(
                "push batch exceeds maximum of {MAX_PUSH_BATCH_SIZE} notebooks"
            )));
        }
        if req.folders.len() > papercraft_doc::MAX_LIBRARY_FOLDERS {
            return Err(DbError::Validation(
                "folder push batch exceeds maximum library folder count".to_string(),
            ));
        }
        let mut seen_ids = HashSet::with_capacity(req.changes.len());
        let mut validated_changes = Vec::with_capacity(req.changes.len());
        for change in req.changes {
            let clean_id = validate_notebook_id(&change.notebook_id)?;
            if !seen_ids.insert(clean_id.clone()) {
                return Err(DbError::Validation(format!(
                    "duplicate notebook ID `{clean_id}` in push batch"
                )));
            }
            validated_changes.push(prepare_push_change(clean_id, change, max_notebook_bytes)?);
        }
        let mut seen_folders = HashSet::with_capacity(req.folders.len());
        let mut validated_folders = Vec::with_capacity(req.folders.len());
        for folder in req.folders {
            let clean_id = validate_folder_id(&folder.folder_id)?;
            if !seen_folders.insert(clean_id.clone()) {
                return Err(DbError::Validation(format!(
                    "duplicate folder ID `{clean_id}` in push batch"
                )));
            }
            validated_folders.push((clean_id, folder));
        }

        let now = now_unix_ms();
        let tx = self.db.begin_write().map_err(map_tx_err)?;
        let response = {
            let users = tx.open_table(USERS_TABLE).map_err(map_table_err)?;
            if users.get(user_id).map_err(map_storage_err)?.is_none() {
                return Err(DbError::Unauthorized);
            }
            let mut cursors = tx.open_table(USER_CURSORS_TABLE).map_err(map_table_err)?;
            let mut notebooks = tx.open_table(NOTEBOOKS_TABLE).map_err(map_table_err)?;
            let mut blobs = tx.open_table(NOTEBOOK_BLOBS_TABLE).map_err(map_table_err)?;
            let mut folders_table = tx.open_table(FOLDERS_TABLE).map_err(map_table_err)?;
            let mut prefs_table = tx.open_table(USER_PREFS_TABLE).map_err(map_table_err)?;

            let mut current_cursor = cursors
                .get(user_id)
                .map_err(map_storage_err)?
                .map_or(0, |g| g.value());

            let mut accepted = Vec::new();
            let mut conflicts = Vec::new();

            for prepared in validated_changes {
                let key = notebook_key(user_id, &prepared.notebook_id);
                let existing = notebooks
                    .get(key.as_str())
                    .map_err(map_storage_err)?
                    .map(|g| from_json::<StoredNotebookRow>(g.value()))
                    .transpose()?;

                let mut ctx = PushWriteCtx {
                    user_id,
                    key: &key,
                    now,
                    max_notebook_bytes,
                    force_overwrite: req.force_overwrite,
                    current_cursor: &mut current_cursor,
                    notebooks: &mut notebooks,
                    blobs: &mut blobs,
                    accepted: &mut accepted,
                    conflicts: &mut conflicts,
                };
                apply_single_notebook_push(prepared, existing, &mut ctx)?;
            }

            let mut accepted_folders = Vec::new();
            let mut folder_conflicts = Vec::new();
            for (clean_id, folder) in validated_folders {
                apply_single_folder_push(
                    user_id,
                    &clean_id,
                    folder,
                    req.force_overwrite,
                    now,
                    &mut current_cursor,
                    &mut folders_table,
                    &mut accepted_folders,
                    &mut folder_conflicts,
                )?;
            }

            let (prefs_version, prefs_conflict) = apply_prefs_push(
                user_id,
                req.studio_prefs,
                req.force_overwrite,
                now,
                &mut prefs_table,
            )?;

            cursors
                .insert(user_id, current_cursor)
                .map_err(map_storage_err)?;

            SyncPushResponse {
                server_cursor: current_cursor,
                accepted,
                conflicts,
                accepted_folders,
                folder_conflicts,
                prefs_version,
                prefs_conflict,
            }
        };
        tx.commit().map_err(map_commit_err)?;
        Ok(response)
    }

    pub fn prune_to_local(
        &self,
        user_id: &str,
        req: PruneToLocalRequest,
    ) -> Result<PruneToLocalResponse, DbError> {
        let mut keep_nb = HashSet::with_capacity(req.keep_notebook_ids.len());
        for raw_id in req.keep_notebook_ids {
            keep_nb.insert(validate_notebook_id(&raw_id)?);
        }
        let mut keep_fld = HashSet::with_capacity(req.keep_folder_ids.len());
        for raw_id in req.keep_folder_ids {
            keep_fld.insert(validate_folder_id(&raw_id)?);
        }

        let now = now_unix_ms();
        let prefix = user_notebook_prefix(user_id);
        let tx = self.db.begin_write().map_err(map_tx_err)?;
        let resp = {
            let users = tx.open_table(USERS_TABLE).map_err(map_table_err)?;
            if users.get(user_id).map_err(map_storage_err)?.is_none() {
                return Err(DbError::Unauthorized);
            }
            let mut cursors = tx.open_table(USER_CURSORS_TABLE).map_err(map_table_err)?;
            let mut notebooks = tx.open_table(NOTEBOOKS_TABLE).map_err(map_table_err)?;
            let mut blobs = tx.open_table(NOTEBOOK_BLOBS_TABLE).map_err(map_table_err)?;
            let mut folders = tx.open_table(FOLDERS_TABLE).map_err(map_table_err)?;

            let mut current_cursor = cursors
                .get(user_id)
                .map_err(map_storage_err)?
                .map_or(0, |g| g.value());

            let mut stale_notebooks: Vec<(String, StoredNotebookRow)> = Vec::new();
            for item in notebooks.iter().map_err(map_storage_err)? {
                let (k, v) = item.map_err(map_storage_err)?;
                if !k.value().starts_with(&prefix) {
                    continue;
                }
                let row: StoredNotebookRow = from_json(v.value())?;
                if !row.deleted && !keep_nb.contains(&row.notebook_id) {
                    stale_notebooks.push((k.value().to_string(), row));
                }
            }

            let mut pruned_notebooks = 0usize;
            for (key, mut row) in stale_notebooks {
                current_cursor = current_cursor.saturating_add(1);
                row.version = row.version.saturating_add(1);
                row.cursor = current_cursor;
                row.deleted = true;
                row.updated_at_ms = now;
                row.deleted_at_ms = Some(now);
                row.title.clear();
                row.content_hash = "deleted".to_string();
                row.body_hash.clear();
                row.meta_hash.clear();
                row.ncraft_json = None;
                let encoded = to_json(&row)?;
                notebooks
                    .insert(key.as_str(), encoded.as_str())
                    .map_err(map_storage_err)?;
                blobs.remove(key.as_str()).map_err(map_storage_err)?;
                pruned_notebooks = pruned_notebooks.saturating_add(1);
            }

            let mut stale_folders: Vec<(String, StoredFolderRow)> = Vec::new();
            for item in folders.iter().map_err(map_storage_err)? {
                let (k, v) = item.map_err(map_storage_err)?;
                if !k.value().starts_with(&prefix) {
                    continue;
                }
                let row: StoredFolderRow = from_json(v.value())?;
                if !row.deleted && !keep_fld.contains(&row.folder_id) {
                    stale_folders.push((k.value().to_string(), row));
                }
            }

            let mut pruned_folders = 0usize;
            for (key, mut row) in stale_folders {
                current_cursor = current_cursor.saturating_add(1);
                row.version = row.version.saturating_add(1);
                row.cursor = current_cursor;
                row.deleted = true;
                row.updated_at_ms = now;
                row.deleted_at_ms = Some(now);
                let encoded = to_json(&row)?;
                folders
                    .insert(key.as_str(), encoded.as_str())
                    .map_err(map_storage_err)?;
                pruned_folders = pruned_folders.saturating_add(1);
            }

            cursors
                .insert(user_id, current_cursor)
                .map_err(map_storage_err)?;

            PruneToLocalResponse {
                server_cursor: current_cursor,
                pruned_notebooks,
                pruned_folders,
            }
        };
        tx.commit().map_err(map_commit_err)?;
        Ok(resp)
    }

    pub fn stats(&self) -> Result<DbStats, DbError> {
        let now = now_unix_ms();
        let _ = self.cleanup_expired_trashed_users_at(now);
        let rx = self.db.begin_read().map_err(map_tx_err)?;
        let users = rx.open_table(USERS_TABLE).map_err(map_table_err)?;
        let trashed = rx.open_table(TRASHED_USERS_TABLE).map_err(map_table_err)?;
        let notebooks = rx.open_table(NOTEBOOKS_TABLE).map_err(map_table_err)?;
        let refresh = rx.open_table(REFRESH_TOKENS_TABLE).map_err(map_table_err)?;

        let user_count = users.iter().map_err(map_storage_err)?.count();
        let mut trashed_ids = HashSet::new();
        for entry in trashed.iter().map_err(map_storage_err)? {
            let (k, _) = entry.map_err(map_storage_err)?;
            trashed_ids.insert(k.value().to_string());
        }
        let trashed_user_count = trashed_ids.len();

        let mut active_notebook_count = 0usize;
        let mut deleted_tombstone_count = 0usize;
        for entry in notebooks.iter().map_err(map_storage_err)? {
            let (_, v) = entry.map_err(map_storage_err)?;
            let row: StoredNotebookRow = from_json(v.value())?;
            if trashed_ids.contains(&row.user_id) {
                continue;
            }
            if row.deleted {
                deleted_tombstone_count = deleted_tombstone_count.saturating_add(1);
            } else {
                active_notebook_count = active_notebook_count.saturating_add(1);
            }
        }
        let mut active_session_count = 0usize;
        for entry in refresh.iter().map_err(map_storage_err)? {
            let (_, v) = entry.map_err(map_storage_err)?;
            let row: RefreshTokenRow = from_json(v.value())?;
            if row.expires_at_ms > now {
                active_session_count = active_session_count.saturating_add(1);
            }
        }

        Ok(DbStats {
            user_count,
            trashed_user_count,
            active_notebook_count,
            deleted_tombstone_count,
            active_session_count,
        })
    }

    pub fn list_users(&self) -> Result<Vec<UserSummary>, DbError> {
        let rx = self.db.begin_read().map_err(map_tx_err)?;
        let users = rx.open_table(USERS_TABLE).map_err(map_table_err)?;
        let notebooks = rx.open_table(NOTEBOOKS_TABLE).map_err(map_table_err)?;
        let counts = count_active_notebooks_by_user(&notebooks)?;

        let mut out = Vec::new();
        for entry in users.iter().map_err(map_storage_err)? {
            let (_, v) = entry.map_err(map_storage_err)?;
            let row: UserRow = from_json(v.value())?;
            let notebook_count = counts.get(&row.user_id).copied().unwrap_or(0);
            out.push(UserSummary {
                user_id: row.user_id,
                email: row.email,
                username: row.username,
                display_name: row.display_name,
                notebook_count,
                created_at_ms: row.created_at_ms,
                must_change_password: row.must_change_password,
            });
        }
        out.sort_by_key(|a| a.created_at_ms);
        Ok(out)
    }

    /// Lists accounts currently held in the 30-day server Trash after purging any expired ones.
    pub fn list_trashed_users(&self) -> Result<Vec<TrashedUserSummary>, DbError> {
        let now = now_unix_ms();
        self.cleanup_expired_trashed_users_at(now)?;
        let rx = self.db.begin_read().map_err(map_tx_err)?;
        let trashed = rx.open_table(TRASHED_USERS_TABLE).map_err(map_table_err)?;
        let notebooks = rx.open_table(NOTEBOOKS_TABLE).map_err(map_table_err)?;
        let counts = count_active_notebooks_by_user(&notebooks)?;

        let mut out = Vec::new();
        for entry in trashed.iter().map_err(map_storage_err)? {
            let (_, v) = entry.map_err(map_storage_err)?;
            let row: TrashedUserRow = from_json(v.value())?;
            let notebook_count = counts.get(&row.user.user_id).copied().unwrap_or(0);
            out.push(TrashedUserSummary {
                user_id: row.user.user_id,
                email: row.user.email,
                username: row.user.username,
                display_name: row.user.display_name,
                notebook_count,
                created_at_ms: row.user.created_at_ms,
                deleted_at_ms: row.deleted_at_ms,
                expires_at_ms: row.expires_at_ms,
            });
        }
        out.sort_by(|a, b| {
            b.deleted_at_ms
                .cmp(&a.deleted_at_ms)
                .then_with(|| a.user_id.cmp(&b.user_id))
        });
        Ok(out)
    }

    pub fn set_user_password(
        &self,
        identifier_raw: &str,
        new_password: &str,
    ) -> Result<(), DbError> {
        self.set_user_password_with_flag(identifier_raw, new_password, false)
    }

    pub fn set_user_password_with_flag(
        &self,
        identifier_raw: &str,
        new_password: &str,
        must_change_password: bool,
    ) -> Result<(), DbError> {
        validate_password(new_password)?;
        let new_hash = hash_password(new_password)?;
        let now = now_unix_ms();

        let user_id = {
            let rx = self.db.begin_read().map_err(map_tx_err)?;
            lookup_user_id_by_id_or_identifier(&rx, identifier_raw)?
                .ok_or_else(|| DbError::UserNotFound(identifier_raw.trim().to_string()))?
        };

        let tx = self.db.begin_write().map_err(map_tx_err)?;
        {
            let mut users = tx.open_table(USERS_TABLE).map_err(map_table_err)?;
            let mut access = tx.open_table(ACCESS_TOKENS_TABLE).map_err(map_table_err)?;
            let mut refresh = tx.open_table(REFRESH_TOKENS_TABLE).map_err(map_table_err)?;
            let mut user_row: UserRow = {
                let Some(row_guard) = users.get(user_id.as_str()).map_err(map_storage_err)? else {
                    return Err(DbError::UserNotFound(identifier_raw.trim().to_string()));
                };
                from_json(row_guard.value())?
            };
            user_row.password_hash = new_hash;
            user_row.must_change_password = must_change_password;
            user_row.updated_at_ms = now;
            let encoded = to_json(&user_row)?;
            users
                .insert(user_id.as_str(), encoded.as_str())
                .map_err(map_storage_err)?;
            revoke_all_user_tokens(&mut access, &mut refresh, &user_id)?;
        }
        tx.commit().map_err(map_commit_err)?;
        Ok(())
    }

    /// Generates a random one-time temporary password/recovery code, revokes active sessions,
    /// and forces the user to enter that code and choose a new password on their next login.
    pub fn reset_user_password_temporary(
        &self,
        user_id_or_identifier: &str,
    ) -> Result<PasswordResetOutcome, DbError> {
        let user_id = {
            let rx = self.db.begin_read().map_err(map_tx_err)?;
            lookup_user_id_by_id_or_identifier(&rx, user_id_or_identifier)?
                .ok_or_else(|| DbError::UserNotFound(user_id_or_identifier.trim().to_string()))?
        };
        let temporary_password = generate_temporary_password()?;
        let new_hash = hash_password(&temporary_password)?;
        let now = now_unix_ms();

        let tx = self.db.begin_write().map_err(map_tx_err)?;
        let outcome = {
            let mut users = tx.open_table(USERS_TABLE).map_err(map_table_err)?;
            let mut access = tx.open_table(ACCESS_TOKENS_TABLE).map_err(map_table_err)?;
            let mut refresh = tx.open_table(REFRESH_TOKENS_TABLE).map_err(map_table_err)?;

            let mut user_row: UserRow = {
                let Some(row_guard) = users.get(user_id.as_str()).map_err(map_storage_err)? else {
                    return Err(DbError::UserNotFound(
                        user_id_or_identifier.trim().to_string(),
                    ));
                };
                from_json(row_guard.value())?
            };
            user_row.password_hash = new_hash;
            user_row.must_change_password = true;
            user_row.updated_at_ms = now;
            let encoded = to_json(&user_row)?;
            users
                .insert(user_id.as_str(), encoded.as_str())
                .map_err(map_storage_err)?;
            revoke_all_user_tokens(&mut access, &mut refresh, &user_id)?;

            PasswordResetOutcome {
                user_id: user_row.user_id,
                username: user_row.username,
                email: user_row.email,
                display_name: user_row.display_name,
                temporary_password,
            }
        };
        tx.commit().map_err(map_commit_err)?;
        Ok(outcome)
    }

    /// Permanently purges any trashed user accounts whose 30-day retention period has expired.
    pub fn cleanup_expired_trashed_users(&self) -> Result<usize, DbError> {
        self.cleanup_expired_trashed_users_at(now_unix_ms())
    }

    /// Permanently purges trashed user accounts expired as of `now_ms`.
    pub fn cleanup_expired_trashed_users_at(&self, now_ms: i64) -> Result<usize, DbError> {
        let has_expired = {
            let rx = self.db.begin_read().map_err(map_tx_err)?;
            let trashed = rx.open_table(TRASHED_USERS_TABLE).map_err(map_table_err)?;
            let mut found = false;
            for item in trashed.iter().map_err(map_storage_err)? {
                let (_, v) = item.map_err(map_storage_err)?;
                let row: TrashedUserRow = from_json(v.value())?;
                if is_trashed_user_expired(&row, now_ms) {
                    found = true;
                    break;
                }
            }
            found
        };
        if !has_expired {
            return Ok(0);
        }

        let tx = self.db.begin_write().map_err(map_tx_err)?;
        let purged_count = {
            let mut trashed = tx.open_table(TRASHED_USERS_TABLE).map_err(map_table_err)?;
            let mut access = tx.open_table(ACCESS_TOKENS_TABLE).map_err(map_table_err)?;
            let mut refresh = tx.open_table(REFRESH_TOKENS_TABLE).map_err(map_table_err)?;
            let mut notebooks = tx.open_table(NOTEBOOKS_TABLE).map_err(map_table_err)?;
            let mut blobs = tx.open_table(NOTEBOOK_BLOBS_TABLE).map_err(map_table_err)?;
            let mut folders = tx.open_table(FOLDERS_TABLE).map_err(map_table_err)?;
            let mut prefs = tx.open_table(USER_PREFS_TABLE).map_err(map_table_err)?;
            let mut cursors = tx.open_table(USER_CURSORS_TABLE).map_err(map_table_err)?;

            let mut expired_ids = Vec::new();
            for item in trashed.iter().map_err(map_storage_err)? {
                let (k, v) = item.map_err(map_storage_err)?;
                let row: TrashedUserRow = from_json(v.value())?;
                if is_trashed_user_expired(&row, now_ms) {
                    expired_ids.push(k.value().to_string());
                }
            }
            let count = expired_ids.len();
            for uid in expired_ids {
                let _ = trashed.remove(uid.as_str()).map_err(map_storage_err)?;
                let _ = purge_user_data_in_tx(
                    &uid,
                    &mut access,
                    &mut refresh,
                    &mut notebooks,
                    &mut blobs,
                    &mut folders,
                    &mut prefs,
                    &mut cursors,
                )?;
            }
            count
        };
        tx.commit().map_err(map_commit_err)?;
        Ok(purged_count)
    }

    /// Moves an active user account into the 30-day server Trash, revoking sessions while keeping synced data recoverable.
    pub fn trash_user(
        &self,
        user_id_or_identifier: &str,
    ) -> Result<TrashedUserActionSummary, DbError> {
        let now = now_unix_ms();
        self.cleanup_expired_trashed_users_at(now)?;
        let user_id = {
            let rx = self.db.begin_read().map_err(map_tx_err)?;
            lookup_user_id_by_id_or_identifier(&rx, user_id_or_identifier)?
                .ok_or_else(|| DbError::UserNotFound(user_id_or_identifier.trim().to_string()))?
        };
        let expires_at_ms = now.saturating_add(ACCOUNT_TRASH_RETENTION_MS);

        let tx = self.db.begin_write().map_err(map_tx_err)?;
        let summary = {
            let mut users = tx.open_table(USERS_TABLE).map_err(map_table_err)?;
            let mut by_name = tx.open_table(USERS_BY_NAME_TABLE).map_err(map_table_err)?;
            let mut by_email = tx.open_table(USERS_BY_EMAIL_TABLE).map_err(map_table_err)?;
            let mut trashed = tx.open_table(TRASHED_USERS_TABLE).map_err(map_table_err)?;
            let mut access = tx.open_table(ACCESS_TOKENS_TABLE).map_err(map_table_err)?;
            let mut refresh = tx.open_table(REFRESH_TOKENS_TABLE).map_err(map_table_err)?;
            let notebooks = tx.open_table(NOTEBOOKS_TABLE).map_err(map_table_err)?;

            let removed_user = users.remove(user_id.as_str()).map_err(map_storage_err)?;
            let Some(user_guard) = removed_user else {
                return Err(DbError::UserNotFound(
                    user_id_or_identifier.trim().to_string(),
                ));
            };
            let user_row: UserRow = from_json(user_guard.value())?;
            drop(user_guard);

            let _ = by_name
                .remove(user_row.username_lower.as_str())
                .map_err(map_storage_err)?;
            if let Some(ref email_lower) = user_row.email_lower {
                let _ = by_email
                    .remove(email_lower.as_str())
                    .map_err(map_storage_err)?;
            }

            revoke_all_user_tokens(&mut access, &mut refresh, &user_id)?;
            let notebook_count = count_user_active_notebooks(&notebooks, &user_id)?;

            let trashed_row = TrashedUserRow {
                user: user_row.clone(),
                deleted_at_ms: now,
                expires_at_ms,
            };
            let encoded = to_json(&trashed_row)?;
            trashed
                .insert(user_id.as_str(), encoded.as_str())
                .map_err(map_storage_err)?;

            TrashedUserActionSummary {
                user_id: user_row.user_id,
                username: user_row.username,
                email: user_row.email,
                display_name: user_row.display_name,
                notebook_count,
                deleted_at_ms: now,
                expires_at_ms,
            }
        };
        tx.commit().map_err(map_commit_err)?;
        Ok(summary)
    }

    /// Restores a non-expired user account from the 30-day server Trash back to active status.
    pub fn restore_trashed_user(
        &self,
        user_id_or_identifier: &str,
    ) -> Result<RestoredUserSummary, DbError> {
        let now = now_unix_ms();
        let matched_expired = {
            let rx = self.db.begin_read().map_err(map_tx_err)?;
            lookup_trashed_user_by_id_or_identifier(&rx, user_id_or_identifier)?
                .is_some_and(|row| is_trashed_user_expired(&row, now))
        };
        self.cleanup_expired_trashed_users_at(now)?;
        if matched_expired {
            return Err(DbError::TrashedUserExpired(
                user_id_or_identifier.trim().to_string(),
            ));
        }

        let tx = self.db.begin_write().map_err(map_tx_err)?;
        let summary = {
            let mut trashed = tx.open_table(TRASHED_USERS_TABLE).map_err(map_table_err)?;
            let mut users = tx.open_table(USERS_TABLE).map_err(map_table_err)?;
            let mut by_name = tx.open_table(USERS_BY_NAME_TABLE).map_err(map_table_err)?;
            let mut by_email = tx.open_table(USERS_BY_EMAIL_TABLE).map_err(map_table_err)?;
            let notebooks = tx.open_table(NOTEBOOKS_TABLE).map_err(map_table_err)?;

            let mut target_key: Option<String> = None;
            for item in trashed.iter().map_err(map_storage_err)? {
                let (k, v) = item.map_err(map_storage_err)?;
                let row: TrashedUserRow = from_json(v.value())?;
                if trashed_row_matches_id_or_identifier(&row, user_id_or_identifier) {
                    target_key = Some(k.value().to_string());
                    break;
                }
            }
            let Some(key) = target_key else {
                return Err(DbError::UserNotFound(
                    user_id_or_identifier.trim().to_string(),
                ));
            };
            let Some(trashed_guard) = trashed.get(key.as_str()).map_err(map_storage_err)? else {
                return Err(DbError::UserNotFound(
                    user_id_or_identifier.trim().to_string(),
                ));
            };
            let mut trashed_row: TrashedUserRow = from_json(trashed_guard.value())?;
            drop(trashed_guard);

            if by_name
                .get(trashed_row.user.username_lower.as_str())
                .map_err(map_storage_err)?
                .is_some()
            {
                return Err(DbError::UsernameTaken(trashed_row.user.username));
            }
            if let Some(ref email_lower) = trashed_row.user.email_lower
                && by_email
                    .get(email_lower.as_str())
                    .map_err(map_storage_err)?
                    .is_some()
            {
                return Err(DbError::EmailTaken(
                    trashed_row.user.email.clone().unwrap_or_default(),
                ));
            }

            let _ = trashed.remove(key.as_str()).map_err(map_storage_err)?;
            trashed_row.user.updated_at_ms = now;
            let user_row = trashed_row.user;
            let encoded = to_json(&user_row)?;
            users
                .insert(user_row.user_id.as_str(), encoded.as_str())
                .map_err(map_storage_err)?;
            by_name
                .insert(user_row.username_lower.as_str(), user_row.user_id.as_str())
                .map_err(map_storage_err)?;
            if let Some(ref email_lower) = user_row.email_lower {
                by_email
                    .insert(email_lower.as_str(), user_row.user_id.as_str())
                    .map_err(map_storage_err)?;
            }

            let restored_notebooks = count_user_active_notebooks(&notebooks, &user_row.user_id)?;
            RestoredUserSummary {
                user_id: user_row.user_id,
                username: user_row.username,
                email: user_row.email,
                display_name: user_row.display_name,
                restored_notebooks,
            }
        };
        tx.commit().map_err(map_commit_err)?;
        Ok(summary)
    }

    /// Permanently deletes a user account (active or trashed) and all of its notebooks, folders, preferences, and tokens.
    pub fn delete_user_permanently(
        &self,
        user_id_or_identifier: &str,
    ) -> Result<DeletedUserSummary, DbError> {
        let now = now_unix_ms();
        let _ = self.cleanup_expired_trashed_users_at(now);
        let user_id = {
            let rx = self.db.begin_read().map_err(map_tx_err)?;
            if let Some(id) = lookup_user_id_by_id_or_identifier(&rx, user_id_or_identifier)? {
                id
            } else if let Some(trashed) =
                lookup_trashed_user_by_id_or_identifier(&rx, user_id_or_identifier)?
            {
                trashed.user.user_id
            } else {
                return Err(DbError::UserNotFound(
                    user_id_or_identifier.trim().to_string(),
                ));
            }
        };

        let tx = self.db.begin_write().map_err(map_tx_err)?;
        let summary = {
            let mut users = tx.open_table(USERS_TABLE).map_err(map_table_err)?;
            let mut by_name = tx.open_table(USERS_BY_NAME_TABLE).map_err(map_table_err)?;
            let mut by_email = tx.open_table(USERS_BY_EMAIL_TABLE).map_err(map_table_err)?;
            let mut trashed = tx.open_table(TRASHED_USERS_TABLE).map_err(map_table_err)?;
            let mut access = tx.open_table(ACCESS_TOKENS_TABLE).map_err(map_table_err)?;
            let mut refresh = tx.open_table(REFRESH_TOKENS_TABLE).map_err(map_table_err)?;
            let mut notebooks = tx.open_table(NOTEBOOKS_TABLE).map_err(map_table_err)?;
            let mut blobs = tx.open_table(NOTEBOOK_BLOBS_TABLE).map_err(map_table_err)?;
            let mut folders = tx.open_table(FOLDERS_TABLE).map_err(map_table_err)?;
            let mut prefs = tx.open_table(USER_PREFS_TABLE).map_err(map_table_err)?;
            let mut cursors = tx.open_table(USER_CURSORS_TABLE).map_err(map_table_err)?;

            let user_row: UserRow = if let Some(user_guard) =
                users.remove(user_id.as_str()).map_err(map_storage_err)?
            {
                let row: UserRow = from_json(user_guard.value())?;
                drop(user_guard);
                let _ = by_name
                    .remove(row.username_lower.as_str())
                    .map_err(map_storage_err)?;
                if let Some(ref email_lower) = row.email_lower {
                    let _ = by_email
                        .remove(email_lower.as_str())
                        .map_err(map_storage_err)?;
                }
                row
            } else if let Some(trashed_guard) =
                trashed.remove(user_id.as_str()).map_err(map_storage_err)?
            {
                let row: TrashedUserRow = from_json(trashed_guard.value())?;
                drop(trashed_guard);
                row.user
            } else {
                return Err(DbError::UserNotFound(
                    user_id_or_identifier.trim().to_string(),
                ));
            };

            let deleted_notebooks = purge_user_data_in_tx(
                &user_id,
                &mut access,
                &mut refresh,
                &mut notebooks,
                &mut blobs,
                &mut folders,
                &mut prefs,
                &mut cursors,
            )?;

            DeletedUserSummary {
                user_id: user_row.user_id,
                username: user_row.username,
                email: user_row.email,
                display_name: user_row.display_name,
                deleted_notebooks,
            }
        };
        tx.commit().map_err(map_commit_err)?;
        Ok(summary)
    }
}

struct PreparedPushChange {
    notebook_id: String,
    base_version: u64,
    deleted: bool,
    deleted_at_ms: Option<i64>,
    title: String,
    content_hash: String,
    body_hash: String,
    meta_hash: String,
    meta_updated_at_ms: i64,
    doc: Option<Document>,
    ncraft_bytes: Option<Vec<u8>>,
}

struct PushWriteCtx<'a, 'b> {
    user_id: &'a str,
    key: &'a str,
    now: i64,
    max_notebook_bytes: usize,
    force_overwrite: bool,
    current_cursor: &'a mut u64,
    notebooks: &'a mut redb::Table<'b, &'static str, &'static str>,
    blobs: &'a mut redb::Table<'b, &'static str, &'static [u8]>,
    accepted: &'a mut Vec<AcceptedNotebookSync>,
    conflicts: &'a mut Vec<ConflictNotebookSync>,
}

fn prepare_push_change(
    notebook_id: String,
    change: LocalNotebookChange,
    max_notebook_bytes: usize,
) -> Result<PreparedPushChange, DbError> {
    if change.deleted {
        return Ok(PreparedPushChange {
            notebook_id,
            base_version: change.base_version,
            deleted: true,
            deleted_at_ms: Some(change.deleted_at_ms.unwrap_or(change.updated_at_ms)),
            title: String::new(),
            content_hash: "deleted".to_string(),
            body_hash: String::new(),
            meta_hash: String::new(),
            meta_updated_at_ms: change.meta_updated_at_ms,
            doc: None,
            ncraft_bytes: None,
        });
    }
    let Some(mut doc) = change.document else {
        return Err(DbError::Validation(format!(
            "notebook `{notebook_id}` update must include document payload"
        )));
    };
    if doc.id.is_empty() {
        doc.id = notebook_id.clone();
    }
    doc.title = sanitize_notebook_title(&doc.title);
    doc.sync_active_to_pages();
    doc.load_active_from_pages();
    let content_hash = canonical_document_hash(&doc);
    let body_hash = canonical_document_body_hash(&doc);
    let meta_hash = canonical_document_meta_hash(&doc);
    let meta_updated_at_ms = doc
        .meta_updated_at_ms
        .max(change.meta_updated_at_ms)
        .max(doc.updated_at_ms);
    let ncraft_bytes = save_document_bytes(&doc)
        .map_err(|e| DbError::Validation(format!("invalid notebook `{notebook_id}`: {e}")))?;
    if ncraft_bytes.len() > max_notebook_bytes {
        return Err(DbError::PayloadTooLarge {
            title: doc.title,
            actual_bytes: ncraft_bytes.len(),
            limit_bytes: max_notebook_bytes,
        });
    }
    Ok(PreparedPushChange {
        notebook_id,
        base_version: change.base_version,
        deleted: false,
        deleted_at_ms: None,
        title: doc.title.clone(),
        content_hash,
        body_hash,
        meta_hash,
        meta_updated_at_ms,
        doc: Some(doc),
        ncraft_bytes: Some(ncraft_bytes),
    })
}

fn apply_single_notebook_push(
    mut prepared: PreparedPushChange,
    existing: Option<StoredNotebookRow>,
    ctx: &mut PushWriteCtx<'_, '_>,
) -> Result<(), DbError> {
    let existing_version = existing.as_ref().map_or(0, |r| r.version);

    if let Some(ref row) = existing {
        // Idempotent retry or identical state across devices.
        if row.deleted == prepared.deleted
            && (row.deleted || row.content_hash == prepared.content_hash)
        {
            ctx.accepted.push(AcceptedNotebookSync {
                notebook_id: prepared.notebook_id,
                new_version: row.version,
                cursor: row.cursor,
                deleted: row.deleted,
                content_hash: row.content_hash.clone(),
                body_hash: row.body_hash.clone(),
                meta_hash: row.meta_hash.clone(),
            });
            return Ok(());
        }
        if !ctx.force_overwrite && prepared.base_version != row.version {
            let same_body = !row.deleted
                && !prepared.deleted
                && !row.body_hash.is_empty()
                && row.body_hash == prepared.body_hash;
            if same_body && prepared.meta_updated_at_ms > row.meta_updated_at_ms {
                prepared = merge_client_metadata_onto_server(row, prepared, ctx)?;
            } else {
                let blob = ctx
                    .blobs
                    .get(ctx.key)
                    .map_err(map_storage_err)?
                    .map(|g| g.value().to_vec());
                ctx.conflicts.push(ConflictNotebookSync {
                    notebook_id: prepared.notebook_id,
                    server_record: stored_row_to_remote(row, blob.as_deref())?,
                });
                return Ok(());
            }
        }
    } else if prepared.deleted {
        // Deleting a notebook the server has never seen succeeds cleanly as a no-op.
        ctx.accepted.push(AcceptedNotebookSync {
            notebook_id: prepared.notebook_id,
            new_version: 0,
            cursor: *ctx.current_cursor,
            deleted: true,
            content_hash: "deleted".to_string(),
            body_hash: String::new(),
            meta_hash: String::new(),
        });
        return Ok(());
    }

    *ctx.current_cursor = ctx.current_cursor.saturating_add(1);
    let new_version = existing_version.saturating_add(1);
    let next_row = StoredNotebookRow {
        notebook_id: prepared.notebook_id.clone(),
        user_id: ctx.user_id.to_string(),
        version: new_version,
        cursor: *ctx.current_cursor,
        deleted: prepared.deleted,
        updated_at_ms: ctx.now,
        deleted_at_ms: prepared.deleted_at_ms,
        title: prepared.title,
        content_hash: prepared.content_hash.clone(),
        body_hash: prepared.body_hash.clone(),
        meta_hash: prepared.meta_hash.clone(),
        meta_updated_at_ms: prepared.meta_updated_at_ms,
        ncraft_json: None,
    };
    let encoded = to_json(&next_row)?;
    ctx.notebooks
        .insert(ctx.key, encoded.as_str())
        .map_err(map_storage_err)?;
    match prepared.ncraft_bytes {
        Some(ref bytes) if !prepared.deleted => {
            ctx.blobs
                .insert(ctx.key, bytes.as_slice())
                .map_err(map_storage_err)?;
        }
        _ => {
            ctx.blobs.remove(ctx.key).map_err(map_storage_err)?;
        }
    }
    ctx.accepted.push(AcceptedNotebookSync {
        notebook_id: prepared.notebook_id,
        new_version,
        cursor: *ctx.current_cursor,
        deleted: prepared.deleted,
        content_hash: prepared.content_hash,
        body_hash: prepared.body_hash,
        meta_hash: prepared.meta_hash,
    });
    Ok(())
}

fn merge_client_metadata_onto_server(
    row: &StoredNotebookRow,
    mut prepared: PreparedPushChange,
    ctx: &PushWriteCtx<'_, '_>,
) -> Result<PreparedPushChange, DbError> {
    let blob = ctx
        .blobs
        .get(ctx.key)
        .map_err(map_storage_err)?
        .map(|g| g.value().to_vec());
    let mut server_doc = stored_row_to_remote(row, blob.as_deref())?
        .document
        .unwrap_or_default();
    if let Some(ref client_doc) = prepared.doc {
        server_doc.title = client_doc.title.clone();
        server_doc.cover = client_doc.cover;
        server_doc.folder_path = client_doc.folder_path.clone();
        server_doc.favorite = client_doc.favorite;
        server_doc.tags = client_doc.tags.clone();
        server_doc.meta_updated_at_ms = prepared.meta_updated_at_ms;
    }
    server_doc.sync_active_to_pages();
    server_doc.load_active_from_pages();
    let content_hash = canonical_document_hash(&server_doc);
    let body_hash = canonical_document_body_hash(&server_doc);
    let meta_hash = canonical_document_meta_hash(&server_doc);
    let ncraft_bytes = save_document_bytes(&server_doc)
        .map_err(|e| DbError::Validation(format!("invalid notebook `{}`: {e}", row.notebook_id)))?;
    if ncraft_bytes.len() > ctx.max_notebook_bytes {
        return Err(DbError::PayloadTooLarge {
            title: server_doc.title,
            actual_bytes: ncraft_bytes.len(),
            limit_bytes: ctx.max_notebook_bytes,
        });
    }
    prepared.title = server_doc.title.clone();
    prepared.content_hash = content_hash;
    prepared.body_hash = body_hash;
    prepared.meta_hash = meta_hash;
    prepared.doc = Some(server_doc);
    prepared.ncraft_bytes = Some(ncraft_bytes);
    Ok(prepared)
}

fn apply_prefs_push(
    user_id: &str,
    push: Option<LocalPrefsPush>,
    force_overwrite: bool,
    now: i64,
    prefs_table: &mut redb::Table<&str, &str>,
) -> Result<(u64, Option<RemotePrefsRecord>), DbError> {
    let existing = prefs_table
        .get(user_id)
        .map_err(map_storage_err)?
        .map(|g| from_json::<StoredPrefsRow>(g.value()))
        .transpose()?;
    let existing_ver = existing.as_ref().map_or(0, |r| r.version);

    let Some(req_prefs) = push else {
        return Ok((existing_ver, None));
    };

    let clean_prefs = req_prefs.prefs.sanitized();
    let hash = canonical_prefs_hash(&clean_prefs);
    if let Some(ref row) = existing {
        if row.content_hash == hash {
            return Ok((row.version, None));
        }
        if !force_overwrite && req_prefs.base_version != row.version {
            return Ok((
                row.version,
                Some(RemotePrefsRecord {
                    version: row.version,
                    updated_at_ms: row.updated_at_ms,
                    content_hash: row.content_hash.clone(),
                    prefs: row.prefs.clone(),
                }),
            ));
        }
    }

    let next_ver = existing_ver.saturating_add(1);
    let next_row = StoredPrefsRow {
        version: next_ver,
        updated_at_ms: now,
        content_hash: hash,
        prefs: clean_prefs,
    };
    let encoded = to_json(&next_row)?;
    prefs_table
        .insert(user_id, encoded.as_str())
        .map_err(map_storage_err)?;
    Ok((next_ver, None))
}

fn stored_row_to_remote(
    row: &StoredNotebookRow,
    blob: Option<&[u8]>,
) -> Result<RemoteNotebookRecord, DbError> {
    let corrupted = |e: papercraft_format::FormatError| {
        DbError::Storage(format!(
            "corrupted stored notebook `{}`: {e}",
            row.notebook_id
        ))
    };
    let loaded = if row.deleted {
        None
    } else if let Some(bytes) = blob {
        Some(load_document_bytes(bytes).map_err(corrupted)?)
    } else if let Some(ref json_str) = row.ncraft_json {
        Some(load_document(json_str).map_err(corrupted)?)
    } else {
        None
    };
    let document: Option<Document> = loaded.map(|mut doc| {
        doc.sync_active_to_pages();
        doc.load_active_from_pages();
        doc
    });
    Ok(RemoteNotebookRecord {
        notebook_id: row.notebook_id.clone(),
        version: row.version,
        cursor: row.cursor,
        deleted: row.deleted,
        updated_at_ms: row.updated_at_ms,
        deleted_at_ms: row.deleted_at_ms,
        title: row.title.clone(),
        content_hash: row.content_hash.clone(),
        body_hash: row.body_hash.clone(),
        meta_hash: row.meta_hash.clone(),
        meta_updated_at_ms: row.meta_updated_at_ms,
        document,
    })
}

fn allocate_unique_username(
    by_name: &redb::Table<&str, &str>,
    trashed_usernames: &HashSet<String>,
    base_username: &str,
    user_id: &str,
    strict_unique: bool,
) -> Result<(String, String), DbError> {
    let base_lower = base_username.to_ascii_lowercase();
    let active_taken = by_name
        .get(base_lower.as_str())
        .map_err(map_storage_err)?
        .is_some();
    let trashed_taken = trashed_usernames.contains(base_lower.as_str());
    if !active_taken && !trashed_taken {
        return Ok((base_username.to_string(), base_lower));
    }
    if strict_unique {
        if trashed_taken && !active_taken {
            return Err(DbError::UsernameInTrash(base_username.to_string()));
        }
        return Err(DbError::UsernameTaken(base_username.to_string()));
    }
    let short: String = user_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(6)
        .collect();
    for suffix in 2..=99 {
        let stem: String = base_username.chars().take(26).collect();
        let cand = format!("{stem}_{suffix}");
        let cand_lower = cand.to_ascii_lowercase();
        if by_name
            .get(cand_lower.as_str())
            .map_err(map_storage_err)?
            .is_none()
            && !trashed_usernames.contains(cand_lower.as_str())
        {
            return Ok((cand, cand_lower));
        }
    }
    let stem: String = base_username.chars().take(24).collect();
    let cand = format!("{stem}_{short}");
    let cand_lower = cand.to_ascii_lowercase();
    Ok((cand, cand_lower))
}

fn password_matches_user(user: &UserRow, password: &str) -> bool {
    if verify_password(password, &user.password_hash).is_ok() {
        return true;
    }
    if user.must_change_password {
        let trimmed = password.trim();
        return !trimmed.is_empty()
            && trimmed != password
            && verify_password(trimmed, &user.password_hash).is_ok();
    }
    false
}

fn count_active_notebooks_by_user(
    notebooks: &impl ReadableTable<&'static str, &'static str>,
) -> Result<std::collections::HashMap<String, usize>, DbError> {
    let mut counts = std::collections::HashMap::<String, usize>::new();
    for entry in notebooks.iter().map_err(map_storage_err)? {
        let (_, v) = entry.map_err(map_storage_err)?;
        let row: StoredNotebookRow = from_json(v.value())?;
        if !row.deleted {
            *counts.entry(row.user_id).or_insert(0) += 1;
        }
    }
    Ok(counts)
}

fn count_user_active_notebooks(
    notebooks: &impl ReadableTable<&'static str, &'static str>,
    user_id: &str,
) -> Result<usize, DbError> {
    let prefix = user_notebook_prefix(user_id);
    let mut count = 0usize;
    for entry in notebooks.iter().map_err(map_storage_err)? {
        let (k, v) = entry.map_err(map_storage_err)?;
        if !k.value().starts_with(&prefix) {
            continue;
        }
        let row: StoredNotebookRow = from_json(v.value())?;
        if !row.deleted {
            count = count.saturating_add(1);
        }
    }
    Ok(count)
}

#[allow(clippy::too_many_arguments)]
fn purge_user_data_in_tx(
    user_id: &str,
    access: &mut redb::Table<&str, &str>,
    refresh: &mut redb::Table<&str, &str>,
    notebooks: &mut redb::Table<&str, &str>,
    blobs: &mut redb::Table<&str, &[u8]>,
    folders: &mut redb::Table<&str, &str>,
    prefs: &mut redb::Table<&str, &str>,
    cursors: &mut redb::Table<&str, u64>,
) -> Result<usize, DbError> {
    let prefix = user_notebook_prefix(user_id);
    revoke_all_user_tokens(access, refresh, user_id)?;

    let mut notebook_keys = Vec::new();
    let mut deleted_notebooks = 0usize;
    for item in notebooks.iter().map_err(map_storage_err)? {
        let (k, v) = item.map_err(map_storage_err)?;
        if k.value().starts_with(&prefix) {
            notebook_keys.push(k.value().to_string());
            if from_json::<StoredNotebookRow>(v.value())
                .ok()
                .is_some_and(|row| !row.deleted)
            {
                deleted_notebooks = deleted_notebooks.saturating_add(1);
            }
        }
    }
    for k in notebook_keys {
        let _ = notebooks.remove(k.as_str()).map_err(map_storage_err)?;
    }

    let mut blob_keys = Vec::new();
    for item in blobs.iter().map_err(map_storage_err)? {
        let (k, _) = item.map_err(map_storage_err)?;
        if k.value().starts_with(&prefix) {
            blob_keys.push(k.value().to_string());
        }
    }
    for k in blob_keys {
        let _ = blobs.remove(k.as_str()).map_err(map_storage_err)?;
    }

    let mut folder_keys = Vec::new();
    for item in folders.iter().map_err(map_storage_err)? {
        let (k, _) = item.map_err(map_storage_err)?;
        if k.value().starts_with(&prefix) {
            folder_keys.push(k.value().to_string());
        }
    }
    for k in folder_keys {
        let _ = folders.remove(k.as_str()).map_err(map_storage_err)?;
    }

    let _ = prefs.remove(user_id).map_err(map_storage_err)?;
    let _ = cursors.remove(user_id).map_err(map_storage_err)?;
    Ok(deleted_notebooks)
}

fn trashed_row_matches_identifier(row: &TrashedUserRow, identifier_raw: &str) -> bool {
    let trimmed = identifier_raw.trim();
    if trimmed.is_empty() {
        return false;
    }
    if trimmed.contains('@') {
        let Ok((_, email_lower)) = normalize_email(trimmed) else {
            return false;
        };
        return row.user.email_lower.as_deref() == Some(email_lower.as_str());
    }
    let Ok((_, username_lower)) = normalize_username(trimmed) else {
        return false;
    };
    row.user.username_lower == username_lower
}

fn trashed_row_matches_id_or_identifier(row: &TrashedUserRow, raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return false;
    }
    row.user.user_id == trimmed || trashed_row_matches_identifier(row, trimmed)
}

fn lookup_trashed_user_by_email(
    rx: &redb::ReadTransaction,
    email_lower: &str,
) -> Result<Option<TrashedUserRow>, DbError> {
    let trashed = rx.open_table(TRASHED_USERS_TABLE).map_err(map_table_err)?;
    for item in trashed.iter().map_err(map_storage_err)? {
        let (_, v) = item.map_err(map_storage_err)?;
        let row: TrashedUserRow = from_json(v.value())?;
        if row.user.email_lower.as_deref() == Some(email_lower) {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

fn lookup_user_id_by_email(
    rx: &redb::ReadTransaction,
    email_lower: &str,
) -> Result<Option<String>, DbError> {
    let by_email = rx.open_table(USERS_BY_EMAIL_TABLE).map_err(map_table_err)?;
    Ok(by_email
        .get(email_lower)
        .map_err(map_storage_err)?
        .map(|g| g.value().to_string()))
}

fn lookup_trashed_user_by_id_or_identifier(
    rx: &redb::ReadTransaction,
    raw: &str,
) -> Result<Option<TrashedUserRow>, DbError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let trashed = rx.open_table(TRASHED_USERS_TABLE).map_err(map_table_err)?;
    if let Some(guard) = trashed.get(trimmed).map_err(map_storage_err)? {
        return Ok(Some(from_json(guard.value())?));
    }
    for item in trashed.iter().map_err(map_storage_err)? {
        let (_, v) = item.map_err(map_storage_err)?;
        let row: TrashedUserRow = from_json(v.value())?;
        if trashed_row_matches_id_or_identifier(&row, trimmed) {
            return Ok(Some(row));
        }
    }
    Ok(None)
}

fn lookup_user_id_by_id_or_identifier(
    rx: &redb::ReadTransaction,
    raw: &str,
) -> Result<Option<String>, DbError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let users = rx.open_table(USERS_TABLE).map_err(map_table_err)?;
    if users.get(trimmed).map_err(map_storage_err)?.is_some() {
        return Ok(Some(trimmed.to_string()));
    }
    lookup_user_id_by_identifier(rx, trimmed)
}

fn lookup_user_id_by_identifier(
    rx: &redb::ReadTransaction,
    identifier_raw: &str,
) -> Result<Option<String>, DbError> {
    let trimmed = identifier_raw.trim();
    if trimmed.contains('@') {
        let Ok((_, email_lower)) = normalize_email(trimmed) else {
            return Ok(None);
        };
        let by_email = rx.open_table(USERS_BY_EMAIL_TABLE).map_err(map_table_err)?;
        return Ok(by_email
            .get(email_lower.as_str())
            .map_err(map_storage_err)?
            .map(|g| g.value().to_string()));
    }
    let Ok((_, username_lower)) = normalize_username(trimmed) else {
        return Ok(None);
    };
    let by_name = rx.open_table(USERS_BY_NAME_TABLE).map_err(map_table_err)?;
    Ok(by_name
        .get(username_lower.as_str())
        .map_err(map_storage_err)?
        .map(|g| g.value().to_string()))
}

fn load_pulled_folders(
    rx: &redb::ReadTransaction,
    user_id: &str,
    since_cursor: u64,
) -> Result<Vec<RemoteFolderRecord>, DbError> {
    let table = rx.open_table(FOLDERS_TABLE).map_err(map_table_err)?;
    let prefix = user_notebook_prefix(user_id);
    let mut out = Vec::new();
    for item in table.iter().map_err(map_storage_err)? {
        let (k, v) = item.map_err(map_storage_err)?;
        if !k.value().starts_with(&prefix) {
            continue;
        }
        let row: StoredFolderRow = from_json(v.value())?;
        if row.cursor <= since_cursor {
            continue;
        }
        if since_cursor == 0 && row.deleted {
            continue;
        }
        out.push(RemoteFolderRecord {
            folder_id: row.folder_id,
            version: row.version,
            cursor: row.cursor,
            path: row.path,
            color_hex: row.color_hex,
            deleted: row.deleted,
            updated_at_ms: row.updated_at_ms,
            deleted_at_ms: row.deleted_at_ms,
        });
    }
    out.sort_by_key(|r| r.cursor);
    Ok(out)
}

#[allow(clippy::too_many_arguments)]
fn apply_single_folder_push(
    user_id: &str,
    folder_id: &str,
    change: LocalFolderChange,
    force_overwrite: bool,
    now: i64,
    current_cursor: &mut u64,
    folders: &mut redb::Table<&str, &str>,
    accepted: &mut Vec<AcceptedFolderSync>,
    conflicts: &mut Vec<RemoteFolderRecord>,
) -> Result<(), DbError> {
    let key = notebook_key(user_id, folder_id);
    let clean_path = sanitize_folder_path(&change.path);
    let existing = folders
        .get(key.as_str())
        .map_err(map_storage_err)?
        .map(|g| from_json::<StoredFolderRow>(g.value()))
        .transpose()?;
    let existing_ver = existing.as_ref().map_or(0, |r| r.version);

    if let Some(ref row) = existing {
        let same_state = row.deleted == change.deleted
            && (row.deleted || (row.path == clean_path && row.color_hex == change.color_hex));
        if same_state {
            accepted.push(AcceptedFolderSync {
                folder_id: folder_id.to_string(),
                new_version: row.version,
                cursor: row.cursor,
                deleted: row.deleted,
            });
            return Ok(());
        }
        if !force_overwrite
            && change.base_version != row.version
            && change.updated_at_ms <= row.updated_at_ms
        {
            conflicts.push(RemoteFolderRecord {
                folder_id: row.folder_id.clone(),
                version: row.version,
                cursor: row.cursor,
                path: row.path.clone(),
                color_hex: row.color_hex.clone(),
                deleted: row.deleted,
                updated_at_ms: row.updated_at_ms,
                deleted_at_ms: row.deleted_at_ms,
            });
            return Ok(());
        }
    } else if change.deleted {
        accepted.push(AcceptedFolderSync {
            folder_id: folder_id.to_string(),
            new_version: 0,
            cursor: *current_cursor,
            deleted: true,
        });
        return Ok(());
    }

    *current_cursor = current_cursor.saturating_add(1);
    let new_ver = existing_ver.saturating_add(1);
    let next = StoredFolderRow {
        folder_id: folder_id.to_string(),
        user_id: user_id.to_string(),
        path: clean_path,
        color_hex: change.color_hex,
        version: new_ver,
        cursor: *current_cursor,
        deleted: change.deleted,
        updated_at_ms: change.updated_at_ms.max(now),
        deleted_at_ms: if change.deleted {
            Some(change.deleted_at_ms.unwrap_or(now))
        } else {
            None
        },
    };
    let encoded = to_json(&next)?;
    folders
        .insert(key.as_str(), encoded.as_str())
        .map_err(map_storage_err)?;
    accepted.push(AcceptedFolderSync {
        folder_id: folder_id.to_string(),
        new_version: new_ver,
        cursor: *current_cursor,
        deleted: change.deleted,
    });
    Ok(())
}

fn issue_tokens(
    user: &UserRow,
    device_name: &str,
    now: i64,
    ttls: TokenTtls,
    access: &mut redb::Table<&str, &str>,
    refresh: &mut redb::Table<&str, &str>,
) -> Result<AuthResponse, DbError> {
    let access_token = generate_secret_token("nca")?;
    let refresh_token = generate_secret_token("ncr")?;
    let access_hash = sha256_hex(access_token.as_bytes());
    let refresh_hash = sha256_hex(refresh_token.as_bytes());

    let expires_at_ms = now.saturating_add(ttls.access_ttl_ms);
    let refresh_expires_at_ms = now.saturating_add(ttls.refresh_ttl_ms);

    let access_row = AccessTokenRow {
        token_hash: access_hash.clone(),
        user_id: user.user_id.clone(),
        created_at_ms: now,
        expires_at_ms,
    };
    let refresh_row = RefreshTokenRow {
        token_hash: refresh_hash.clone(),
        user_id: user.user_id.clone(),
        device_name: device_name.to_string(),
        created_at_ms: now,
        expires_at_ms: refresh_expires_at_ms,
    };

    let access_json = to_json(&access_row)?;
    let refresh_json = to_json(&refresh_row)?;
    access
        .insert(access_hash.as_str(), access_json.as_str())
        .map_err(map_storage_err)?;
    refresh
        .insert(refresh_hash.as_str(), refresh_json.as_str())
        .map_err(map_storage_err)?;

    Ok(AuthResponse {
        access_token,
        refresh_token,
        expires_at_ms,
        refresh_expires_at_ms,
        user: user.to_profile(),
    })
}

fn prune_expired_tokens(
    access: &mut redb::Table<&str, &str>,
    refresh: &mut redb::Table<&str, &str>,
    user_id: &str,
    now: i64,
) -> Result<(), DbError> {
    let mut expired_access = Vec::new();
    for item in access.iter().map_err(map_storage_err)? {
        let (k, v) = item.map_err(map_storage_err)?;
        let row: AccessTokenRow = from_json(v.value())?;
        if row.expires_at_ms <= now {
            expired_access.push(k.value().to_string());
            if expired_access.len() >= MAX_TOKENS_PER_USER_PRUNE {
                break;
            }
        }
    }
    for k in expired_access {
        let _ = access.remove(k.as_str()).map_err(map_storage_err)?;
    }

    let mut expired_refresh = Vec::new();
    let mut user_refresh_tokens: Vec<(i64, String)> = Vec::new();
    for item in refresh.iter().map_err(map_storage_err)? {
        let (k, v) = item.map_err(map_storage_err)?;
        let row: RefreshTokenRow = from_json(v.value())?;
        if row.expires_at_ms <= now {
            expired_refresh.push(k.value().to_string());
        } else if row.user_id == user_id {
            user_refresh_tokens.push((row.created_at_ms, k.value().to_string()));
        }
    }
    for k in expired_refresh {
        let _ = refresh.remove(k.as_str()).map_err(map_storage_err)?;
    }
    if user_refresh_tokens.len() > 24 {
        user_refresh_tokens.sort_by_key(|(ts, _)| *ts);
        let remove_count = user_refresh_tokens.len().saturating_sub(24);
        for (_, k) in user_refresh_tokens.into_iter().take(remove_count) {
            let _ = refresh.remove(k.as_str()).map_err(map_storage_err)?;
        }
    }
    Ok(())
}

fn revoke_all_user_tokens(
    access: &mut redb::Table<&str, &str>,
    refresh: &mut redb::Table<&str, &str>,
    user_id: &str,
) -> Result<(), DbError> {
    let mut access_keys = Vec::new();
    for item in access.iter().map_err(map_storage_err)? {
        let (k, v) = item.map_err(map_storage_err)?;
        let row: AccessTokenRow = from_json(v.value())?;
        if row.user_id == user_id {
            access_keys.push(k.value().to_string());
        }
    }
    for k in access_keys {
        let _ = access.remove(k.as_str()).map_err(map_storage_err)?;
    }

    let mut refresh_keys = Vec::new();
    for item in refresh.iter().map_err(map_storage_err)? {
        let (k, v) = item.map_err(map_storage_err)?;
        let row: RefreshTokenRow = from_json(v.value())?;
        if row.user_id == user_id {
            refresh_keys.push(k.value().to_string());
        }
    }
    for k in refresh_keys {
        let _ = refresh.remove(k.as_str()).map_err(map_storage_err)?;
    }
    Ok(())
}

fn generate_temporary_password() -> Result<String, DbError> {
    const ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let mut bytes = [0u8; 12];
    getrandom::fill(&mut bytes).map_err(|e| {
        DbError::Storage(format!("CSPRNG failure generating temporary password: {e}"))
    })?;
    let mut out = String::with_capacity(14);
    for (idx, byte) in bytes.into_iter().enumerate() {
        if idx == 4 || idx == 8 {
            out.push('-');
        }
        let ch = ALPHABET[usize::from(byte & 31)];
        out.push(char::from(ch));
    }
    Ok(out)
}

fn generate_secret_token(prefix: &str) -> Result<String, DbError> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)
        .map_err(|e| DbError::Storage(format!("CSPRNG failure generating token: {e}")))?;
    Ok(format!("{prefix}_{}", bytes_to_hex(&bytes)))
}

fn hash_password(password: &str) -> Result<String, DbError> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| DbError::Storage(format!("password hashing failed: {e}")))
}

fn verify_password(password: &str, phc_hash: &str) -> Result<(), DbError> {
    let parsed = PasswordHash::new(phc_hash).map_err(|_| DbError::InvalidCredentials)?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| DbError::InvalidCredentials)
}

fn user_notebook_prefix(user_id: &str) -> String {
    format!("{user_id}\0")
}

fn notebook_key(user_id: &str, notebook_id: &str) -> String {
    format!("{user_id}\0{notebook_id}")
}

fn to_json<T: Serialize>(value: &T) -> Result<String, DbError> {
    serde_json::to_string(value).map_err(|e| DbError::Storage(format!("serialization error: {e}")))
}

fn from_json<T: for<'de> Deserialize<'de>>(raw: &str) -> Result<T, DbError> {
    serde_json::from_str(raw).map_err(|e| DbError::Storage(format!("corrupt database row: {e}")))
}

fn map_tx_err(e: redb::TransactionError) -> DbError {
    DbError::Storage(e.to_string())
}

fn map_table_err(e: redb::TableError) -> DbError {
    DbError::Storage(e.to_string())
}

fn map_storage_err(e: redb::StorageError) -> DbError {
    DbError::Storage(e.to_string())
}

fn map_commit_err(e: redb::CommitError) -> DbError {
    DbError::Storage(e.to_string())
}
