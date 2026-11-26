#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod app;
pub mod auth;
pub mod config;
pub mod db;
pub mod sync_routes;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

pub use papercraft_sync_protocol::LocalDiscoverySummary;

use crate::app::{
    AppState, AuthRateLimiter, LocalDiscoveryRegistry, build_router,
    discover_local_accounts_on_disk,
};
use crate::config::{ServerConfig, ensure_data_dir_and_meta};
use crate::db::{
    DbStats, DeletedUserSummary, PasswordResetOutcome, RestoredUserSummary, ServerDb,
    TrashedUserActionSummary, TrashedUserSummary, UserSummary,
};

const TRASH_CLEANUP_INTERVAL: Duration = Duration::from_secs(3600);

pub type ServerStoppedCallback = Box<dyn FnOnce(Result<(), String>) + Send>;

/// Metadata describing an active in-process sync server instance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunningServerInfo {
    pub server_id: String,
    pub data_dir: PathBuf,
    pub db_path: PathBuf,
    pub bound_addr: SocketAddr,
    pub registration_open: bool,
    pub max_notebook_bytes: usize,
}

/// Lifecycle handle for an embedded [`papercraft_server_cli`] HTTP sync runtime.
pub struct ServerRuntime {
    info: RunningServerInfo,
    db: Option<ServerDb>,
    discovery: Arc<LocalDiscoveryRegistry>,
    shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
    join: Option<JoinHandle<Result<(), String>>>,
}

impl ServerRuntime {
    /// Starts the HTTP sync server on a dedicated background Tokio runtime thread.
    ///
    /// If `on_stopped` is provided, it is invoked once the started server exits.
    pub fn start(
        config: ServerConfig,
        on_stopped: Option<ServerStoppedCallback>,
    ) -> Result<Self, String> {
        let meta = ensure_data_dir_and_meta(&config.data_dir)?;
        let db_path = config.db_path();
        let db = ServerDb::open(&db_path).map_err(|e| format_db_open_error(&db_path, &e))?;
        let discovery = Arc::new(LocalDiscoveryRegistry::for_config(&config));

        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<SocketAddr, String>>(1);
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let thread_db = db.clone();
        let thread_config = config.clone();
        let thread_discovery = Arc::clone(&discovery);
        let join = std::thread::Builder::new()
            .name("papercraft-server-rt".to_string())
            .spawn(move || {
                run_runtime_thread(
                    thread_db,
                    thread_config,
                    thread_discovery,
                    ready_tx,
                    shutdown_rx,
                    on_stopped,
                )
            })
            .map_err(|e| format!("Failed to spawn server runtime thread: {e}"))?;

        let bound_addr = match ready_rx.recv() {
            Ok(Ok(addr)) => addr,
            Ok(Err(err)) => {
                let _ = join.join();
                drop(db);
                return Err(err);
            }
            Err(_) => {
                let _ = join.join();
                drop(db);
                return Err("Server runtime exited unexpectedly during startup.".to_string());
            }
        };

        Ok(Self {
            info: RunningServerInfo {
                server_id: meta.server_id,
                data_dir: config.data_dir,
                db_path,
                bound_addr,
                registration_open: config.registration_open,
                max_notebook_bytes: config.max_notebook_bytes,
            },
            db: Some(db),
            discovery,
            shutdown_tx: Some(shutdown_tx),
            join: Some(join),
        })
    }

    pub fn info(&self) -> &RunningServerInfo {
        &self.info
    }

    pub fn local_discovery_summary(&self) -> LocalDiscoverySummary {
        self.discovery.summary()
    }

