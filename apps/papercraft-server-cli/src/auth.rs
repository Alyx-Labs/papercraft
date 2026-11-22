use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};

use axum::Json;
use axum::extract::{ConnectInfo, FromRequestParts, State};
use axum::http::StatusCode;
use axum::http::header::AUTHORIZATION;
use axum::http::request::Parts;
use papercraft_sync_protocol::{
    AuthResponse, ChangeEmailRequest, ChangePasswordRequest, CheckEmailRequest, CheckEmailResponse,
    CompletePasswordResetRequest, LoginRequest, LogoutRequest, RefreshRequest, RegisterRequest,
    UpdateProfileRequest, UserProfile,
};

use crate::app::{ApiError, AppState};

pub struct MaybeClientIp(pub Option<IpAddr>);

impl<S> FromRequestParts<S> for MaybeClientIp
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let ip = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|c| c.0.ip());
        Ok(Self(ip))
    }
}

pub struct AuthenticatedUser {
    pub profile: UserProfile,
}

impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = extract_bearer_token(&parts.headers).ok_or_else(|| {
            ApiError::new(
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                "Missing Bearer authentication token.",
            )
        })?;
        let db = state.db.clone();
        let profile = tokio::task::spawn_blocking(move || db.authenticate_access_token(&token))
            .await
            .map_err(|e| ApiError::internal(format!("auth task failed: {e}")))?
            .map_err(ApiError::from)?;
        Ok(Self { profile })
    }
}

pub fn extract_bearer_token(headers: &axum::http::HeaderMap) -> Option<String> {
    let raw = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let trimmed = raw.trim();
    let token = trimmed
        .strip_prefix("Bearer ")
        .or_else(|| trimmed.strip_prefix("bearer "))?;
    let clean = token.trim();
    if clean.is_empty() {
        None
    } else {
        Some(clean.to_string())
    }
}

fn rate_limit_key(ip: MaybeClientIp, action: &str) -> String {
    match ip.0 {
        Some(ip) => format!("{action}:{ip}"),
        None => format!("{action}:local"),
    }
}

pub async fn register(
    State(state): State<AppState>,
    ip: MaybeClientIp,
    Json(req): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<AuthResponse>), ApiError> {
    state
        .rate_limiter
        .check_and_record(&rate_limit_key(ip, "register"))?;

    let db = state.db.clone();
    let reg_open = state.config.registration_open;
    let ttls = state.token_ttls();
    let auth = tokio::task::spawn_blocking(move || {
        db.register_user_with_email(
            req.email.as_deref(),
            &req.username,
            &req.password,
            req.display_name.as_deref(),
            req.device_name.as_deref(),
            reg_open,
            ttls,
        )
    })
    .await
    .map_err(|e| ApiError::internal(format!("registration task failed: {e}")))?
    .map_err(ApiError::from)?;

    log::info!(
        "Registered new user `{}` (id={})",
        auth.user.primary_label(),
        auth.user.user_id
    );
    Ok((StatusCode::CREATED, Json(auth)))
}

pub async fn login(
    State(state): State<AppState>,
    ip: MaybeClientIp,
    Json(req): Json<LoginRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    state
        .rate_limiter
        .check_and_record(&rate_limit_key(ip, "login"))?;

    let db = state.db.clone();
    let ttls = state.token_ttls();
    let identifier = req
        .email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&req.username)
        .to_string();
    let auth = tokio::task::spawn_blocking(move || {
        db.login_user(&identifier, &req.password, req.device_name.as_deref(), ttls)
    })
    .await
    .map_err(|e| ApiError::internal(format!("login task failed: {e}")))?
    .map_err(ApiError::from)?;

    log::info!("User `{}` signed in", auth.user.primary_label());
    Ok(Json(auth))
}

