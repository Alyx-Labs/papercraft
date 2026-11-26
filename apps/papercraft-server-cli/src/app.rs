use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::extract::{DefaultBodyLimit, Request};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use papercraft_sync::{inspect_local_accounts_in_library, sanitize_client_id};
use papercraft_sync_protocol::{
    ApiErrorBody, ERR_CODE_ACCOUNT_IN_TRASH, ERR_CODE_EMAIL_IN_TRASH,
    ERR_CODE_PASSWORD_CHANGE_REQUIRED, ERR_CODE_PASSWORD_RESET_REQUIRED,
    ERR_CODE_USERNAME_IN_TRASH, LocalDiscoveryReport, LocalDiscoverySummary,
};

use crate::auth;
use crate::config::ServerConfig;
use crate::db::{DbError, ServerDb, TokenTtls};
use crate::sync_routes;

const MAX_RATE_LIMIT_BUCKETS: usize = 4096;
const DISCOVERY_TTL: Duration = Duration::from_secs(120);
const MAX_DISCOVERY_CLIENTS: usize = 1024;
const MAX_REPORTED_ACCOUNTS_PER_CLIENT: usize = 512;
const MAX_REPORTED_NOTEBOOKS_PER_CLIENT: usize = 16_384;

#[derive(Clone, Copy)]
struct ReportedClientDiscovery {
    unsynced_local_accounts: usize,
    total_local_notebooks: usize,
    updated_at: Instant,
}

/// Privacy-preserving registry aggregating ONLY counts of unsynced local accounts and notebooks.
pub struct LocalDiscoveryRegistry {
    clients: Mutex<HashMap<String, ReportedClientDiscovery>>,
    sibling_roots: Vec<PathBuf>,
}

impl LocalDiscoveryRegistry {
    pub fn for_config(config: &ServerConfig) -> Self {
        Self::new(&config.data_dir, &config.extra_library_roots)
    }

    pub fn new(data_dir: &Path, extra_roots: &[PathBuf]) -> Self {
        let mut sibling_roots = Vec::new();
        if let Some(parent) = data_dir.parent().filter(|p| !p.as_os_str().is_empty()) {
            push_unique_root(&mut sibling_roots, parent.join("library"));
        }
        for root in extra_roots {
            push_unique_root(&mut sibling_roots, root.clone());
        }
        Self {
            clients: Mutex::new(HashMap::new()),
            sibling_roots,
        }
    }

    pub fn record_report(
        &self,
        report: LocalDiscoveryReport,
    ) -> Result<LocalDiscoverySummary, ApiError> {
        let client_id = sanitize_client_id(&report.client_id);
        if client_id.is_empty() {
            return Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                "invalid_client_id",
                "Client identifier cannot be empty.",
            ));
        }
        let now = Instant::now();
        if let Ok(mut guard) = self.clients.lock() {
            guard.retain(|_, v| now.duration_since(v.updated_at) <= DISCOVERY_TTL);
            if guard.len() >= MAX_DISCOVERY_CLIENTS && !guard.contains_key(&client_id) {
                guard.clear();
            }
            guard.insert(
                client_id,
                ReportedClientDiscovery {
                    unsynced_local_accounts: report
                        .unsynced_local_accounts
                        .min(MAX_REPORTED_ACCOUNTS_PER_CLIENT),
                    total_local_notebooks: report
                        .total_local_notebooks
                        .min(MAX_REPORTED_NOTEBOOKS_PER_CLIENT),
                    updated_at: now,
                },
            );
        }
        Ok(self.summary())
    }

    pub fn summary(&self) -> LocalDiscoverySummary {
        let now = Instant::now();
        let mut by_client: HashMap<String, LocalDiscoverySummary> = HashMap::new();

        for root in &self.sibling_roots {
            if let Some((cid, local_summary)) = inspect_local_accounts_in_library(root) {
                let key = sanitize_client_id(&cid);
                if !key.is_empty() {
                    by_client.insert(key, local_summary);
                }
            }
        }

        if let Ok(mut guard) = self.clients.lock() {
            guard.retain(|_, v| now.duration_since(v.updated_at) <= DISCOVERY_TTL);
            for (cid, entry) in guard.iter() {
                by_client.insert(
                    cid.clone(),
                    LocalDiscoverySummary {
                        unsynced_local_accounts: entry.unsynced_local_accounts,
                        total_local_notebooks: entry.total_local_notebooks,
                    },
                );
            }
        }

        let mut total = LocalDiscoverySummary::default();
        for item in by_client.into_values() {
            total.unsynced_local_accounts = total
                .unsynced_local_accounts
                .saturating_add(item.unsynced_local_accounts);
            total.total_local_notebooks = total
                .total_local_notebooks
                .saturating_add(item.total_local_notebooks);
        }
        total
    }
}

