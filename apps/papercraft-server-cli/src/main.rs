#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

use papercraft_server_cli::{app, config, db};

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Args, Parser, Subcommand};

use app::{
    AppState, AuthRateLimiter, LocalDiscoveryRegistry, build_router,
    discover_local_accounts_on_disk,
};
use config::{
    DEFAULT_ACCESS_TTL_HOURS, DEFAULT_BIND_ADDR, DEFAULT_DATA_DIR, DEFAULT_MAX_NOTEBOOK_MB,
    DEFAULT_REFRESH_TTL_DAYS, ServerConfig, ensure_data_dir_and_meta,
};
use db::ServerDb;

#[derive(Parser)]
#[command(
    name = "papercraft-server-cli",
    version,
    about = "Self-hosted PaperCraft account and offline-first sync server"
)]
struct Cli {
    #[command(flatten)]
    serve: ServeArgs,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Args, Clone, Debug)]
struct ServeArgs {
    /// Directory where the server database and metadata are stored (e.g. C:\\papercraft-server).
    #[arg(long, env = "PAPERCRAFT_DATA_DIR", default_value = DEFAULT_DATA_DIR)]
    data_dir: PathBuf,
    /// Socket address to listen on (use 0.0.0.0:8787 for mesh VPN or LAN access).
    #[arg(long, env = "PAPERCRAFT_BIND", default_value = DEFAULT_BIND_ADDR)]
    bind: SocketAddr,
    /// Disable public account registration (existing accounts can still sign in).
    #[arg(long, env = "PAPERCRAFT_DISABLE_REGISTRATION", default_value_t = false)]
    disable_registration: bool,
    /// Access token lifetime in hours.
    #[arg(long, env = "PAPERCRAFT_ACCESS_TTL_HOURS", default_value_t = DEFAULT_ACCESS_TTL_HOURS)]
    access_ttl_hours: u64,
    /// Refresh token lifetime in days.
    #[arg(long, env = "PAPERCRAFT_REFRESH_TTL_DAYS", default_value_t = DEFAULT_REFRESH_TTL_DAYS)]
    refresh_ttl_days: u64,
    /// Maximum single notebook payload size in MiB.
    #[arg(long, env = "PAPERCRAFT_MAX_NOTEBOOK_MB", default_value_t = DEFAULT_MAX_NOTEBOOK_MB)]
    max_notebook_mb: usize,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the HTTP sync and authentication server (default if omitted).
    Serve(ServeArgs),
    /// Print database statistics for the configured data directory.
    Status {
        /// Server data directory path.
        #[arg(long, env = "PAPERCRAFT_DATA_DIR", default_value = DEFAULT_DATA_DIR)]
        data_dir: PathBuf,
    },
    /// List registered user accounts stored in the database.
    Users {
        /// Server data directory path.
        #[arg(long, env = "PAPERCRAFT_DATA_DIR", default_value = DEFAULT_DATA_DIR)]
        data_dir: PathBuf,
        /// Show accounts currently held in the 30-day server Trash instead of active accounts.
        #[arg(long, default_value_t = false)]
        trash: bool,
    },
    /// Reset a user's password (random one-time recovery code if --password is omitted).
    ResetPassword {
        /// Server data directory path.
        #[arg(long, env = "PAPERCRAFT_DATA_DIR", default_value = DEFAULT_DATA_DIR)]
        data_dir: PathBuf,
        /// Target account username, email, or user ID.
        #[arg(long)]
        username: String,
        /// Explicit new password (at least 8 characters). Omit to generate a random one-time recovery code.
        #[arg(long)]
        password: Option<String>,
        /// Require the user to choose a new password on their next sign-in when passing --password.
        #[arg(long, default_value_t = false)]
        temporary: bool,
    },
    /// Move a user account to the 30-day server Trash (or permanently delete with --permanent).
    DeleteUser {
        /// Server data directory path.
        #[arg(long, env = "PAPERCRAFT_DATA_DIR", default_value = DEFAULT_DATA_DIR)]
        data_dir: PathBuf,
        /// Target account username, email, or user ID.
        #[arg(long)]
        user: String,
        /// Permanently delete immediately instead of keeping in 30-day Trash.
        #[arg(long, default_value_t = false)]
        permanent: bool,
    },
    /// Restore a user account from the 30-day server Trash back to active status.
    RestoreUser {
        /// Server data directory path.
        #[arg(long, env = "PAPERCRAFT_DATA_DIR", default_value = DEFAULT_DATA_DIR)]
        data_dir: PathBuf,
        /// Target account username, email, or user ID.
        #[arg(long)]
        user: String,
    },
    /// Permanently remove expired accounts from the 30-day server Trash.
    CleanupTrash {
        /// Server data directory path.
        #[arg(long, env = "PAPERCRAFT_DATA_DIR", default_value = DEFAULT_DATA_DIR)]
        data_dir: PathBuf,
    },
}

fn exe_sibling_library_roots() -> Vec<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|p| p.join("library")))
        .into_iter()
        .collect()
}

fn build_server_config(args: ServeArgs) -> ServerConfig {
    let mut cfg = ServerConfig::from_options(
        args.data_dir,
        args.bind,
        !args.disable_registration,
        args.access_ttl_hours,
        args.refresh_ttl_days,
        args.max_notebook_mb,
    );
    cfg.extra_library_roots = exe_sibling_library_roots();
    cfg
}

fn run_serve(args: ServeArgs) -> Result<(), String> {
    let config = build_server_config(args);
    let meta = ensure_data_dir_and_meta(&config.data_dir)?;
    let db_path = config.db_path();
    let db = ServerDb::open(&db_path).map_err(|e| e.to_string())?;
    let stats = db.stats().map_err(|e| e.to_string())?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to start async runtime: {e}"))?;

    rt.block_on(async move {
        let discovery = Arc::new(LocalDiscoveryRegistry::for_config(&config));
        let local_counts = discovery.summary();
        let cleanup_db = db.clone();
        let cleanup_task = tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(3600));
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
            config: Arc::new(config.clone()),
            rate_limiter: Arc::new(AuthRateLimiter::new(Duration::from_secs(60), 30)),
            discovery,
        };

        let listener = tokio::net::TcpListener::bind(config.bind_addr)
            .await
            .map_err(|e| format!("failed to bind {}: {e}", config.bind_addr))?;

        println!("PaperCraft Server v{}", env!("CARGO_PKG_VERSION"));
        println!("  Server ID    : {}", meta.server_id);
        println!("  Data Dir     : {}", config.data_dir.display());
        println!("  Database     : {}", db_path.display());
        println!("  Listening    : http://{}", config.bind_addr);
        println!(
            "  Registration : {}",
            if config.registration_open {
                "open"
            } else {
                "closed"
            }
        );
        println!(
            "  Stored Data  : {} users ({} in trash), {} active notebooks",
            stats.user_count, stats.trashed_user_count, stats.active_notebook_count
        );
        println!(
            "  Local Discovery (counts only): {} unsynced local accounts, {} notebooks in local accounts",
            local_counts.unsynced_local_accounts, local_counts.total_local_notebooks
        );
        println!("Press Ctrl+C to stop gracefully.");

        let router = build_router(state);
        let serve_res = axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| format!("server stopped with error: {e}"));

        cleanup_task.abort();
        let _ = cleanup_task.await;
        serve_res
    })
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
    println!("\nShutdown signal received; flushing database and closing server...");
}

fn run_status(data_dir: PathBuf) -> Result<(), String> {
    let meta = ensure_data_dir_and_meta(&data_dir)?;
    let db_path = crate::config::resolve_db_path(&data_dir);
    let db = ServerDb::open(&db_path).map_err(|e| e.to_string())?;
    let stats = db.stats().map_err(|e| e.to_string())?;
    let db_size = std::fs::metadata(&db_path).map_or(0, |m| m.len());
    let local_counts = discover_local_accounts_on_disk(&data_dir, &exe_sibling_library_roots());

    println!("PaperCraft Server Status");
    println!("  Data Dir          : {}", data_dir.display());
    println!(
        "  Database File     : {} ({db_size} bytes)",
        db_path.display()
    );
    println!("  Server ID         : {}", meta.server_id);
    println!("  Users             : {}", stats.user_count);
    println!("  Trashed Accounts  : {}", stats.trashed_user_count);
    println!("  Active Notebooks  : {}", stats.active_notebook_count);
    println!("  Deleted Tombstones: {}", stats.deleted_tombstone_count);
    println!("  Active Sessions   : {}", stats.active_session_count);
    println!(
        "  Unsynced Local Accounts     : {}",
        local_counts.unsynced_local_accounts
    );
    println!(
        "  Notebooks in Local Accounts : {}",
        local_counts.total_local_notebooks
    );
    Ok(())
}