    pub fn stats(&self) -> Result<DbStats, String> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| "Server database handle is closed.".to_string())?;
        db.stats().map_err(|e| e.to_string())
    }

    pub fn list_users(&self) -> Result<Vec<UserSummary>, String> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| "Server database handle is closed.".to_string())?;
        db.list_users().map_err(|e| e.to_string())
    }

    pub fn list_trashed_users(&self) -> Result<Vec<TrashedUserSummary>, String> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| "Server database handle is closed.".to_string())?;
        db.list_trashed_users().map_err(|e| e.to_string())
    }

    pub fn reset_user_password_temporary(
        &self,
        user_id: &str,
    ) -> Result<PasswordResetOutcome, String> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| "Server database handle is closed.".to_string())?;
        db.reset_user_password_temporary(user_id)
            .map_err(|e| e.to_string())
    }

    pub fn trash_user(&self, user_id: &str) -> Result<TrashedUserActionSummary, String> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| "Server database handle is closed.".to_string())?;
        db.trash_user(user_id).map_err(|e| e.to_string())
    }

    pub fn restore_trashed_user(&self, user_id: &str) -> Result<RestoredUserSummary, String> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| "Server database handle is closed.".to_string())?;
        db.restore_trashed_user(user_id).map_err(|e| e.to_string())
    }

    pub fn cleanup_expired_trashed_users(&self) -> Result<usize, String> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| "Server database handle is closed.".to_string())?;
        db.cleanup_expired_trashed_users()
            .map_err(|e| e.to_string())
    }

    pub fn delete_user_permanently(&self, user_id: &str) -> Result<DeletedUserSummary, String> {
        let db = self
            .db
            .as_ref()
            .ok_or_else(|| "Server database handle is closed.".to_string())?;
        db.delete_user_permanently(user_id)
            .map_err(|e| e.to_string())
    }

    pub fn db_file_size_bytes(&self) -> u64 {
        std::fs::metadata(&self.info.db_path).map_or(0, |m| m.len())
    }

    /// Requests graceful shutdown, waits for the runtime thread to flush, and drops the database lock.
    pub fn stop(mut self) -> Result<(), String> {
        self.stop_inner()
    }

    fn stop_inner(&mut self) -> Result<(), String> {
        if let Some(tx) = self.shutdown_tx.take() {
            let _ = tx.send(());
        }
        let result = match self.join.take() {
            Some(handle) => match handle.join() {
                Ok(res) => res,
                Err(_) => Err("Server runtime thread terminated unexpectedly.".to_string()),
            },
            None => Ok(()),
        };
        // Drop the main DB handle only after the background runtime exits so redb releases its file lock cleanly.
        self.db.take();
        result
    }
}

impl Drop for ServerRuntime {
    fn drop(&mut self) {
        let _ = self.stop_inner();
    }
}

fn run_runtime_thread(
    db: ServerDb,
    config: ServerConfig,
    discovery: Arc<LocalDiscoveryRegistry>,
    ready_tx: mpsc::SyncSender<Result<SocketAddr, String>>,
    shutdown_rx: tokio::sync::oneshot::Receiver<()>,
    on_stopped: Option<ServerStoppedCallback>,
) -> Result<(), String> {
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(err) => {
            let msg = format!("Failed to start async server runtime: {err}");
            let _ = ready_tx.send(Err(msg.clone()));
            return Err(msg);
        }
    };

    let result = rt.block_on(async move {
        let listener = match tokio::net::TcpListener::bind(config.bind_addr).await {
            Ok(listener) => listener,
            Err(err) => {
                let msg = format_bind_error(config.bind_addr, &err);
                let _ = ready_tx.send(Err(msg.clone()));
                return Err(msg);
            }
        };

        let bound_addr = match listener.local_addr() {
            Ok(addr) => addr,
            Err(err) => {
                let msg = format!("Failed to read bound server socket address: {err}");
                let _ = ready_tx.send(Err(msg.clone()));
                return Err(msg);
            }
        };

        let cleanup_db = db.clone();
        let cleanup_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(TRASH_CLEANUP_INTERVAL);
            interval.tick().await;
            loop {
                interval.tick().await;
                let db_ref = cleanup_db.clone();
                if let Ok(Ok(purged)) =
                    tokio::task::spawn_blocking(move || db_ref.cleanup_expired_trashed_users())
                        .await
                    && purged > 0
                {
                    log::info!(
                        "Automatically purged {purged} expired account(s) from server Trash."
                    );
                }
            }
        });

        let state = AppState {
            db,
            config: Arc::new(config),
            rate_limiter: Arc::new(AuthRateLimiter::new(Duration::from_secs(60), 30)),
            discovery,
        };

        let _ = ready_tx.send(Ok(bound_addr));

        let router = build_router(state);
        let serve_res = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(async move {
            let _ = shutdown_rx.await;
        })
        .await
        .map_err(|e| format!("Server stopped with error: {e}"));

        // Abort and await cleanup task before leaving block_on so all ServerDb clones drop before stop() returns.
        cleanup_task.abort();
        let _ = cleanup_task.await;
        serve_res
    });

    if let Some(cb) = on_stopped {
        cb(result.clone());
    }
    result
}

fn open_existing_db(data_dir: &Path) -> Result<Option<ServerDb>, String> {
    let db_path = crate::config::resolve_db_path(data_dir);
    if !db_path.is_file() {
        return Ok(None);
    }
    let db = ServerDb::open(&db_path).map_err(|e| format_db_open_error(&db_path, &e))?;
    Ok(Some(db))
}