fn push_unique_root(list: &mut Vec<PathBuf>, candidate: PathBuf) {
    let norm = std::fs::canonicalize(&candidate).unwrap_or_else(|_| candidate.clone());
    let exists = list.iter().any(|existing| {
        let existing_norm = std::fs::canonicalize(existing).unwrap_or_else(|_| existing.clone());
        existing_norm == norm
    });
    if !exists {
        list.push(candidate);
    }
}

/// Inspects co-located client library folders when a server binary is offline or printing CLI status.
pub fn discover_local_accounts_on_disk(
    data_dir: &Path,
    extra_roots: &[PathBuf],
) -> LocalDiscoverySummary {
    LocalDiscoveryRegistry::new(data_dir, extra_roots).summary()
}

#[derive(Clone)]
pub struct AppState {
    pub db: ServerDb,
    pub config: Arc<ServerConfig>,
    pub rate_limiter: Arc<AuthRateLimiter>,
    pub discovery: Arc<LocalDiscoveryRegistry>,
}

impl AppState {
    pub fn token_ttls(&self) -> TokenTtls {
        TokenTtls {
            access_ttl_ms: self.config.access_token_ttl_ms,
            refresh_ttl_ms: self.config.refresh_token_ttl_ms,
        }
    }
}

pub struct AuthRateLimiter {
    window: Duration,
    max_attempts: usize,
    buckets: Mutex<HashMap<String, Vec<Instant>>>,
}

impl AuthRateLimiter {
    pub fn new(window: Duration, max_attempts: usize) -> Self {
        Self {
            window,
            max_attempts,
            buckets: Mutex::new(HashMap::new()),
        }
    }

    pub fn check_and_record(&self, key: &str) -> Result<(), ApiError> {
        let now = Instant::now();
        let Ok(mut guard) = self.buckets.lock() else {
            return Ok(());
        };
        if guard.len() >= MAX_RATE_LIMIT_BUCKETS && !guard.contains_key(key) {
            guard.retain(|_, times| {
                times.retain(|t| now.duration_since(*t) < self.window);
                !times.is_empty()
            });
            if guard.len() >= MAX_RATE_LIMIT_BUCKETS {
                guard.clear();
            }
        }
        let entries = guard.entry(key.to_string()).or_default();
        entries.retain(|t| now.duration_since(*t) < self.window);
        if entries.len() >= self.max_attempts {
            return Err(ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "Too many authentication attempts. Please wait a moment and try again.",
            ));
        }
        entries.push(now);
        Ok(())
    }
}

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: String,
    pub message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status,
            code: code.into(),
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
    }
}