fn run_users(data_dir: PathBuf, trash: bool) -> Result<(), String> {
    let _ = ensure_data_dir_and_meta(&data_dir)?;
    let db_path = crate::config::resolve_db_path(&data_dir);
    let db = ServerDb::open(&db_path).map_err(|e| e.to_string())?;
    if trash {
        let trashed = db.list_trashed_users().map_err(|e| e.to_string())?;
        if trashed.is_empty() {
            println!("No trashed accounts in `{}`.", data_dir.display());
            return Ok(());
        }
        let now_ms = papercraft_sync_protocol::now_unix_ms();
        let day_ms = 24 * 60 * 60 * 1000_i64;
        println!(
            "{:<38} {:<28} {:<20} {:<10} DAYS LEFT",
            "USER ID", "ACCOUNT", "DISPLAY NAME", "NOTEBOOKS"
        );
        for u in trashed {
            let days_left = (u.expires_at_ms.saturating_sub(now_ms).max(0) + day_ms - 1) / day_ms;
            println!(
                "{:<38} {:<28} {:<20} {:<10} {}",
                u.user_id,
                u.account_label(),
                u.display_name,
                u.notebook_count,
                days_left
            );
        }
        return Ok(());
    }

    let users = db.list_users().map_err(|e| e.to_string())?;
    if users.is_empty() {
        println!("No registered accounts in `{}`.", data_dir.display());
        return Ok(());
    }
    println!(
        "{:<38} {:<28} {:<22} NOTEBOOKS",
        "USER ID", "ACCOUNT", "DISPLAY NAME"
    );
    for u in users {
        let account = u.email.as_deref().unwrap_or(&u.username);
        println!(
            "{:<38} {:<28} {:<22} {}",
            u.user_id, account, u.display_name, u.notebook_count
        );
    }
    Ok(())
}

fn run_reset_password(
    data_dir: PathBuf,
    username: String,
    password: Option<String>,
    temporary: bool,
) -> Result<(), String> {
    let _ = ensure_data_dir_and_meta(&data_dir)?;
    let db_path = crate::config::resolve_db_path(&data_dir);
    let db = ServerDb::open(&db_path).map_err(|e| e.to_string())?;
    match password {
        Some(explicit) => {
            db.set_user_password_with_flag(&username, &explicit, temporary)
                .map_err(|e| e.to_string())?;
            if temporary {
                println!(
                    "Temporary password updated for `{username}` (user must change password on next login)."
                );
            } else {
                println!("Password updated for `{username}`.");
            }
        }
        None => {
            let outcome = db
                .reset_user_password_temporary(&username)
                .map_err(|e| e.to_string())?;
            println!(
                "Temporary recovery password generated for `{}`:",
                outcome.account_label()
            );
            println!("  Recovery Code: {}", outcome.temporary_password);
            println!(
                "Share this code with the user; they will be prompted to set a new password on next sign-in."
            );
        }
    }
    Ok(())
}

fn run_delete_user(data_dir: PathBuf, user: String, permanent: bool) -> Result<(), String> {
    let _ = ensure_data_dir_and_meta(&data_dir)?;
    let db_path = crate::config::resolve_db_path(&data_dir);
    let db = ServerDb::open(&db_path).map_err(|e| e.to_string())?;
    if permanent {
        let deleted = db
            .delete_user_permanently(&user)
            .map_err(|e| e.to_string())?;
        println!(
            "Permanently deleted user `{}` (id={}, {} synced notebook(s) removed).",
            deleted.account_label(),
            deleted.user_id,
            deleted.deleted_notebooks
        );
    } else {
        let trashed = db.trash_user(&user).map_err(|e| e.to_string())?;
        println!(
            "Moved user `{}` (id={}, {} synced notebook(s)) to server Trash for {} days.",
            trashed.account_label(),
            trashed.user_id,
            trashed.notebook_count,
            db::ACCOUNT_TRASH_RETENTION_DAYS
        );
    }
    Ok(())
}

fn run_restore_user(data_dir: PathBuf, user: String) -> Result<(), String> {
    let _ = ensure_data_dir_and_meta(&data_dir)?;
    let db_path = crate::config::resolve_db_path(&data_dir);
    let db = ServerDb::open(&db_path).map_err(|e| e.to_string())?;
    let restored = db.restore_trashed_user(&user).map_err(|e| e.to_string())?;
    println!(
        "Restored user `{}` (id={}, {} synced notebook(s) kept).",
        restored.account_label(),
        restored.user_id,
        restored.restored_notebooks
    );
    Ok(())
}