pub async fn check_email(
    State(state): State<AppState>,
    ip: MaybeClientIp,
    Json(req): Json<CheckEmailRequest>,
) -> Result<Json<CheckEmailResponse>, ApiError> {
    state
        .rate_limiter
        .check_and_record(&rate_limit_key(ip, "check_email"))?;

    let db = state.db.clone();
    let reg_open = state.config.registration_open;
    let resp = tokio::task::spawn_blocking(move || db.check_email_exists(&req.email, reg_open))
        .await
        .map_err(|e| ApiError::internal(format!("email check failed: {e}")))?
        .map_err(ApiError::from)?;
    Ok(Json(resp))
}

pub async fn change_email(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(req): Json<ChangeEmailRequest>,
) -> Result<Json<UserProfile>, ApiError> {
    let db = state.db.clone();
    let user_id = user.profile.user_id;
    let updated = tokio::task::spawn_blocking(move || {
        db.change_email(&user_id, &req.current_password, &req.new_email)
    })
    .await
    .map_err(|e| ApiError::internal(format!("change email task failed: {e}")))?
    .map_err(ApiError::from)?;
    Ok(Json(updated))
}

pub async fn refresh(
    State(state): State<AppState>,
    ip: MaybeClientIp,
    Json(req): Json<RefreshRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    state
        .rate_limiter
        .check_and_record(&rate_limit_key(ip, "refresh"))?;

    let db = state.db.clone();
    let ttls = state.token_ttls();
    let auth = tokio::task::spawn_blocking(move || db.refresh_session(&req.refresh_token, ttls))
        .await
        .map_err(|e| ApiError::internal(format!("refresh task failed: {e}")))?
        .map_err(ApiError::from)?;

    Ok(Json(auth))
}

pub async fn logout(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(req): Json<LogoutRequest>,
) -> Result<StatusCode, ApiError> {
    let access_token = extract_bearer_token(&headers);
    let db = state.db.clone();
    tokio::task::spawn_blocking(move || {
        db.logout(access_token.as_deref(), req.refresh_token.as_deref())
    })
    .await
    .map_err(|e| ApiError::internal(format!("logout task failed: {e}")))?
    .map_err(ApiError::from)?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn me(user: AuthenticatedUser) -> Json<UserProfile> {
    Json(user.profile)
}

pub async fn complete_password_reset(
    State(state): State<AppState>,
    ip: MaybeClientIp,
    Json(req): Json<CompletePasswordResetRequest>,
) -> Result<Json<AuthResponse>, ApiError> {
    state
        .rate_limiter
        .check_and_record(&rate_limit_key(ip, "complete_password_reset"))?;

    let db = state.db.clone();
    let ttls = state.token_ttls();
    let identifier = req
        .email
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(&req.username)
        .to_string();
    let auth = tokio::task::spawn_blocking(move || {
        db.complete_password_reset(
            &identifier,
            &req.recovery_code,
            &req.new_password,
            req.device_name.as_deref(),
            ttls,
        )
    })
    .await
    .map_err(|e| ApiError::internal(format!("password reset task failed: {e}")))?
    .map_err(ApiError::from)?;

    log::info!(
        "User `{}` completed temporary password reset",
        auth.user.primary_label()
    );
    Ok(Json(auth))
}

pub async fn update_profile(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(req): Json<UpdateProfileRequest>,
) -> Result<Json<UserProfile>, ApiError> {
    let db = state.db.clone();
    let user_id = user.profile.user_id;
    let updated = tokio::task::spawn_blocking(move || {
        db.update_profile(
            &user_id,
            req.username.as_deref(),
            req.display_name.as_deref(),
        )
    })
    .await
    .map_err(|e| ApiError::internal(format!("update profile task failed: {e}")))?
    .map_err(ApiError::from)?;
    Ok(Json(updated))
}

pub async fn change_password(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<Json<UserProfile>, ApiError> {
    let db = state.db.clone();
    let user_id = user.profile.user_id;
    let updated = tokio::task::spawn_blocking(move || {
        db.change_password(&user_id, &req.current_password, &req.new_password)
    })
    .await
    .map_err(|e| ApiError::internal(format!("change password task failed: {e}")))?
    .map_err(ApiError::from)?;
    Ok(Json(updated))
}