fn require_existing_db(data_dir: &Path) -> Result<ServerDb, String> {
    open_existing_db(data_dir)?.ok_or_else(|| "Server database does not exist yet.".to_string())
}

/// Lists registered accounts in `data_dir` when no [`ServerRuntime`] holds the database open.
pub fn list_users_in_dir(data_dir: &Path) -> Result<Vec<UserSummary>, String> {
    let Some(db) = open_existing_db(data_dir)? else {
        return Ok(Vec::new());
    };
    let users = db.list_users().map_err(|e| e.to_string())?;
    drop(db);
    Ok(users)
}

/// Lists both active and trashed accounts in `data_dir` using a single database open.
pub fn list_users_and_trash_in_dir(
    data_dir: &Path,
) -> Result<(Vec<UserSummary>, Vec<TrashedUserSummary>), String> {
    let Some(db) = open_existing_db(data_dir)? else {
        return Ok((Vec::new(), Vec::new()));
    };
    let users = db.list_users().map_err(|e| e.to_string())?;
    let trashed = db.list_trashed_users().map_err(|e| e.to_string())?;
    drop(db);
    Ok((users, trashed))
}

/// Lists accounts held in the 30-day server Trash when the server is stopped.
pub fn list_trashed_users_in_dir(data_dir: &Path) -> Result<Vec<TrashedUserSummary>, String> {
    let Some(db) = open_existing_db(data_dir)? else {
        return Ok(Vec::new());
    };
    let trashed = db.list_trashed_users().map_err(|e| e.to_string())?;
    drop(db);
    Ok(trashed)
}

/// Generates a random temporary password/recovery code for `user_id` when the server is stopped.
pub fn reset_user_password_temporary_in_dir(
    data_dir: &Path,
    user_id: &str,
) -> Result<PasswordResetOutcome, String> {
    let db = require_existing_db(data_dir)?;
    let outcome = db
        .reset_user_password_temporary(user_id)
        .map_err(|e| e.to_string())?;
    drop(db);
    Ok(outcome)
}

/// Moves `user_id` into the 30-day server Trash when the server is stopped.
pub fn trash_user_in_dir(
    data_dir: &Path,
    user_id: &str,
) -> Result<TrashedUserActionSummary, String> {
    let db = require_existing_db(data_dir)?;
    let summary = db.trash_user(user_id).map_err(|e| e.to_string())?;
    drop(db);
    Ok(summary)
}

/// Restores `user_id` from the 30-day server Trash when the server is stopped.
pub fn restore_trashed_user_in_dir(
    data_dir: &Path,
    user_id: &str,
) -> Result<RestoredUserSummary, String> {
    let db = require_existing_db(data_dir)?;
    let summary = db
        .restore_trashed_user(user_id)
        .map_err(|e| e.to_string())?;
    drop(db);
    Ok(summary)
}

/// Purges any expired trashed accounts in `data_dir` when the server is stopped.
pub fn cleanup_expired_trashed_users_in_dir(data_dir: &Path) -> Result<usize, String> {
    let Some(db) = open_existing_db(data_dir)? else {
        return Ok(0);
    };
    let count = db
        .cleanup_expired_trashed_users()
        .map_err(|e| e.to_string())?;
    drop(db);
    Ok(count)
}

/// Permanently deletes `user_id` and all of its notebooks when the server is stopped.
pub fn delete_user_permanently_in_dir(
    data_dir: &Path,
    user_id: &str,
) -> Result<DeletedUserSummary, String> {
    let db = require_existing_db(data_dir)?;
    let summary = db
        .delete_user_permanently(user_id)
        .map_err(|e| e.to_string())?;
    drop(db);
    Ok(summary)
}

/// Returns ONLY aggregate counts of unsynced local accounts and their notebooks near `data_dir`.
pub fn local_discovery_in_dir(data_dir: &Path, extra_roots: &[PathBuf]) -> LocalDiscoverySummary {
    discover_local_accounts_on_disk(data_dir, extra_roots)
}

pub fn format_bind_error(addr: SocketAddr, err: &std::io::Error) -> String {
    match err.kind() {
        std::io::ErrorKind::AddrInUse => format!(
            "Port {} is already in use on {}. Choose another port or stop the other server using it.",
            addr.port(),
            addr.ip()
        ),
        std::io::ErrorKind::PermissionDenied => format!(
            "Permission denied binding to {addr}. Ports below 1024 may require administrator rights; try a port like 8787."
        ),
        std::io::ErrorKind::AddrNotAvailable => format!(
            "The IP address {} is not available on this computer. Switch to \"This PC only\" or \"Local network / VPN\".",
            addr.ip()
        ),
        _ => format!("Could not start listener on {addr}: {err}"),
    }
}

