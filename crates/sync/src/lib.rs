//! L4 HTTP sync client, real-time server driver, and local profile metadata store for PaperCraft.

#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

pub mod client;
pub mod engine;
pub mod merge;
pub mod store;
pub mod types;

pub use client::SyncHttpClient;
pub use engine::{SyncCycleDriver, outcome_from_full_pull, run_sync_cycle};
pub use merge::{
    DocumentMergeBase, MergeDecision, MergeKind, PageElementFingerprints,
    compute_document_page_elements, compute_page_elements, merge_documents_three_way,
    strip_conflict_copy_suffix,
};
pub use store::{
    account_library_dir, default_device_name, inspect_local_accounts_in_library, load_global_state,
    load_manifest, profile_library_dir, sanitize_account_dir_segment, sanitize_client_id,
    sanitize_profile_nickname, sanitize_remembered_accounts, save_global_state, save_manifest,
    write_bytes_atomic,
};
pub use types::{
    ACCOUNTS_DIR, ActiveAccountSession, ActiveWorkspaceTarget, EntitySyncStatus, FolderSyncMeta,
    GLOBAL_SYNC_FILE, GlobalSyncState, LocalNotebookSnapshot, LocalProfile, NotebookSyncMeta,
    NotebookTombstone, OwnerId, PROFILES_DIR, RememberedServerAccount, RemoteNotebookUpdate,
    SYNC_MANIFEST_FILE, SyncBatch, SyncConnectionState, SyncCycleOutcome, SyncError, SyncManifest,
    SyncStatusSummary,
};

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::path::PathBuf;

    use papercraft_doc::{Document, NotebookCover};
    use papercraft_sync_protocol::canonical_document_hash;

    use crate::store::{
        account_library_dir, inspect_local_accounts_in_library, load_global_state, load_manifest,
        profile_library_dir, save_global_state, save_manifest,
    };
    use crate::types::{GlobalSyncState, LocalProfile, SyncManifest};

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
        let p = std::env::temp_dir().join(format!("papercraft_sync_test_{label}_{nanos}"));
        let _ = std::fs::remove_dir_all(&p);
        TempDirGuard(p)
    }

    #[test]
    fn global_state_and_manifest_roundtrip_and_dirtiness_tracking() {
        let dir = temp_dir("manifest_roundtrip");

        let mut global = GlobalSyncState::with_defaults("MyLaptop".to_string());
        global.server_url = "100.80.1.5:8787".to_string();
        save_global_state(&dir.0, &global).unwrap();

        let loaded_global = load_global_state(&dir.0);
        assert_eq!(loaded_global.server_url, "http://100.80.1.5:8787");
        assert_eq!(loaded_global.device_name, "MyLaptop");

        let acct_dir = account_library_dir(&dir.0, "user-abc/../../bad");
        assert!(
            acct_dir.ends_with("accounts\\user-abcbad")
                || acct_dir.ends_with("accounts/user-abcbad")
        );

        let mut manifest = SyncManifest::default();
        let doc = Document {
            title: "Geometry".to_string(),
            cover: NotebookCover::ClassicNavy,
            ..Default::default()
        };
        let hash = canonical_document_hash(&doc);

        manifest.mark_notebook_dirty("nb-local", &hash);
        assert_eq!(manifest.pending_changes_count(), 1);

        manifest.mark_notebook_synced("nb-local", 1, hash.clone(), &hash, 12345);
        assert_eq!(manifest.pending_changes_count(), 0);

        // Local edit changes hash -> becomes dirty without erasing synced server version!
        manifest.mark_notebook_dirty("nb-local", "new-hash-after-offline-edit");
        assert_eq!(manifest.pending_changes_count(), 1);
        assert_eq!(manifest.meta("nb-local").unwrap().server_version, 1);

        // Deleting a synced notebook creates a tombstone.
        manifest.record_local_delete("nb-local", 999);
        assert_eq!(manifest.tombstones.len(), 1);
        assert_eq!(manifest.tombstones[0].base_version, 1);

        // Deleting an unsynced local-only notebook leaves zero tombstones.
        manifest.mark_notebook_dirty("nb-never-synced", "abc");
        manifest.record_local_delete("nb-never-synced", 1000);
        assert_eq!(manifest.tombstones.len(), 1);

        save_manifest(&acct_dir, &manifest).unwrap();
        let loaded_manifest = load_manifest(&acct_dir);
        assert_eq!(loaded_manifest, manifest);
    }

    #[test]
    fn legacy_persisted_session_migrates_to_cached_session_and_forgets_cleanly() {
        let dir = temp_dir("legacy_migrate");
        std::fs::create_dir_all(&dir.0).unwrap();
        let legacy_json = serde_json::json!({
            "server_url": "http://127.0.0.1:8787",
            "device_name": "Desk",
            "session": {
                "server_url": "http://127.0.0.1:8787/",
                "user": {
                    "user_id": "u-legacy",
                    "email": "legacy@example.com",
                    "username": "legacy_user",
                    "display_name": "Legacy User",
                    "created_at_ms": 100,
                    "must_change_password": false
                },
                "access_token": "secret-access-token",
                "refresh_token": "secret-refresh-token",
                "access_expires_at_ms": 999999,
                "refresh_expires_at_ms": 999999,
                "device_name": "Desk",
                "last_auth_ms": 100
            }
        });
        std::fs::write(
            dir.0.join(crate::types::GLOBAL_SYNC_FILE),
            serde_json::to_vec_pretty(&legacy_json).unwrap(),
        )
        .unwrap();

        let mut loaded = load_global_state(&dir.0);
        assert!(loaded.legacy_session.is_none());
        assert_eq!(loaded.remembered_server_accounts.len(), 1);
        assert_eq!(loaded.cached_server_sessions.len(), 1);
        assert!(loaded.has_cached_server_session("http://127.0.0.1:8787", "u-legacy"));
        assert_eq!(loaded.remembered_server_accounts[0].user_id, "u-legacy");
        assert_eq!(
            loaded.remembered_server_accounts[0].server_url,
            "http://127.0.0.1:8787"
        );

        save_global_state(&dir.0, &loaded).unwrap();
        let reloaded = load_global_state(&dir.0);
        assert_eq!(reloaded.cached_server_sessions.len(), 1);

        assert!(loaded.forget_server_account("http://127.0.0.1:8787/", "u-legacy"));
        assert!(loaded.remembered_server_accounts.is_empty());
        assert!(loaded.cached_server_sessions.is_empty());
        assert!(!loaded.has_cached_server_session("http://127.0.0.1:8787", "u-legacy"));
    }

    #[test]
    fn inspect_local_accounts_in_library_returns_counts_only() {
        let dir = temp_dir("inspect_counts");
        assert!(inspect_local_accounts_in_library(&dir.0).is_none());

        let mut global = GlobalSyncState::with_defaults("Desk".to_string());
        global.client_id = "client-a".to_string();
        global.local_profiles = vec![
            LocalProfile {
                local_profile_id: "p1".to_string(),
                nickname: "BrunoSecret".to_string(),
                created_at_ms: 1,
                last_used_at_ms: 1,
            },
            LocalProfile {
                local_profile_id: "p2".to_string(),
                nickname: "SecondSecret".to_string(),
                created_at_ms: 2,
                last_used_at_ms: 2,
            },
        ];
        save_global_state(&dir.0, &global).unwrap();

        let p1_dir = profile_library_dir(&dir.0, "p1");
        std::fs::create_dir_all(p1_dir.join("trash")).unwrap();
        std::fs::write(p1_dir.join("a.pcraft"), b"dummy").unwrap();
        std::fs::write(p1_dir.join("b.NCRAFT"), b"dummy").unwrap();
        // Trashed notebooks must not inflate active count.
        std::fs::write(p1_dir.join("trash").join("old.pcraft"), b"dummy").unwrap();

        let p2_dir = profile_library_dir(&dir.0, "p2");
        std::fs::create_dir_all(&p2_dir).unwrap();
        std::fs::write(p2_dir.join("c.pcraft"), b"dummy").unwrap();

        let (cid, summary) = inspect_local_accounts_in_library(&dir.0).unwrap();
        assert_eq!(cid, "client-a");
        assert_eq!(summary.unsynced_local_accounts, 2);
        assert_eq!(summary.total_local_notebooks, 3);

        // Before completing initial nickname, report 0/0.
        global.needs_initial_nickname = true;
        save_global_state(&dir.0, &global).unwrap();
        let (_, pending_summary) = inspect_local_accounts_in_library(&dir.0).unwrap();
        assert_eq!(pending_summary.unsynced_local_accounts, 0);
        assert_eq!(pending_summary.total_local_notebooks, 0);
    }

    #[test]
    fn write_bytes_atomic_handles_concurrent_threads_without_leaving_temp_files() {
        let dir = temp_dir("atomic_concurrent");
        std::fs::create_dir_all(&dir.0).unwrap();
        let target = dir.0.join("notebook.pcraft");

        let payloads: Vec<Vec<u8>> = (0..4_u8)
            .map(|tid| vec![b'A' + tid; 16 * 1024 + usize::from(tid) * 37])
            .collect();

        let mut handles = Vec::new();
        for (tid, payload) in payloads.iter().enumerate() {
            let path = target.clone();
            let bytes = payload.clone();
            handles.push(std::thread::spawn(move || {
                for iter in 0..12 {
                    let mut buf = bytes.clone();
                    buf[0] = b'0' + (tid as u8);
                    if let Some(last) = buf.last_mut() {
                        *last = b'a' + ((iter % 26) as u8);
                    }
                    crate::write_bytes_atomic(&path, &buf).expect("atomic write should succeed");
                }
            }));
        }
        for handle in handles {
            handle.join().unwrap();
        }

        let final_bytes = std::fs::read(&target).unwrap();
        assert!(
            final_bytes.len() >= 16 * 1024,
            "final file should never be truncated"
        );
        let leftovers: Vec<String> = std::fs::read_dir(&dir.0)
            .unwrap()
            .flatten()
            .filter_map(|e| e.file_name().into_string().ok())
            .filter(|name| name.contains(".tmp."))
            .collect();
        assert!(
            leftovers.is_empty(),
            "unexpected leftover temp files: {leftovers:?}"
        );
    }
}