fn run_cleanup_trash(data_dir: PathBuf) -> Result<(), String> {
    let _ = ensure_data_dir_and_meta(&data_dir)?;
    let db_path = crate::config::resolve_db_path(&data_dir);
    let db = ServerDb::open(&db_path).map_err(|e| e.to_string())?;
    let purged = db
        .cleanup_expired_trashed_users()
        .map_err(|e| e.to_string())?;
    println!("Permanently removed {purged} expired account(s) from server Trash.");
    Ok(())
}

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let cli = Cli::parse();
    let result = match cli.command {
        None => run_serve(cli.serve),
        Some(Commands::Serve(args)) => run_serve(args),
        Some(Commands::Status { data_dir }) => run_status(data_dir),
        Some(Commands::Users { data_dir, trash }) => run_users(data_dir, trash),
        Some(Commands::ResetPassword {
            data_dir,
            username,
            password,
            temporary,
        }) => run_reset_password(data_dir, username, password, temporary),
        Some(Commands::DeleteUser {
            data_dir,
            user,
            permanent,
        }) => run_delete_user(data_dir, user, permanent),
        Some(Commands::RestoreUser { data_dir, user }) => run_restore_user(data_dir, user),
        Some(Commands::CleanupTrash { data_dir }) => run_cleanup_trash(data_dir),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::db::{DbError, TokenTtls};
    use papercraft_doc::{Document, NotebookCover, PenSettings, StrokeSample};
    use papercraft_sync_protocol::{
        LocalNotebookChange, SyncPushRequest, canonical_document_body_hash,
        canonical_document_hash, canonical_document_meta_hash, now_unix_ms,
    };
    use std::path::Path;

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
        let p = std::env::temp_dir().join(format!("papercraft_srv_test_{label}_{nanos}"));
        let _ = std::fs::remove_dir_all(&p);
        TempDirGuard(p)
    }

    #[test]
    fn db_email_uniqueness_and_change_email_preserve_user_id() {
        let dir = temp_dir("db_email");
        let _ = ensure_data_dir_and_meta(&dir.0).unwrap();
        let db = ServerDb::open(&dir.0.join("notecraft.redb")).unwrap();
        let ttls = TokenTtls {
            access_ttl_ms: 3_600_000,
            refresh_ttl_ms: 86_400_000,
        };

        let reg = db
            .register_user_with_email(
                Some("Alex@Example.com"),
                "",
                "strong-password-1",
                Some("Alex"),
                Some("Workstation"),
                true,
                ttls,
            )
            .unwrap();
        assert_eq!(reg.user.email.as_deref(), Some("Alex@Example.com"));
        assert!(db.authenticate_access_token(&reg.access_token).is_ok());

        // Case-insensitive duplicate email rejected, check_email sees it, change_email preserves user_id.
        assert!(matches!(
            db.register_user_with_email(
                Some("alex@example.com"),
                "",
                "strong-password-1",
                None,
                None,
                true,
                ttls
            ),
            Err(DbError::EmailTaken(_))
        ));
        assert!(
            db.check_email_exists("alex@example.com", true)
                .unwrap()
                .exists
        );
        assert!(
            !db.check_email_exists("new@example.com", true)
                .unwrap()
                .exists
        );
        let updated_profile = db
            .change_email(
                &reg.user.user_id,
                "strong-password-1",
                "alex.new@example.com",
            )
            .unwrap();
        assert_eq!(updated_profile.user_id, reg.user.user_id);
        assert_eq!(
            updated_profile.email.as_deref(),
            Some("alex.new@example.com")
        );

        let login = db
            .login_user(
                "ALEX.NEW@EXAMPLE.COM",
                "strong-password-1",
                Some("Laptop"),
                ttls,
            )
            .unwrap();
        assert_eq!(login.user.user_id, reg.user.user_id);
        let refreshed = db.refresh_session(&login.refresh_token, ttls).unwrap();
        assert_ne!(refreshed.refresh_token, login.refresh_token);
        assert!(db.refresh_session(&login.refresh_token, ttls).is_err());
    }

    #[test]
    fn db_auth_sync_conflict_and_tombstone_lifecycle() {
        let dir = temp_dir("db_lifecycle");
        let _ = ensure_data_dir_and_meta(&dir.0).unwrap();
        let db = ServerDb::open(&dir.0.join("notecraft.redb")).unwrap();
        let ttls = TokenTtls {
            access_ttl_ms: 3_600_000,
            refresh_ttl_ms: 86_400_000,
        };

        let reg = db
            .register_user_with_email(
                Some("sync@example.com"),
                "",
                "strong-password-1",
                Some("Sync"),
                Some("Workstation"),
                true,
                ttls,
            )
            .unwrap();

        // Device 1 pushes a notebook.
        let mut doc = Document {
            id: "nb-1".to_string(),
            title: "Physics Notes".to_string(),
            cover: NotebookCover::SageGreen,
            ..Default::default()
        };
        let id = doc.alloc_stroke_id();
        let pts = vec![
            StrokeSample::from_xy_pressure(10.0, 10.0, 0.5),
            StrokeSample::from_xy_pressure(60.0, 40.0, 0.8),
        ];
        doc.strokes.push(Arc::new(papercraft_doc::InkStroke::new(
            id,
            PenSettings::fountain_pen(),
            pts.clone(),
            pts.clone(),
        )));
        let hash1 = canonical_document_hash(&doc);

        let push1 = db
            .push_changes(
                &reg.user.user_id,
                SyncPushRequest {
                    client_cursor: 0,
                    device_name: "Workstation".to_string(),
                    changes: vec![LocalNotebookChange {
                        notebook_id: "nb-1".to_string(),
                        base_version: 0,
                        deleted: false,
                        updated_at_ms: 1000,
                        deleted_at_ms: None,
                        content_hash: hash1.clone(),
                        body_hash: canonical_document_body_hash(&doc),
                        meta_hash: canonical_document_meta_hash(&doc),
                        meta_updated_at_ms: 1000,
                        document: Some(doc.clone()),
                    }],
                    folders: Vec::new(),
                    studio_prefs: None,
                    force_overwrite: false,
                },
                32 * 1024 * 1024,
            )
            .unwrap();
        assert_eq!(push1.accepted.len(), 1);
        assert_eq!(push1.accepted[0].new_version, 1);
        assert!(push1.conflicts.is_empty());

        // Device 2 pulls from cursor 0.
        let pulled = db.pull_changes(&reg.user.user_id, 0).unwrap();
        assert_eq!(pulled.notebooks.len(), 1);
        assert_eq!(pulled.notebooks[0].version, 1);
        assert_eq!(pulled.notebooks[0].title, "Physics Notes");

        // Device 1 edits strokes (v2); Device 2 edits only metadata with newer timestamp on same body?
        // First: metadata-only concurrent edit when body didn't diverge resolves LWW cleanly!
        let mut meta_v2 = doc.clone();
        meta_v2.title = "Physics Metadata v2".to_string();
        meta_v2.meta_updated_at_ms = 2000;
        let push_meta = db
            .push_changes(
                &reg.user.user_id,
                SyncPushRequest {
                    client_cursor: push1.server_cursor,
                    device_name: "Workstation".to_string(),
                    changes: vec![LocalNotebookChange {
                        notebook_id: "nb-1".to_string(),
                        base_version: 1,
                        deleted: false,
                        updated_at_ms: 2000,
                        deleted_at_ms: None,
                        content_hash: canonical_document_hash(&meta_v2),
                        body_hash: canonical_document_body_hash(&meta_v2),
                        meta_hash: canonical_document_meta_hash(&meta_v2),
                        meta_updated_at_ms: 2000,
                        document: Some(meta_v2),
                    }],
                    folders: Vec::new(),
                    studio_prefs: None,
                    force_overwrite: false,
                },
                32 * 1024 * 1024,
            )
            .unwrap();
        assert_eq!(push_meta.accepted[0].new_version, 2);

        let mut meta_newer = doc.clone();
        meta_newer.favorite = true;
        meta_newer.meta_updated_at_ms = 3000;
        let lww_push = db
            .push_changes(
                &reg.user.user_id,
                SyncPushRequest {
                    client_cursor: 1,
                    device_name: "Laptop".to_string(),
                    changes: vec![LocalNotebookChange {
                        notebook_id: "nb-1".to_string(),
                        base_version: 1,
                        deleted: false,
                        updated_at_ms: 3000,
                        deleted_at_ms: None,
                        content_hash: canonical_document_hash(&meta_newer),
                        body_hash: canonical_document_body_hash(&meta_newer),
                        meta_hash: canonical_document_meta_hash(&meta_newer),
                        meta_updated_at_ms: 3000,
                        document: Some(meta_newer),
                    }],
                    folders: Vec::new(),
                    studio_prefs: None,
                    force_overwrite: false,
                },
                32 * 1024 * 1024,
            )
            .unwrap();
        assert_eq!(lww_push.accepted.len(), 1);
        assert_eq!(lww_push.accepted[0].new_version, 3);
        assert!(lww_push.conflicts.is_empty());

        // Actual content divergence (strokes changed on stale base_version 1) -> conflict!
        let mut stale_doc = doc;
        let s2 = stale_doc.alloc_stroke_id();
        stale_doc
            .strokes
            .push(Arc::new(papercraft_doc::InkStroke::new(
                s2,
                PenSettings::ball_pen(),
                pts.clone(),
                pts,
            )));
        let conflict_push = db
            .push_changes(
                &reg.user.user_id,
                SyncPushRequest {
                    client_cursor: 1,
                    device_name: "Laptop".to_string(),
                    changes: vec![LocalNotebookChange {
                        notebook_id: "nb-1".to_string(),
                        base_version: 1,
                        deleted: false,
                        updated_at_ms: 4000,
                        deleted_at_ms: None,
                        content_hash: canonical_document_hash(&stale_doc),
                        body_hash: canonical_document_body_hash(&stale_doc),
                        meta_hash: canonical_document_meta_hash(&stale_doc),
                        meta_updated_at_ms: 4000,
                        document: Some(stale_doc),
                    }],
                    folders: Vec::new(),
                    studio_prefs: None,
                    force_overwrite: false,
                },
                32 * 1024 * 1024,
            )
            .unwrap();
        assert!(conflict_push.accepted.is_empty());
        assert_eq!(conflict_push.conflicts.len(), 1);
        assert_eq!(conflict_push.conflicts[0].server_record.version, 3);

        // Delete nb-1 at v3 -> tombstone appears on incremental pull since cursor 1.
        let del_push = db
            .push_changes(
                &reg.user.user_id,
                SyncPushRequest {
                    client_cursor: 2,
                    device_name: "Workstation".to_string(),
                    changes: vec![LocalNotebookChange {
                        notebook_id: "nb-1".to_string(),
                        base_version: 3,
                        deleted: true,
                        updated_at_ms: now_unix_ms(),
                        deleted_at_ms: Some(now_unix_ms()),
                        content_hash: "deleted".to_string(),
                        body_hash: String::new(),
                        meta_hash: String::new(),
                        meta_updated_at_ms: 0,
                        document: None,
                    }],
                    folders: Vec::new(),
                    studio_prefs: None,
                    force_overwrite: false,
                },
                32 * 1024 * 1024,
            )
            .unwrap();
        assert!(del_push.accepted[0].deleted);
        let inc_pull = db.pull_changes(&reg.user.user_id, 1).unwrap();
        assert_eq!(inc_pull.notebooks.len(), 1);
        assert!(inc_pull.notebooks[0].deleted);

        // Full pull from cursor 0 hides ancient tombstones.
        let fresh_pull = db.pull_changes(&reg.user.user_id, 0).unwrap();
        assert!(fresh_pull.notebooks.is_empty());
    }

    struct SpawnedTestServer {
        base_url: String,
        shutdown_tx: Option<tokio::sync::oneshot::Sender<()>>,
        join: Option<std::thread::JoinHandle<()>>,
    }

    impl Drop for SpawnedTestServer {
        fn drop(&mut self) {
            if let Some(tx) = self.shutdown_tx.take() {
                let _ = tx.send(());
            }
            if let Some(handle) = self.join.take() {
                let _ = handle.join();
            }
        }
    }

    fn spawn_server_on(data_dir: &Path) -> SpawnedTestServer {
        let _ = ensure_data_dir_and_meta(data_dir).unwrap();
        let db = ServerDb::open(&data_dir.join("notecraft.redb")).unwrap();
        let cfg = ServerConfig {
            data_dir: data_dir.to_path_buf(),
            bind_addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            registration_open: true,
            access_token_ttl_ms: 3_600_000,
            refresh_token_ttl_ms: 86_400_000,
            max_notebook_bytes: 128 * 1024 * 1024,
            max_request_bytes: 256 * 1024 * 1024,
            extra_library_roots: Vec::new(),
        };
        let discovery = Arc::new(LocalDiscoveryRegistry::for_config(&cfg));
        let state = AppState {
            db,
            config: Arc::new(cfg),
            rate_limiter: Arc::new(AuthRateLimiter::new(Duration::from_secs(60), 50)),
            discovery,
        };

        let (addr_tx, addr_rx) = std::sync::mpsc::channel::<SocketAddr>();
        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

        let join = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                let local_addr = listener.local_addr().unwrap();
                let _ = addr_tx.send(local_addr);
                let router = build_router(state);
                axum::serve(
                    listener,
                    router.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .with_graceful_shutdown(async move {
                    let _ = shutdown_rx.await;
                })
                .await
                .unwrap();
            });
        });

        let addr = addr_rx.recv().unwrap();
        SpawnedTestServer {
            base_url: format!("http://{addr}"),
            shutdown_tx: Some(shutdown_tx),
            join: Some(join),
        }
    }

    #[test]
    fn http_end_to_end_auth_sync_refresh_logout_and_restart_persistence() {
        use papercraft_sync_protocol::{
            AuthResponse, LoginRequest, LogoutRequest, RefreshRequest, RegisterRequest,
            ServerInfoResponse, SyncPullResponse, SyncPushResponse, UserProfile,
        };

        let dir = temp_dir("http_e2e");
        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .http_status_as_error(false)
                .build(),
        );

        let doc = Document {
            title: "Chemistry Lab".to_string(),
            cover: NotebookCover::Terracotta,
            ..Default::default()
        };
        let doc_hash = canonical_document_hash(&doc);

        {
            let srv = spawn_server_on(&dir.0);

            // 1. Health
            let mut resp = agent
                .get(&format!("{}/api/v1/health", srv.base_url))
                .call()
                .unwrap();
            assert_eq!(resp.status().as_u16(), 200);
            let info: ServerInfoResponse = resp.body_mut().read_json().unwrap();
            assert!(info.registration_open);

            // 2. Register
            let mut reg_resp = agent
                .post(&format!("{}/api/v1/auth/register", srv.base_url))
                .send_json(RegisterRequest {
                    email: Some("maya@example.com".to_string()),
                    username: "maya".to_string(),
                    password: "vault-password-123".to_string(),
                    display_name: Some("Maya".to_string()),
                    device_name: Some("WinPC".to_string()),
                })
                .unwrap();
            assert_eq!(reg_resp.status().as_u16(), 201);
            let reg: AuthResponse = reg_resp.body_mut().read_json().unwrap();
            assert_eq!(reg.user.username, "maya");

            // 3. /auth/me
            let mut me_resp = agent
                .get(&format!("{}/api/v1/auth/me", srv.base_url))
                .header("Authorization", &format!("Bearer {}", reg.access_token))
                .call()
                .unwrap();
            assert_eq!(me_resp.status().as_u16(), 200);
            let me: UserProfile = me_resp.body_mut().read_json().unwrap();
            assert_eq!(me.user_id, reg.user.user_id);

            // 4. Push notebook
            let mut push_resp = agent
                .post(&format!("{}/api/v1/sync/push", srv.base_url))
                .header("Authorization", &format!("Bearer {}", reg.access_token))
                .send_json(SyncPushRequest {
                    client_cursor: 0,
                    device_name: "WinPC".to_string(),
                    changes: vec![LocalNotebookChange {
                        notebook_id: "chem-1".to_string(),
                        base_version: 0,
                        deleted: false,
                        updated_at_ms: now_unix_ms(),
                        deleted_at_ms: None,
                        content_hash: doc_hash.clone(),
                        body_hash: canonical_document_body_hash(&doc),
                        meta_hash: canonical_document_meta_hash(&doc),
                        meta_updated_at_ms: now_unix_ms(),
                        document: Some(doc.clone()),
                    }],
                    folders: Vec::new(),
                    studio_prefs: None,
                    force_overwrite: false,
                })
                .unwrap();
            assert_eq!(push_resp.status().as_u16(), 200);
            let pushed: SyncPushResponse = push_resp.body_mut().read_json().unwrap();
            assert_eq!(pushed.accepted.len(), 1);
            assert_eq!(pushed.accepted[0].new_version, 1);

            // 5. Refresh tokens
            let mut ref_resp = agent
                .post(&format!("{}/api/v1/auth/refresh", srv.base_url))
                .send_json(RefreshRequest {
                    refresh_token: reg.refresh_token.clone(),
                })
                .unwrap();
            assert_eq!(ref_resp.status().as_u16(), 200);
            let refreshed: AuthResponse = ref_resp.body_mut().read_json().unwrap();
            assert_ne!(refreshed.refresh_token, reg.refresh_token);

            // 6. Logout revokes new access/refresh tokens
            let logout_resp = agent
                .post(&format!("{}/api/v1/auth/logout", srv.base_url))
                .header(
                    "Authorization",
                    &format!("Bearer {}", refreshed.access_token),
                )
                .send_json(LogoutRequest {
                    refresh_token: Some(refreshed.refresh_token.clone()),
                })
                .unwrap();
            assert_eq!(logout_resp.status().as_u16(), 204);

            let revoked_me = agent
                .get(&format!("{}/api/v1/auth/me", srv.base_url))
                .header(
                    "Authorization",
                    &format!("Bearer {}", refreshed.access_token),
                )
                .call()
                .unwrap();
            assert_eq!(revoked_me.status().as_u16(), 401);
        }

        // 7. Restart server on same data_dir and verify persistence via login + pull
        {
            let srv2 = spawn_server_on(&dir.0);
            let mut login_resp = agent
                .post(&format!("{}/api/v1/auth/login", srv2.base_url))
                .send_json(LoginRequest {
                    email: Some("MAYA@EXAMPLE.COM".to_string()),
                    username: String::new(),
                    password: "vault-password-123".to_string(),
                    device_name: Some("Tablet".to_string()),
                })
                .unwrap();
            assert_eq!(login_resp.status().as_u16(), 200);
            let login: AuthResponse = login_resp.body_mut().read_json().unwrap();

            let mut pull_resp = agent
                .get(&format!("{}/api/v1/sync/pull?since=0", srv2.base_url))
                .header("Authorization", &format!("Bearer {}", login.access_token))
                .call()
                .unwrap();
            assert_eq!(pull_resp.status().as_u16(), 200);
            let pulled: SyncPullResponse = pull_resp.body_mut().read_json().unwrap();
            assert_eq!(pulled.notebooks.len(), 1);
            assert_eq!(pulled.notebooks[0].notebook_id, "chem-1");
            assert_eq!(pulled.notebooks[0].title, "Chemistry Lab");
            assert_eq!(pulled.notebooks[0].content_hash, doc_hash);
        }
    }

    #[test]
    fn two_desktop_sessions_offline_edits_conflict_recovery_and_restart_converge() {
        use papercraft_doc::{PenSettings, StrokeSample, UiTheme};
        use papercraft_engine::Session;
        use papercraft_sync::{SyncHttpClient, run_sync_cycle};
        use papercraft_sync_protocol::{LoginRequest, RegisterRequest};
        use serde_json::json;

        let srv_dir = temp_dir("two_pc_srv");
        let pc1_dir = temp_dir("two_pc_1");
        let pc2_dir = temp_dir("two_pc_2");
        let client = SyncHttpClient::new();

        // Start server phase 1 and create account on PC1 with custom offline notebook.
        {
            let srv = spawn_server_on(&srv_dir.0);
            let mut pc1 = Session::open_desktop_library(pc1_dir.0.clone());
            pc1.execute("notebook.create", &json!({ "title": "Linear Algebra" }))
                .unwrap();

            let auth1 = client
                .register(
                    &srv.base_url,
                    &RegisterRequest {
                        email: Some("carlos@example.com".to_string()),
                        username: "carlos".to_string(),
                        password: "tunnel-secret-pass".to_string(),
                        display_name: Some("Carlos".to_string()),
                        device_name: Some("PC1".to_string()),
                    },
                    "PC1",
                )
                .unwrap();
            let prof1 = pc1.active_local_profile().unwrap().local_profile_id.clone();
            pc1.switch_to_authenticated_library(
                auth1,
                papercraft_engine::AuthWorkspaceMode::PromoteLocalProfile {
                    source_profile_id: prof1,
                },
            );
            let mut acct1 = pc1.active_account_session().unwrap().clone();
            let batch1 = pc1.prepare_sync_batch().unwrap();
            let outcome1 = run_sync_cycle(&client, &mut acct1, &batch1).unwrap();
            pc1.apply_sync_outcome(outcome1).unwrap();
            assert_eq!(pc1.sync_status_summary().pending_changes, 0);

            // PC2 logs in fresh and pulls cleanly without duplicating notebooks.
            let mut pc2 = Session::open_desktop_library(pc2_dir.0.clone());
            let auth2 = client
                .login(
                    &srv.base_url,
                    &LoginRequest {
                        email: Some("carlos@example.com".to_string()),
                        username: String::new(),
                        password: "tunnel-secret-pass".to_string(),
                        device_name: Some("PC2".to_string()),
                    },
                    "PC2",
                )
                .unwrap();
            pc2.switch_to_authenticated_library(
                auth2,
                papercraft_engine::AuthWorkspaceMode::SwitchOnly,
            );
            let mut acct2 = pc2.active_account_session().unwrap().clone();
            let batch2 = pc2.prepare_sync_batch().unwrap();
            let outcome2 = run_sync_cycle(&client, &mut acct2, &batch2).unwrap();
            pc2.apply_sync_outcome(outcome2).unwrap();

            assert_eq!(pc2.notebooks.len(), pc1.notebooks.len());
            assert!(pc2.notebooks.iter().any(|n| n.title == "Linear Algebra"));
            assert_eq!(pc2.sync_status_summary().pending_changes, 0);
        }

        // Server is OFFLINE now. Both PCs work completely offline and restart!
        {
            let mut pc1_reopened = Session::open_desktop_library(pc1_dir.0.clone());
            let idx1 = pc1_reopened
                .notebooks
                .iter()
                .position(|n| n.title == "Linear Algebra")
                .unwrap();
            assert!(pc1_reopened.switch_notebook(idx1));
            pc1_reopened.commit_raw_stroke(
                PenSettings::fountain_pen(),
                vec![
                    StrokeSample::from_xy_pressure(12.0, 12.0, 0.5),
                    StrokeSample::from_xy_pressure(80.0, 60.0, 0.7),
                ],
            );
            pc1_reopened
                .execute("theme.set", &json!({ "theme": UiTheme::Dark }))
                .unwrap();
            let _ = pc1_reopened.save_library();

            let mut pc2_reopened = Session::open_desktop_library(pc2_dir.0.clone());
            let idx2 = pc2_reopened
                .notebooks
                .iter()
                .position(|n| n.title == "Linear Algebra")
                .unwrap();
            assert!(pc2_reopened.switch_notebook(idx2));
            pc2_reopened.commit_raw_stroke(
                PenSettings::ball_pen(),
                vec![
                    StrokeSample::from_xy_pressure(100.0, 100.0, 0.6),
                    StrokeSample::from_xy_pressure(160.0, 140.0, 0.8),
                ],
            );
            let _ = pc2_reopened.save_library();
        }

        // Server restarts at a NEW port (like reconnecting tunnel / restart).
        let srv2 = spawn_server_on(&srv_dir.0);
        let mut pc1 = Session::open_desktop_library(pc1_dir.0.clone());
        let mut pc2 = Session::open_desktop_library(pc2_dir.0.clone());
        pc1.set_server_url_preference(&srv2.base_url).unwrap();
        pc2.set_server_url_preference(&srv2.base_url).unwrap();

        // PC1 syncs first -> pushes stroke + Dark theme cleanly.
        let mut acct1 = pc1.active_account_session().unwrap().clone();
        let batch1 = pc1.prepare_sync_batch().unwrap();
        let out1 = run_sync_cycle(&client, &mut acct1, &batch1).unwrap();
        pc1.apply_sync_outcome(out1).unwrap();

        // PC2 syncs next -> hits conflict, preserves Conflict Copy + pulls PC1 edit & Dark theme!
        let mut acct2 = pc2.active_account_session().unwrap().clone();
        let batch2 = pc2.prepare_sync_batch().unwrap();
        let out2 = run_sync_cycle(&client, &mut acct2, &batch2).unwrap();
        assert_eq!(out2.conflict_count, 1);
        pc2.apply_sync_outcome(out2).unwrap();
        assert_eq!(pc2.prefs.theme, UiTheme::Dark);
        assert!(
            pc2.notebooks
                .iter()
                .any(|n| n.title == "Linear Algebra (Conflict Copy)")
        );

        // Next sync on PC2 uploads the preserved Conflict Copy, then PC1 pulls it -> convergence!
        let batch2_followup = pc2.prepare_sync_batch().unwrap();
        let out2_followup = run_sync_cycle(&client, &mut acct2, &batch2_followup).unwrap();
        pc2.apply_sync_outcome(out2_followup).unwrap();
        assert_eq!(pc2.sync_status_summary().pending_changes, 0);

        let batch1_pull = pc1.prepare_sync_batch().unwrap();
        let out1_pull = run_sync_cycle(&client, &mut acct1, &batch1_pull).unwrap();
        pc1.apply_sync_outcome(out1_pull).unwrap();

        assert_eq!(pc1.notebooks.len(), pc2.notebooks.len());
        assert!(
            pc1.notebooks
                .iter()
                .any(|n| n.title == "Linear Algebra (Conflict Copy)")
        );
    }

    #[test]
    fn oversized_notebook_returns_diagnostic_payload_too_large_error() {
        let dir = temp_dir("oversized_nb");
        let _ = ensure_data_dir_and_meta(&dir.0).unwrap();
        let db = ServerDb::open(&dir.0.join("notecraft.redb")).unwrap();
        let ttls = TokenTtls {
            access_ttl_ms: 3_600_000,
            refresh_ttl_ms: 86_400_000,
        };
        let reg = db
            .register_user(
                "oversize_user",
                "strong-password-1",
                None,
                Some("WinPC"),
                true,
                ttls,
            )
            .unwrap();
        let doc = Document {
            title: "Huge Sketchbook".to_string(),
            ..Default::default()
        };
        let req = SyncPushRequest {
            client_cursor: 0,
            device_name: "WinPC".to_string(),
            changes: vec![LocalNotebookChange {
                notebook_id: "huge-1".to_string(),
                base_version: 0,
                deleted: false,
                updated_at_ms: now_unix_ms(),
                deleted_at_ms: None,
                content_hash: canonical_document_hash(&doc),
                body_hash: canonical_document_body_hash(&doc),
                meta_hash: canonical_document_meta_hash(&doc),
                meta_updated_at_ms: now_unix_ms(),
                document: Some(doc),
            }],
            folders: Vec::new(),
            studio_prefs: None,
            force_overwrite: false,
        };

        // Compressed binary `.ncraft` v2 shrinks even empty notebooks below 128 bytes,
        // so use a 16-byte limit (smaller than the 8-byte header + zlib stream) to force the error.
        let err = db.push_changes(&reg.user.user_id, req, 16).unwrap_err();
        assert!(matches!(
            err,
            DbError::PayloadTooLarge {
                ref title,
                limit_bytes: 16,
                ..
            } if title == "Huge Sketchbook"
        ));
    }

    #[test]
    fn cors_preflight_and_headers_allow_browser_sync() {
        let dir = temp_dir("cors");
        let srv = spawn_server_on(&dir.0);
        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .http_status_as_error(false)
                .build(),
        );
        let req = ureq::http::Request::builder()
            .method("OPTIONS")
            .uri(format!("{}/api/v1/health", srv.base_url))
            .header("Origin", "http://127.0.0.1:8080")
            .header("Access-Control-Request-Method", "POST")
            .body("")
            .unwrap();
        let resp = agent.run(req).unwrap();
        assert_eq!(resp.status().as_u16(), 204);
        assert_eq!(
            resp.headers()
                .get("access-control-allow-origin")
                .and_then(|v| v.to_str().ok()),
            Some("*")
        );
    }

    #[test]
    fn local_discovery_counts_only_and_deduplicates_same_machine_reports() {
        use papercraft_sync::{
            GlobalSyncState, LocalProfile, profile_library_dir, save_global_state,
        };
        use papercraft_sync_protocol::{
            LocalDiscoveryReport, LocalDiscoverySummary, ServerInfoResponse,
        };

        let root = temp_dir("discovery_privacy");
        let server_data = root.0.join("server-data");
        let client_lib = root.0.join("library");
        let mut global = GlobalSyncState::with_defaults("Desk".to_string());
        global.client_id = "pc-client-1".to_string();
        global.local_profiles = vec![LocalProfile {
            local_profile_id: "loc-bruno".to_string(),
            nickname: "BrunoPrivateNickname".to_string(),
            created_at_ms: 1,
            last_used_at_ms: 1,
            owner_cloud_user_id: None,
        }];
        save_global_state(&client_lib, &global).unwrap();
        let prof_dir = profile_library_dir(&client_lib, "loc-bruno");
        std::fs::create_dir_all(&prof_dir).unwrap();
        std::fs::write(prof_dir.join("SecretTitle.ncraft"), b"data").unwrap();
        std::fs::write(prof_dir.join("AnotherSecret.ncraft"), b"data").unwrap();

        let srv = spawn_server_on(&server_data);
        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .http_status_as_error(false)
                .build(),
        );

        let health_json = agent
            .get(&format!("{}/api/v1/health", srv.base_url))
            .call()
            .unwrap()
            .body_mut()
            .read_to_string()
            .unwrap();
        assert!(!health_json.contains("BrunoPrivateNickname"));
        assert!(!health_json.contains("SecretTitle"));
        let info: ServerInfoResponse = serde_json::from_str(&health_json).unwrap();
        assert_eq!(
            info.local_discovery,
            LocalDiscoverySummary {
                unsynced_local_accounts: 1,
                total_local_notebooks: 2,
            }
        );

        // Reporting the same client_id via HTTP deduplicates instead of double counting.
        let mut rep_resp = agent
            .post(&format!("{}/api/v1/discovery/local-accounts", srv.base_url))
            .send_json(LocalDiscoveryReport {
                client_id: "pc-client-1".to_string(),
                unsynced_local_accounts: 1,
                total_local_notebooks: 2,
            })
            .unwrap();
        assert_eq!(rep_resp.status().as_u16(), 200);
        let combined: LocalDiscoverySummary = rep_resp.body_mut().read_json().unwrap();
        assert_eq!(combined.unsynced_local_accounts, 1);
        assert_eq!(combined.total_local_notebooks, 2);

        // A second distinct client adds cleanly.
        let mut rep2 = agent
            .post(&format!("{}/api/v1/discovery/local-accounts", srv.base_url))
            .send_json(LocalDiscoveryReport {
                client_id: "web-client-2".to_string(),
                unsynced_local_accounts: 2,
                total_local_notebooks: 3,
            })
            .unwrap();
        let total: LocalDiscoverySummary = rep2.body_mut().read_json().unwrap();
        assert_eq!(total.unsynced_local_accounts, 3);
        assert_eq!(total.total_local_notebooks, 5);
    }

    #[test]
    fn force_overwrite_and_prune_discard_unchosen_server_data() {
        use papercraft_sync_protocol::{
            PruneToLocalRequest, PruneToLocalResponse, SyncPullResponse,
        };

        let dir = temp_dir("force_prune");
        let _ = ensure_data_dir_and_meta(&dir.0).unwrap();
        let db = ServerDb::open(&dir.0.join("notecraft.redb")).unwrap();
        let ttls = TokenTtls {
            access_ttl_ms: 3_600_000,
            refresh_ttl_ms: 86_400_000,
        };
        let reg = db
            .register_user("alyx", "strong-password-1", None, Some("Desk"), true, ttls)
            .unwrap();

        let keep_server_side = Document {
            id: "nb-keep".to_string(),
            title: "Keep (Server v1)".to_string(),
            ..Default::default()
        };
        let extra_server_only = Document {
            id: "nb-extra-server".to_string(),
            title: "Server Only Note".to_string(),
            ..Default::default()
        };
        db.push_changes(
            &reg.user.user_id,
            SyncPushRequest {
                client_cursor: 0,
                device_name: "Desk".to_string(),
                changes: vec![
                    LocalNotebookChange {
                        notebook_id: "nb-keep".to_string(),
                        base_version: 0,
                        deleted: false,
                        updated_at_ms: 100,
                        deleted_at_ms: None,
                        content_hash: canonical_document_hash(&keep_server_side),
                        body_hash: canonical_document_body_hash(&keep_server_side),
                        meta_hash: canonical_document_meta_hash(&keep_server_side),
                        meta_updated_at_ms: 100,
                        document: Some(keep_server_side),
                    },
                    LocalNotebookChange {
                        notebook_id: "nb-extra-server".to_string(),
                        base_version: 0,
                        deleted: false,
                        updated_at_ms: 100,
                        deleted_at_ms: None,
                        content_hash: canonical_document_hash(&extra_server_only),
                        body_hash: canonical_document_body_hash(&extra_server_only),
                        meta_hash: canonical_document_meta_hash(&extra_server_only),
                        meta_updated_at_ms: 100,
                        document: Some(extra_server_only),
                    },
                ],
                folders: Vec::new(),
                studio_prefs: None,
                force_overwrite: false,
            },
            32 * 1024 * 1024,
        )
        .unwrap();

        // Offline client edits nb-keep with stale base_version=0, forces overwrite, and prunes nb-extra-server.
        let local_win = Document {
            id: "nb-keep".to_string(),
            title: "Keep (Local Wins)".to_string(),
            ..Default::default()
        };
        let forced = db
            .push_changes(
                &reg.user.user_id,
                SyncPushRequest {
                    client_cursor: 0,
                    device_name: "Laptop".to_string(),
                    changes: vec![LocalNotebookChange {
                        notebook_id: "nb-keep".to_string(),
                        base_version: 0,
                        deleted: false,
                        updated_at_ms: 200,
                        deleted_at_ms: None,
                        content_hash: canonical_document_hash(&local_win),
                        body_hash: canonical_document_body_hash(&local_win),
                        meta_hash: canonical_document_meta_hash(&local_win),
                        meta_updated_at_ms: 200,
                        document: Some(local_win),
                    }],
                    folders: Vec::new(),
                    studio_prefs: None,
                    force_overwrite: true,
                },
                32 * 1024 * 1024,
            )
            .unwrap();
        assert!(forced.conflicts.is_empty());
        assert_eq!(forced.accepted.len(), 1);
        assert_eq!(forced.accepted[0].new_version, 2);

        let prune: PruneToLocalResponse = db
            .prune_to_local(
                &reg.user.user_id,
                PruneToLocalRequest {
                    keep_notebook_ids: vec!["nb-keep".to_string()],
                    keep_folder_ids: Vec::new(),
                    device_name: "Laptop".to_string(),
                },
            )
            .unwrap();
        assert_eq!(prune.pruned_notebooks, 1);

        let pull: SyncPullResponse = db.pull_changes(&reg.user.user_id, 0).unwrap();
        assert_eq!(pull.notebooks.len(), 1);
        assert_eq!(pull.notebooks[0].notebook_id, "nb-keep");
        assert_eq!(pull.notebooks[0].title, "Keep (Local Wins)");
        assert_eq!(db.stats().unwrap().active_notebook_count, 1);
    }

    #[test]
    fn http_e2e_profile_update_and_change_password() {
        use papercraft_sync_protocol::{
            AuthResponse, ChangePasswordRequest, LoginRequest, RegisterRequest,
            UpdateProfileRequest, UserProfile,
        };

        let dir = temp_dir("profile_and_pw");
        let srv = spawn_server_on(&dir.0);
        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .http_status_as_error(false)
                .build(),
        );

        let mut reg_resp = agent
            .post(&format!("{}/api/v1/auth/register", srv.base_url))
            .send_json(RegisterRequest {
                email: Some("nina@example.com".to_string()),
                username: "nina_old".to_string(),
                password: "initial-strong-pw".to_string(),
                display_name: Some("Nina".to_string()),
                device_name: Some("Desk".to_string()),
            })
            .unwrap();
        assert_eq!(reg_resp.status().as_u16(), 201);
        let reg: AuthResponse = reg_resp.body_mut().read_json().unwrap();

        // Register a second user to verify username conflict handling.
        let other_resp = agent
            .post(&format!("{}/api/v1/auth/register", srv.base_url))
            .send_json(RegisterRequest {
                email: Some("bob@example.com".to_string()),
                username: "taken_bob".to_string(),
                password: "bob-password-123".to_string(),
                display_name: Some("Bob".to_string()),
                device_name: Some("Desk".to_string()),
            })
            .unwrap();
        assert_eq!(other_resp.status().as_u16(), 201);

        // Update username & display name via /api/v1/auth/update-profile.
        let mut taken_resp = agent
            .post(&format!("{}/api/v1/auth/update-profile", srv.base_url))
            .header("Authorization", &format!("Bearer {}", reg.access_token))
            .send_json(UpdateProfileRequest {
                username: Some("TAKEN_BOB".to_string()),
                display_name: None,
            })
            .unwrap();
        assert_eq!(taken_resp.status().as_u16(), 409);
        let _ = taken_resp.body_mut().read_to_string();

        let mut upd_resp = agent
            .post(&format!("{}/api/v1/auth/update-profile", srv.base_url))
            .header("Authorization", &format!("Bearer {}", reg.access_token))
            .send_json(UpdateProfileRequest {
                username: Some("nina_new".to_string()),
                display_name: Some("Nina Updated".to_string()),
            })
            .unwrap();
        assert_eq!(upd_resp.status().as_u16(), 200);
        let updated: UserProfile = upd_resp.body_mut().read_json().unwrap();
        assert_eq!(updated.user_id, reg.user.user_id);
        assert_eq!(updated.username, "nina_new");
        assert_eq!(updated.display_name, "Nina Updated");

        // Old username no longer logs in; new username does.
        let old_login = agent
            .post(&format!("{}/api/v1/auth/login", srv.base_url))
            .send_json(LoginRequest {
                email: None,
                username: "nina_old".to_string(),
                password: "initial-strong-pw".to_string(),
                device_name: None,
            })
            .unwrap();
        assert_eq!(old_login.status().as_u16(), 401);
        let new_login = agent
            .post(&format!("{}/api/v1/auth/login", srv.base_url))
            .send_json(LoginRequest {
                email: None,
                username: "nina_new".to_string(),
                password: "initial-strong-pw".to_string(),
                device_name: None,
            })
            .unwrap();
        assert_eq!(new_login.status().as_u16(), 200);

        // Voluntary password change rejects wrong current password with 400 and succeeds with valid.
        let bad_pw_resp = agent
            .post(&format!("{}/api/v1/auth/change-password", srv.base_url))
            .header("Authorization", &format!("Bearer {}", reg.access_token))
            .send_json(ChangePasswordRequest {
                current_password: "wrong-password".to_string(),
                new_password: "changed-strong-pw".to_string(),
            })
            .unwrap();
        assert_eq!(bad_pw_resp.status().as_u16(), 400);

        let ok_pw_resp = agent
            .post(&format!("{}/api/v1/auth/change-password", srv.base_url))
            .header("Authorization", &format!("Bearer {}", reg.access_token))
            .send_json(ChangePasswordRequest {
                current_password: "initial-strong-pw".to_string(),
                new_password: "changed-strong-pw".to_string(),
            })
            .unwrap();
        assert_eq!(ok_pw_resp.status().as_u16(), 200);
    }

    #[test]
    fn http_e2e_temporary_password_reset_and_delete_user() {
        use papercraft_sync_protocol::{
            ApiErrorBody, AuthResponse, CompletePasswordResetRequest,
            ERR_CODE_PASSWORD_CHANGE_REQUIRED, ERR_CODE_PASSWORD_RESET_REQUIRED, LoginRequest,
            RegisterRequest,
        };

        let dir = temp_dir("reset_and_delete");
        let srv = spawn_server_on(&dir.0);
        let agent = ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .http_status_as_error(false)
                .build(),
        );

        // 1. Register user + push a notebook so delete_user cleans notebooks too.
        let mut reg_resp = agent
            .post(&format!("{}/api/v1/auth/register", srv.base_url))
            .send_json(RegisterRequest {
                email: Some("nina@example.com".to_string()),
                username: "nina_new".to_string(),
                password: "changed-strong-pw".to_string(),
                display_name: Some("Nina".to_string()),
                device_name: Some("Desk".to_string()),
            })
            .unwrap();
        assert_eq!(reg_resp.status().as_u16(), 201);
        let reg: AuthResponse = reg_resp.body_mut().read_json().unwrap();
        let doc = Document {
            id: "nb-nina-1".to_string(),
            title: "Nina Notes".to_string(),
            ..Default::default()
        };
        let push_resp = agent
            .post(&format!("{}/api/v1/sync/push", srv.base_url))
            .header("Authorization", &format!("Bearer {}", reg.access_token))
            .send_json(SyncPushRequest {
                client_cursor: 0,
                device_name: "Desk".to_string(),
                changes: vec![LocalNotebookChange {
                    notebook_id: "nb-nina-1".to_string(),
                    base_version: 0,
                    deleted: false,
                    updated_at_ms: now_unix_ms(),
                    deleted_at_ms: None,
                    content_hash: canonical_document_hash(&doc),
                    body_hash: canonical_document_body_hash(&doc),
                    meta_hash: canonical_document_meta_hash(&doc),
                    meta_updated_at_ms: now_unix_ms(),
                    document: Some(doc),
                }],
                folders: Vec::new(),
                studio_prefs: None,
                force_overwrite: false,
            })
            .unwrap();
        assert_eq!(push_resp.status().as_u16(), 200);
        drop(srv);

        // 2. Server generates temporary random password -> existing tokens revoked.
        let db = ServerDb::open(&dir.0.join("notecraft.redb")).unwrap();
        let reset = db.reset_user_password_temporary(&reg.user.user_id).unwrap();
        assert_eq!(reset.temporary_password.len(), 14);
        assert!(db.authenticate_access_token(&reg.access_token).is_err());
        let listed = db.list_users().unwrap();
        let nina_row = listed
            .iter()
            .find(|u| u.user_id == reg.user.user_id)
            .unwrap();
        assert!(nina_row.must_change_password);
        drop(db);

        let srv2 = spawn_server_on(&dir.0);

        // Existing token stays revoked.
        let me_after_reset = agent
            .get(&format!("{}/api/v1/auth/me", srv2.base_url))
            .header("Authorization", &format!("Bearer {}", reg.access_token))
            .call()
            .unwrap();
        assert_eq!(me_after_reset.status().as_u16(), 401);

        // Logging in with old password returns 403 password_reset_required.
        let mut wrong_login = agent
            .post(&format!("{}/api/v1/auth/login", srv2.base_url))
            .send_json(LoginRequest {
                email: None,
                username: "nina_new".to_string(),
                password: "changed-strong-pw".to_string(),
                device_name: None,
            })
            .unwrap();
        assert_eq!(wrong_login.status().as_u16(), 403);
        let body1: ApiErrorBody = wrong_login.body_mut().read_json().unwrap();
        assert_eq!(body1.code, ERR_CODE_PASSWORD_RESET_REQUIRED);

        // Logging in with temporary recovery code returns 403 password_change_required (no session yet).
        let mut code_login = agent
            .post(&format!("{}/api/v1/auth/login", srv2.base_url))
            .send_json(LoginRequest {
                email: None,
                username: "nina_new".to_string(),
                password: reset.temporary_password.clone(),
                device_name: None,
            })
            .unwrap();
        assert_eq!(code_login.status().as_u16(), 403);
        let body2: ApiErrorBody = code_login.body_mut().read_json().unwrap();
        assert_eq!(body2.code, ERR_CODE_PASSWORD_CHANGE_REQUIRED);

        // Invalid recovery code returns 400.
        let bad_complete = agent
            .post(&format!(
                "{}/api/v1/auth/complete-password-reset",
                srv2.base_url
            ))
            .send_json(CompletePasswordResetRequest {
                email: None,
                username: "nina_new".to_string(),
                recovery_code: "WRONG-CODE-0000".to_string(),
                new_password: "final-strong-password".to_string(),
                device_name: Some("Desk".to_string()),
            })
            .unwrap();
        assert_eq!(bad_complete.status().as_u16(), 400);

        // Reusing the temporary recovery code as the new password returns 400.
        let same_complete = agent
            .post(&format!(
                "{}/api/v1/auth/complete-password-reset",
                srv2.base_url
            ))
            .send_json(CompletePasswordResetRequest {
                email: None,
                username: "nina_new".to_string(),
                recovery_code: reset.temporary_password.clone(),
                new_password: reset.temporary_password.clone(),
                device_name: Some("Desk".to_string()),
            })
            .unwrap();
        assert_eq!(same_complete.status().as_u16(), 400);

        // Valid recovery code + new password returns 200 AuthResponse and clears must_change_password.
        let mut ok_complete = agent
            .post(&format!(
                "{}/api/v1/auth/complete-password-reset",
                srv2.base_url
            ))
            .send_json(CompletePasswordResetRequest {
                email: Some("nina@example.com".to_string()),
                username: String::new(),
                recovery_code: reset.temporary_password.clone(),
                new_password: "final-strong-password".to_string(),
                device_name: Some("Desk".to_string()),
            })
            .unwrap();
        assert_eq!(ok_complete.status().as_u16(), 200);
        let completed_auth: AuthResponse = ok_complete.body_mut().read_json().unwrap();
        assert!(!completed_auth.user.must_change_password);
        assert_eq!(completed_auth.user.username, "nina_new");

        // Regular login with the newly chosen password now succeeds cleanly.
        let final_login = agent
            .post(&format!("{}/api/v1/auth/login", srv2.base_url))
            .send_json(LoginRequest {
                email: None,
                username: "nina_new".to_string(),
                password: "final-strong-password".to_string(),
                device_name: Some("Laptop".to_string()),
            })
            .unwrap();
        assert_eq!(final_login.status().as_u16(), 200);
        drop(srv2);

        // 5. Permanent deletion cleans user, notebooks, and indices immediately, freeing the identifiers.
        let db2 = ServerDb::open(&dir.0.join("notecraft.redb")).unwrap();
        let del_summary = db2.delete_user_permanently(&reg.user.user_id).unwrap();
        assert_eq!(del_summary.user_id, reg.user.user_id);
        assert_eq!(del_summary.deleted_notebooks, 1);
        assert_eq!(db2.stats().unwrap().user_count, 0);
        assert_eq!(db2.stats().unwrap().active_notebook_count, 0);

        let re_reg = db2
            .register_user_with_email(
                Some("nina@example.com"),
                "nina_new",
                "brand-new-password-1",
                Some("Nina Again"),
                Some("Desk"),
                true,
                TokenTtls {
                    access_ttl_ms: 3_600_000,
                    refresh_ttl_ms: 86_400_000,
                },
            )
            .unwrap();
        assert_ne!(re_reg.user.user_id, reg.user.user_id);
    }

    fn test_http_agent() -> ureq::Agent {
        ureq::Agent::new_with_config(
            ureq::config::Config::builder()
                .http_status_as_error(false)
                .build(),
        )
    }

    fn register_and_push_single_notebook(
        base_url: &str,
        agent: &ureq::Agent,
        email: &str,
        username: &str,
        notebook_id: &str,
    ) -> papercraft_sync_protocol::AuthResponse {
        use papercraft_sync_protocol::{AuthResponse, RegisterRequest};

        let mut reg_resp = agent
            .post(&format!("{base_url}/api/v1/auth/register"))
            .send_json(RegisterRequest {
                email: Some(email.to_string()),
                username: username.to_string(),
                password: "strong-password-1".to_string(),
                display_name: Some("Nina".to_string()),
                device_name: Some("Desk".to_string()),
            })
            .unwrap();
        assert_eq!(reg_resp.status().as_u16(), 201);
        let reg: AuthResponse = reg_resp.body_mut().read_json().unwrap();
        let doc = Document {
            id: notebook_id.to_string(),
            title: "Trash Test Notes".to_string(),
            ..Default::default()
        };
        let push_resp = agent
            .post(&format!("{base_url}/api/v1/sync/push"))
            .header("Authorization", &format!("Bearer {}", reg.access_token))
            .send_json(SyncPushRequest {
                client_cursor: 0,
                device_name: "Desk".to_string(),
                changes: vec![LocalNotebookChange {
                    notebook_id: notebook_id.to_string(),
                    base_version: 0,
                    deleted: false,
                    updated_at_ms: now_unix_ms(),
                    deleted_at_ms: None,
                    content_hash: canonical_document_hash(&doc),
                    body_hash: canonical_document_body_hash(&doc),
                    meta_hash: canonical_document_meta_hash(&doc),
                    meta_updated_at_ms: now_unix_ms(),
                    document: Some(doc),
                }],
                folders: Vec::new(),
                studio_prefs: None,
                force_overwrite: false,
            })
            .unwrap();
        assert_eq!(push_resp.status().as_u16(), 200);
        reg
    }

    #[test]
    fn http_e2e_server_account_trash_and_restore() {
        use papercraft_sync_protocol::{ApiErrorBody, LoginRequest, RegisterRequest};

        let dir = temp_dir("trash_and_restore");
        let agent = test_http_agent();
        let srv1 = spawn_server_on(&dir.0);
        let reg = register_and_push_single_notebook(
            &srv1.base_url,
            &agent,
            "trash_nina@example.com",
            "trash_nina",
            "nb-trash-1",
        );
        drop(srv1);

        // Moving user to Trash hides them from active lists while keeping notebooks recoverable.
        let db1 = ServerDb::open(&dir.0.join("notecraft.redb")).unwrap();
        let trash_summary = db1.trash_user(&reg.user.user_id).unwrap();
        assert_eq!(trash_summary.notebook_count, 1);
        assert_eq!(db1.stats().unwrap().user_count, 0);
        assert_eq!(db1.stats().unwrap().trashed_user_count, 1);
        assert_eq!(db1.stats().unwrap().active_notebook_count, 0);
        assert!(db1.list_users().unwrap().is_empty());
        assert_eq!(db1.list_trashed_users().unwrap().len(), 1);
        drop(db1);

        // Trashed account cannot sign in and reserves its email/username until purged.
        let srv2 = spawn_server_on(&dir.0);
        let mut trashed_login = agent
            .post(&format!("{}/api/v1/auth/login", srv2.base_url))
            .send_json(LoginRequest {
                email: None,
                username: "trash_nina".to_string(),
                password: "strong-password-1".to_string(),
                device_name: Some("Laptop".to_string()),
            })
            .unwrap();
        assert_eq!(trashed_login.status().as_u16(), 403);
        let trashed_body: ApiErrorBody = trashed_login.body_mut().read_json().unwrap();
        assert_eq!(
            trashed_body.code,
            papercraft_sync_protocol::ERR_CODE_ACCOUNT_IN_TRASH
        );

        let mut conflict_reg = agent
            .post(&format!("{}/api/v1/auth/register", srv2.base_url))
            .send_json(RegisterRequest {
                email: Some("trash_nina@example.com".to_string()),
                username: "other_name".to_string(),
                password: "brand-new-password-1".to_string(),
                display_name: None,
                device_name: None,
            })
            .unwrap();
        assert_eq!(conflict_reg.status().as_u16(), 409);
        let conflict_body: ApiErrorBody = conflict_reg.body_mut().read_json().unwrap();
        assert_eq!(
            conflict_body.code,
            papercraft_sync_protocol::ERR_CODE_EMAIL_IN_TRASH
        );
        drop(srv2);

        // Restoring from Trash brings back login access and synced notebooks cleanly.
        let db2 = ServerDb::open(&dir.0.join("notecraft.redb")).unwrap();
        let restored = db2.restore_trashed_user(&reg.user.user_id).unwrap();
        assert_eq!(restored.restored_notebooks, 1);
        assert_eq!(db2.stats().unwrap().user_count, 1);
        assert_eq!(db2.stats().unwrap().trashed_user_count, 0);
        assert_eq!(db2.stats().unwrap().active_notebook_count, 1);
        assert!(db2.list_trashed_users().unwrap().is_empty());
        drop(db2);

        let srv3 = spawn_server_on(&dir.0);
        let mut restored_login = agent
            .post(&format!("{}/api/v1/auth/login", srv3.base_url))
            .send_json(LoginRequest {
                email: None,
                username: "trash_nina".to_string(),
                password: "strong-password-1".to_string(),
                device_name: Some("Laptop".to_string()),
            })
            .unwrap();
        assert_eq!(restored_login.status().as_u16(), 200);
        let restored_auth: papercraft_sync_protocol::AuthResponse =
            restored_login.body_mut().read_json().unwrap();
        let mut pull_after_restore = agent
            .get(&format!("{}/api/v1/sync/pull?since=0", srv3.base_url))
            .header(
                "Authorization",
                &format!("Bearer {}", restored_auth.access_token),
            )
            .call()
            .unwrap();
        assert_eq!(pull_after_restore.status().as_u16(), 200);
        let pulled_after_restore: papercraft_sync_protocol::SyncPullResponse =
            pull_after_restore.body_mut().read_json().unwrap();
        assert_eq!(pulled_after_restore.notebooks.len(), 1);
        assert_eq!(pulled_after_restore.notebooks[0].notebook_id, "nb-trash-1");
    }

    #[test]
    fn http_e2e_server_account_trash_auto_expires_after_30_days() {
        let dir = temp_dir("trash_auto_expiry");
        let agent = test_http_agent();
        let srv = spawn_server_on(&dir.0);
        let reg = register_and_push_single_notebook(
            &srv.base_url,
            &agent,
            "expiring_nina@example.com",
            "expiring_nina",
            "nb-exp-1",
        );
        drop(srv);

        let db = ServerDb::open(&dir.0.join("notecraft.redb")).unwrap();
        let trashed = db.trash_user(&reg.user.user_id).unwrap();
        assert_eq!(
            db.cleanup_expired_trashed_users_at(trashed.deleted_at_ms + 29 * 24 * 60 * 60 * 1000)
                .unwrap(),
            0
        );
        assert_eq!(db.list_trashed_users().unwrap().len(), 1);
        assert_eq!(
            db.cleanup_expired_trashed_users_at(trashed.expires_at_ms)
                .unwrap(),
            1
        );
        assert!(db.list_trashed_users().unwrap().is_empty());
        assert_eq!(db.stats().unwrap().user_count, 0);
        assert_eq!(db.stats().unwrap().trashed_user_count, 0);
        assert_eq!(db.stats().unwrap().active_notebook_count, 0);

        // Once expired and purged, the email and username are free again.
        let re_reg = db
            .register_user_with_email(
                Some("expiring_nina@example.com"),
                "expiring_nina",
                "brand-new-password-1",
                Some("Nina Again"),
                Some("Desk"),
                true,
                TokenTtls {
                    access_ttl_ms: 3_600_000,
                    refresh_ttl_ms: 86_400_000,
                },
            )
            .unwrap();
        assert_ne!(re_reg.user.user_id, reg.user.user_id);
    }
}