fn format_db_open_error(db_path: &std::path::Path, err: &crate::db::DbError) -> String {
    let msg = err.to_string();
    let lower = msg.to_ascii_lowercase();
    if lower.contains("already open") || lower.contains("cannot acquire lock") {
        format!(
            "The database `{}` is already in use by another running server instance.",
            db_path.display()
        )
    } else {
        msg
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use papercraft_sync_protocol::ServerInfoResponse;
    use std::sync::atomic::{AtomicBool, Ordering};

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
        let p = std::env::temp_dir().join(format!("papercraft_rt_test_{label}_{nanos}"));
        let _ = std::fs::remove_dir_all(&p);
        TempDirGuard(p)
    }

    #[test]
    fn runtime_starts_serves_health_stops_and_restarts_cleanly_on_same_dir() {
        let dir = temp_dir("restart");
        let stopped_flag = Arc::new(AtomicBool::new(false));
        let stopped_flag_cb = Arc::clone(&stopped_flag);

        let cfg1 = ServerConfig::from_options(
            dir.0.clone(),
            SocketAddr::from(([127, 0, 0, 1], 0)),
            true,
            24,
            30,
            128,
        );
        let rt1 = ServerRuntime::start(
            cfg1,
            Some(Box::new(move |res| {
                assert!(res.is_ok());
                stopped_flag_cb.store(true, Ordering::SeqCst);
            })),
        )
        .unwrap();

        let bound_addr = rt1.info().bound_addr;
        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .http_status_as_error(false)
                .build(),
        );
        let mut resp = agent
            .get(&format!("http://{bound_addr}/api/v1/health"))
            .call()
            .unwrap();
        assert_eq!(resp.status().as_u16(), 200);
        let info: ServerInfoResponse = resp.body_mut().read_json().unwrap();
        assert_eq!(info.service, "papercraft-server");

        let stats = rt1.stats().unwrap();
        assert_eq!(stats.user_count, 0);
        assert!(rt1.list_users().unwrap().is_empty());
        assert!(rt1.db_file_size_bytes() > 0);

        // Register a user via HTTP and confirm live list_users() returns it.
        let mut reg_resp = agent
            .post(&format!("http://{bound_addr}/api/v1/auth/register"))
            .send_json(papercraft_sync_protocol::RegisterRequest {
                email: Some("alex_test@example.com".to_string()),
                username: "alex_test".to_string(),
                password: "strong-password-1".to_string(),
                display_name: Some("Alex".to_string()),
                device_name: Some("Desktop".to_string()),
            })
            .unwrap();
        assert_eq!(reg_resp.status().as_u16(), 201);
        let _ = reg_resp.body_mut().read_to_string();

        let users_live = rt1.list_users().unwrap();
        assert_eq!(users_live.len(), 1);
        assert_eq!(users_live[0].username, "alex_test");
        assert_eq!(users_live[0].display_name, "Alex");

        rt1.stop().unwrap();
        assert!(stopped_flag.load(Ordering::SeqCst));

        // Offline directory query works after stop and returns the registered user cleanly.
        let users_offline = list_users_in_dir(&dir.0).unwrap();
        assert_eq!(users_offline.len(), 1);
        assert_eq!(users_offline[0].username, "alex_test");

        // Immediately restarting on the same directory proves the redb lock was released.
        let cfg2 = ServerConfig::from_options(
            dir.0.clone(),
            SocketAddr::from(([127, 0, 0, 1], 0)),
            false,
            12,
            15,
            64,
        );
        let rt2 = ServerRuntime::start(cfg2, None).unwrap();
        assert!(!rt2.info().registration_open);
        assert_eq!(rt2.info().max_notebook_bytes, 64 * 1024 * 1024);
        rt2.stop().unwrap();
    }

    #[test]
    fn friendly_bind_error_messages_cover_common_kinds() {
        let addr: SocketAddr = "127.0.0.1:8787".parse().unwrap();
        let in_use = format_bind_error(
            addr,
            &std::io::Error::new(std::io::ErrorKind::AddrInUse, "in use"),
        );
        assert!(in_use.contains("Port 8787 is already in use"));

        let perm = format_bind_error(
            addr,
            &std::io::Error::new(std::io::ErrorKind::PermissionDenied, "denied"),
        );
        assert!(perm.contains("Permission denied"));

        let unavail = format_bind_error(
            addr,
            &std::io::Error::new(std::io::ErrorKind::AddrNotAvailable, "nope"),
        );
        assert!(unavail.contains("not available"));
    }
}