impl From<DbError> for ApiError {
    fn from(err: DbError) -> Self {
        match err {
            DbError::Validation(msg) => Self::new(StatusCode::BAD_REQUEST, "validation_error", msg),
            DbError::RegistrationClosed => Self::new(
                StatusCode::FORBIDDEN,
                "registration_closed",
                "Account registration is disabled on this server.",
            ),
            DbError::UsernameTaken(name) => Self::new(
                StatusCode::CONFLICT,
                "username_taken",
                format!("Username `{name}` is already registered."),
            ),
            DbError::EmailTaken(email) => Self::new(
                StatusCode::CONFLICT,
                "email_taken",
                format!("An account with email `{email}` already exists."),
            ),
            DbError::AccountInTrash => Self::new(
                StatusCode::FORBIDDEN,
                ERR_CODE_ACCOUNT_IN_TRASH,
                "This account is in the server Trash and scheduled for permanent deletion. Ask your server administrator to restore it.",
            ),
            DbError::EmailInTrash(email) => Self::new(
                StatusCode::CONFLICT,
                ERR_CODE_EMAIL_IN_TRASH,
                format!(
                    "An account with email `{email}` is in the server Trash. Restore it from Trash or wait for the 30-day retention period to expire."
                ),
            ),
            DbError::UsernameInTrash(name) => Self::new(
                StatusCode::CONFLICT,
                ERR_CODE_USERNAME_IN_TRASH,
                format!(
                    "Username `{name}` belongs to an account in the server Trash. Restore it from Trash or wait for the 30-day retention period to expire."
                ),
            ),
            DbError::TrashedUserExpired(name) => Self::new(
                StatusCode::GONE,
                "trashed_user_expired",
                format!("Trashed account `{name}` has expired and was permanently deleted."),
            ),
            DbError::InvalidCredentials => Self::new(
                StatusCode::UNAUTHORIZED,
                "invalid_credentials",
                "Invalid email/username or password.",
            ),
            DbError::IncorrectCurrentPassword => Self::new(
                StatusCode::BAD_REQUEST,
                "incorrect_current_password",
                "Current password is incorrect.",
            ),
            DbError::PasswordResetRequired => Self::new(
                StatusCode::FORBIDDEN,
                ERR_CODE_PASSWORD_RESET_REQUIRED,
                "Your password was reset on the server. Enter the generated recovery code to choose a new password.",
            ),
            DbError::PasswordChangeRequired => Self::new(
                StatusCode::FORBIDDEN,
                ERR_CODE_PASSWORD_CHANGE_REQUIRED,
                "Temporary password verified. Choose a new password to finish signing in.",
            ),
            DbError::Unauthorized => Self::new(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Authentication token is missing, expired, or revoked.",
            ),
            DbError::UserNotFound(name) => Self::new(
                StatusCode::NOT_FOUND,
                "user_not_found",
                format!("User `{name}` was not found."),
            ),
            DbError::PayloadTooLarge {
                title,
                actual_bytes,
                limit_bytes,
            } => {
                let actual_mb = actual_bytes as f64 / (1024.0 * 1024.0);
                let limit_mb = limit_bytes as f64 / (1024.0 * 1024.0);
                Self::new(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "payload_too_large",
                    format!(
                        "Notebook \"{title}\" ({actual_mb:.1} MiB) exceeds server limit of {limit_mb:.0} MiB. Start the server with a higher --max-notebook-mb value."
                    ),
                )
            }
            DbError::Storage(msg) => {
                log::error!("Database error: {msg}");
                Self::internal("Internal server database error.")
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ApiErrorBody {
            code: self.code,
            message: self.message,
        };
        (self.status, Json(body)).into_response()
    }
}

async fn cors_middleware(req: Request, next: Next) -> Response {
    let mut response = if req.method() == Method::OPTIONS {
        StatusCode::NO_CONTENT.into_response()
    } else {
        next.run(req).await
    };
    let headers = response.headers_mut();
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_ORIGIN,
        HeaderValue::from_static("*"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Authorization, Content-Type"),
    );
    headers.insert(
        header::ACCESS_CONTROL_MAX_AGE,
        HeaderValue::from_static("86400"),
    );
    response
}

pub fn build_router(state: AppState) -> Router {
    let body_limit = state.config.max_request_bytes;
    Router::new()
        .route("/api/v1/health", get(sync_routes::health))
        .route(
            "/api/v1/discovery/local-accounts",
            post(sync_routes::report_local_discovery),
        )
        .route("/api/v1/auth/register", post(auth::register))
        .route("/api/v1/auth/login", post(auth::login))
        .route(
            "/api/v1/auth/complete-password-reset",
            post(auth::complete_password_reset),
        )
        .route("/api/v1/auth/check-email", post(auth::check_email))
        .route("/api/v1/auth/change-email", post(auth::change_email))
        .route("/api/v1/auth/update-profile", post(auth::update_profile))
        .route("/api/v1/auth/change-password", post(auth::change_password))
        .route("/api/v1/auth/refresh", post(auth::refresh))
        .route("/api/v1/auth/logout", post(auth::logout))
        .route("/api/v1/auth/me", get(auth::me))
        .route("/api/v1/sync/pull", get(sync_routes::sync_pull))
        .route("/api/v1/sync/push", post(sync_routes::sync_push))
        .route(
            "/api/v1/sync/prune-to-local",
            post(sync_routes::sync_prune_to_local),
        )
        .layer(DefaultBodyLimit::max(body_limit))
        .layer(middleware::from_fn(cors_middleware))
        .with_state(state)
}
