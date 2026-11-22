use axum::Json;
use axum::extract::{Query, State};
use papercraft_sync_protocol::{
    LocalDiscoveryReport, LocalDiscoverySummary, PROTOCOL_VERSION, PruneToLocalRequest,
    PruneToLocalResponse, ServerInfoResponse, SyncPullResponse, SyncPushRequest, SyncPushResponse,
};
use serde::Deserialize;

use crate::app::{ApiError, AppState};
use crate::auth::AuthenticatedUser;

#[derive(Default, Deserialize)]
pub struct PullQuery {
    #[serde(default)]
    pub since: u64,
}

pub async fn health(State(state): State<AppState>) -> Json<ServerInfoResponse> {
    Json(ServerInfoResponse {
        status: "ok".to_string(),
        service: "papercraft-server".to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        protocol_version: PROTOCOL_VERSION,
        registration_open: state.config.registration_open,
        storage_engine: "redb".to_string(),
        max_notebook_bytes: state.config.max_notebook_bytes,
        local_discovery: state.discovery.summary(),
    })
}

pub async fn report_local_discovery(
    State(state): State<AppState>,
    Json(report): Json<LocalDiscoveryReport>,
) -> Result<Json<LocalDiscoverySummary>, ApiError> {
    Ok(Json(state.discovery.record_report(report)?))
}

pub async fn sync_pull(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Query(query): Query<PullQuery>,
) -> Result<Json<SyncPullResponse>, ApiError> {
    let db = state.db.clone();
    let user_id = user.profile.user_id;
    let since = query.since;
    let payload = tokio::task::spawn_blocking(move || db.pull_changes(&user_id, since))
        .await
        .map_err(|e| ApiError::internal(format!("sync pull task failed: {e}")))?
        .map_err(ApiError::from)?;

    Ok(Json(payload))
}

pub async fn sync_push(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(req): Json<SyncPushRequest>,
) -> Result<Json<SyncPushResponse>, ApiError> {
    let db = state.db.clone();
    let user_id = user.profile.user_id;
    let max_bytes = state.config.max_notebook_bytes;
    let res = tokio::task::spawn_blocking(move || db.push_changes(&user_id, req, max_bytes))
        .await
        .map_err(|e| ApiError::internal(format!("sync push task failed: {e}")))?
        .map_err(ApiError::from)?;

    log::debug!(
        "User `{}` pushed updates: accepted={}, conflicts={}, cursor={}",
        user.profile.username,
        res.accepted.len(),
        res.conflicts.len(),
        res.server_cursor
    );
    Ok(Json(res))
}

pub async fn sync_prune_to_local(
    State(state): State<AppState>,
    user: AuthenticatedUser,
    Json(req): Json<PruneToLocalRequest>,
) -> Result<Json<PruneToLocalResponse>, ApiError> {
    let db = state.db.clone();
    let user_id = user.profile.user_id;
    let res = tokio::task::spawn_blocking(move || db.prune_to_local(&user_id, req))
        .await
        .map_err(|e| ApiError::internal(format!("sync prune task failed: {e}")))?
        .map_err(ApiError::from)?;
    Ok(Json(res))
}
