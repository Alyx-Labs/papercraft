use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use papercraft_sync_protocol::{PROTOCOL_VERSION, now_unix_ms};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const DEFAULT_BIND_ADDR: &str = "127.0.0.1:8787";
pub const DEFAULT_DATA_DIR: &str = "papercraft-server-data";
pub const DEFAULT_ACCESS_TTL_HOURS: u64 = 24;
pub const DEFAULT_REFRESH_TTL_DAYS: u64 = 30;
pub const DEFAULT_MAX_NOTEBOOK_MB: usize = 128;
const SERVER_META_FILE: &str = "server_meta.json";

#[derive(Clone, Debug)]
pub struct ServerConfig {
    pub data_dir: PathBuf,
    pub bind_addr: SocketAddr,
    pub registration_open: bool,
    pub access_token_ttl_ms: i64,
    pub refresh_token_ttl_ms: i64,
    pub max_notebook_bytes: usize,
    pub max_request_bytes: usize,
    pub extra_library_roots: Vec<PathBuf>,
}

impl ServerConfig {
    /// Builds a validated [`ServerConfig`] with clamped lifetimes and payload bounds.
    pub fn from_options(
        data_dir: PathBuf,
        bind_addr: SocketAddr,
        registration_open: bool,
        access_ttl_hours: u64,
        refresh_ttl_days: u64,
        max_notebook_mb: usize,
    ) -> Self {
        let access_hours = access_ttl_hours.clamp(1, 24 * 365);
        let refresh_days = refresh_ttl_days.clamp(1, 3650);
        let max_nb_mb = max_notebook_mb.clamp(1, 512);
        let max_notebook_bytes = max_nb_mb.saturating_mul(1024 * 1024);
        let max_request_bytes = max_notebook_bytes.saturating_mul(2).max(16 * 1024 * 1024);

        Self {
            data_dir,
            bind_addr,
            registration_open,
            access_token_ttl_ms: (access_hours as i64).saturating_mul(3_600_000),
            refresh_token_ttl_ms: (refresh_days as i64).saturating_mul(86_400_000),
            max_notebook_bytes,
            max_request_bytes,
            extra_library_roots: Vec::new(),
        }
    }

    pub fn db_path(&self) -> PathBuf {
        resolve_db_path(&self.data_dir)
    }
}

/// Resolves the database path in `data_dir`, keeping existing legacy `notecraft.redb` vaults readable.
pub fn resolve_db_path(data_dir: &Path) -> PathBuf {
    let primary = data_dir.join("papercraft.redb");
    if !primary.exists() {
        let legacy = data_dir.join("notecraft.redb");
        if legacy.exists() {
            return legacy;
        }
    }
    primary
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServerMeta {
    pub server_id: String,
    pub created_at_ms: i64,
    pub protocol_version: u32,
}

/// Ensures `data_dir` exists and loads or initializes `server_meta.json`.
pub fn ensure_data_dir_and_meta(data_dir: &Path) -> Result<ServerMeta, String> {
    fs::create_dir_all(data_dir).map_err(|err| {
        format!(
            "failed to create server data directory `{}`: {err}",
            data_dir.display()
        )
    })?;

    let meta_path = data_dir.join(SERVER_META_FILE);
    if meta_path.exists() {
        let raw = fs::read_to_string(&meta_path)
            .map_err(|err| format!("failed to read `{}`: {err}", meta_path.display()))?;
        let parsed: ServerMeta = serde_json::from_str(&raw)
            .map_err(|err| format!("invalid `{}`: {err}", meta_path.display()))?;
        return Ok(parsed);
    }

    let meta = ServerMeta {
        server_id: Uuid::new_v4().to_string(),
        created_at_ms: now_unix_ms(),
        protocol_version: PROTOCOL_VERSION,
    };
    let json = serde_json::to_string_pretty(&meta)
        .map_err(|err| format!("failed to serialize server metadata: {err}"))?;
    fs::write(&meta_path, json.as_bytes())
        .map_err(|err| format!("failed to write `{}`: {err}", meta_path.display()))?;
    Ok(meta)
}
