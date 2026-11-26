//! Asynchronous, granular IndexedDB persistence, SHA-256 `blobs` deduplication,
//! zero-loss `localStorage` migration, StorageManager integration, and multi-tab protection.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use js_sys::{Array, Date, Promise, Reflect, Uint8Array};
use papercraft_doc::Document;
use papercraft_engine::web_storage::{
    BlobHashCache, DETACHED_DOC_MAGIC, WebStorageDelta, WebStorageLoadedState,
    WebStorageLoadedWorkspace, WebWorkspaceMetaSnapshot, decode_detached_notebook,
    decode_detached_workspace, encode_detached_notebook, encode_detached_workspace,
    extract_detached_notebook_blob_hashes, extract_detached_workspace_blob_hashes,
    migrate_legacy_web_cache_json, scoped_item_key, workspace_key_prefix_bounds,
};
use papercraft_sync::GlobalSyncState;
use papercraft_ui_egui::{WebStorageBackend, WebStorageEvent, WebStorageStatus};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::{JsFuture, spawn_local};

use crate::web_files::{self, SharedEguiCtx};

const DB_NAME: &str = "papercraft_storage";
const DB_VERSION: u32 = 1;
const STORE_META: &str = "meta";
const STORE_WORKSPACES: &str = "workspaces";
const STORE_NOTEBOOKS: &str = "notebooks";
const STORE_TRASH: &str = "trash";
const STORE_BLOBS: &str = "blobs";
const ALL_STORES: [&str; 5] = [
    STORE_META,
    STORE_WORKSPACES,
    STORE_NOTEBOOKS,
    STORE_TRASH,
    STORE_BLOBS,
];
const GC_STORES: [&str; 4] = [STORE_WORKSPACES, STORE_NOTEBOOKS, STORE_TRASH, STORE_BLOBS];

const META_KEY_SCHEMA: &str = "schema";
const META_KEY_GLOBAL: &str = "global";
const BROADCAST_CHANNEL_NAME: &str = "papercraft_storage_sync_v1";

const DEBOUNCE_TRAILING_MS: i32 = 400;
const DEBOUNCE_MAX_WAIT_MS: f64 = 2_000.0;
const ESTIMATE_REFRESH_MIN_MS: f64 = 30_000.0;
const PEER_STALE_MS: f64 = 90_000.0;
const GC_IDLE_DELAY_MS: i32 = 2_500;
const STARTUP_GC_DELAY_MS: i32 = 5_000;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct SchemaMarker {
    version: u32,
    migrated_from_local_storage_v1: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct TabSyncMessage {
    kind: String,
    tab: String,
}

#[derive(Clone, Debug)]
struct IdbOpError {
    quota_exceeded: bool,
    message: String,
}

impl IdbOpError {
    fn plain(message: impl Into<String>) -> Self {
        Self {
            quota_exceeded: false,
            message: message.into(),
        }
    }

    fn from_js(prefix: &str, err: &JsValue) -> Self {
        let mut name = String::new();
        let mut detail = String::new();
        if let Ok(dom) = err.clone().dyn_into::<web_sys::DomException>() {
            name = dom.name();
            detail = dom.message();
        } else {
            if let Ok(n) = Reflect::get(err, &JsValue::from_str("name"))
                && let Some(s) = n.as_string()
            {
                name = s;
            }
            if let Ok(m) = Reflect::get(err, &JsValue::from_str("message"))
                && let Some(s) = m.as_string()
            {
                detail = s;
            } else if let Some(s) = err.as_string() {
                detail = s;
            }
        }
        let lower_name = name.to_ascii_lowercase();
        let lower_detail = detail.to_ascii_lowercase();
        let quota_exceeded = lower_name.contains("quotaexceeded")
            || lower_name.contains("quota_reached")
            || lower_detail.contains("quota");
        let combined = match (name.is_empty(), detail.is_empty()) {
            (false, false) => format!("{prefix} ({name}: {detail})"),
            (false, true) => format!("{prefix} ({name})"),
            (true, false) => format!("{prefix}: {detail}"),
            (true, true) => prefix.to_string(),
        };
        Self {
            quota_exceeded,
            message: combined,
        }
    }
}

struct EncodedDelta {
    global_json: Option<String>,
    delete_workspaces: Vec<String>,
    put_workspaces: Vec<(String, String)>,
    delete_notebooks: Vec<String>,
    put_notebooks: Vec<(String, Vec<u8>)>,
    delete_trash: Vec<String>,
    put_trash: Vec<(String, Vec<u8>)>,
    put_blobs: Vec<(String, Arc<Vec<u8>>)>,
    schema_migrated: bool,
}

type EventClosureSlot = Rc<RefCell<Option<Closure<dyn FnMut(web_sys::Event)>>>>;

struct FallbackMirror {
    global: Option<GlobalSyncState>,
    workspaces: HashMap<String, WebStorageLoadedWorkspace>,
}

struct WebStorageInner {
    backend: WebStorageBackend,
    db: Option<web_sys::IdbDatabase>,
    ctx_slot: SharedEguiCtx,
    events: Arc<Mutex<Vec<WebStorageEvent>>>,
    tab_id: String,
    channel: Option<web_sys::BroadcastChannel>,
    peer_tabs: HashMap<String, f64>,
    other_tab_wrote: bool,
    persisted: Option<bool>,
    usage_bytes: Option<u64>,
    quota_bytes: Option<u64>,
    last_estimate_ms: f64,
    tried_persist_after_write: bool,
    in_flight: bool,
    has_unsaved_error: bool,
    quota_exceeded: bool,
    last_error: Option<String>,
    pending_delta: WebStorageDelta,
    failed_delta: Option<WebStorageDelta>,
    first_pending_at_ms: Option<f64>,
    debounce_timer_id: Option<i32>,
    gc_timer_id: Option<i32>,
    hash_cache: BlobHashCache,
    known_blobs: HashSet<String>,
    fallback_mirror: Option<FallbackMirror>,
}

impl WebStorageInner {
    fn status_snapshot(&mut self) -> WebStorageStatus {
        self.prune_stale_peers();
        WebStorageStatus {
            backend: self.backend,
            persisted: self.persisted,
            usage_bytes: self.usage_bytes,
            quota_bytes: self.quota_bytes,
            in_flight: self.in_flight,
            has_unsaved_error: self.has_unsaved_error,
            quota_exceeded: self.quota_exceeded,
            last_error: self.last_error.clone(),
            other_tab_active: !self.peer_tabs.is_empty(),
            other_tab_wrote: self.other_tab_wrote,
        }
    }

    fn emit_status(&mut self) {
        let status = self.status_snapshot();
        if let Ok(mut guard) = self.events.lock() {
            guard.push(WebStorageEvent::Status(status));
        }
        if let Ok(guard) = self.ctx_slot.lock()
            && let Some(egui_ctx) = guard.as_ref()
        {
            egui_ctx.request_repaint();
        }
    }

    fn prune_stale_peers(&mut self) {
        let now = Date::now();
        self.peer_tabs
            .retain(|_, seen| (now - *seen) <= PEER_STALE_MS);
    }

    fn has_unflushed_work(&self) -> bool {
        self.in_flight || !self.pending_delta.is_empty() || self.failed_delta.is_some()
    }

    fn can_run_gc(&mut self) -> bool {
        self.prune_stale_peers();
        self.backend == WebStorageBackend::IndexedDb
            && self.db.is_some()
            && self.peer_tabs.is_empty()
            && !self.in_flight
            && self.pending_delta.is_empty()
            && self.failed_delta.is_none()
    }

    fn broadcast(&self, kind: &str) {
        let Some(ref ch) = self.channel else {
            return;
        };
        let msg = TabSyncMessage {
            kind: kind.to_string(),
            tab: self.tab_id.clone(),
        };
        if let Ok(json) = serde_json::to_string(&msg) {
            let _ = ch.post_message(&JsValue::from_str(&json));
        }
    }
}

/// Shared WebAssembly storage controller driving asynchronous IndexedDB writes, quota/persistence
/// monitoring, multi-tab coordination, and browser lifecycle flush hooks.
#[derive(Clone)]
pub struct WebStorageController {
    inner: Rc<RefCell<WebStorageInner>>,
}

impl WebStorageController {
    /// Initializes IndexedDB (migrating any legacy `localStorage` payload if needed), probes
    /// `StorageManager` persistence/quota, installs lifecycle & multi-tab listeners, and returns
    /// any loaded workspace state.
    pub async fn init_and_load(ctx_slot: SharedEguiCtx) -> (Self, Option<WebStorageLoadedState>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let tab_id = Uuid::new_v4().to_string();
        let inner = Rc::new(RefCell::new(WebStorageInner {
            backend: WebStorageBackend::IndexedDb,
            db: None,
            ctx_slot,
            events,
            tab_id,
            channel: None,
            peer_tabs: HashMap::new(),
            other_tab_wrote: false,
            persisted: None,
            usage_bytes: None,
            quota_bytes: None,
            last_estimate_ms: -ESTIMATE_REFRESH_MIN_MS,
            tried_persist_after_write: false,
            in_flight: false,
            has_unsaved_error: false,
            quota_exceeded: false,
            last_error: None,
            pending_delta: WebStorageDelta::default(),
            failed_delta: None,
            first_pending_at_ms: None,
            debounce_timer_id: None,
            gc_timer_id: None,
            hash_cache: BlobHashCache::default(),
            known_blobs: HashSet::new(),
            fallback_mirror: None,
        }));
        let ctrl = Self { inner };
        let loaded = ctrl.bootstrap().await;
        ctrl.install_broadcast_channel();
        ctrl.install_lifecycle_listeners();
        ctrl.refresh_storage_manager_state(true).await;
        ctrl.inner.borrow_mut().emit_status();
        ctrl.schedule_gc(STARTUP_GC_DELAY_MS);
        (ctrl, loaded)
    }

    async fn bootstrap(&self) -> Option<WebStorageLoadedState> {
        match open_idb_database().await {
            Ok(db) => {
                self.inner.borrow_mut().db = Some(db.clone());
                self.load_or_migrate_from_idb(&db).await
            }
            Err(err) => {
                log::warn!("IndexedDB unavailable, probing fallback: {}", err.message);
                self.init_fallback_from_legacy(&err)
            }
        }
    }

    async fn load_or_migrate_from_idb(
        &self,
        db: &web_sys::IdbDatabase,
    ) -> Option<WebStorageLoadedState> {
        let schema_marker = read_schema_marker(db).await.ok().flatten();
        let needs_migration = !schema_marker
            .as_ref()
            .is_some_and(|m| m.migrated_from_local_storage_v1);

        if needs_migration && let Some(legacy_json) = web_files::load_workspace_cache() {
            set_splash_status("Migrating local workspace storage...");
            match migrate_legacy_web_cache_json(&legacy_json) {
                Ok((loaded, migration_delta)) => {
                    if let Some(theme) = migration_delta.theme_hint {
                        web_files::save_theme_hint(theme);
                    }
                    let encoded_res = {
                        let mut st = self.inner.borrow_mut();
                        encode_delta_for_idb(&mut st, &migration_delta, true)
                    };
                    match encoded_res {
                        Ok(encoded) => {
                            let new_hashes: Vec<String> =
                                encoded.put_blobs.iter().map(|(h, _)| h.clone()).collect();
                            match write_encoded_delta_to_idb(db, &encoded).await {
                                Ok(()) => {
                                    {
                                        let mut st = self.inner.borrow_mut();
                                        st.known_blobs.extend(new_hashes);
                                    }
                                    web_files::remove_legacy_workspace_caches();
                                }
                                Err(err) => {
                                    let mut st = self.inner.borrow_mut();
                                    st.has_unsaved_error = true;
                                    st.quota_exceeded = err.quota_exceeded;
                                    st.last_error = Some(err.message);
                                    st.failed_delta = Some(migration_delta);
                                }
                            }
                        }
                        Err(err_msg) => {
                            let mut st = self.inner.borrow_mut();
                            st.has_unsaved_error = true;
                            st.last_error = Some(err_msg);
                            st.failed_delta = Some(migration_delta);
                        }
                    }
                    return Some(loaded);
                }
                Err(err_msg) => {
                    log::warn!("Skipping corrupt legacy workspace cache: {err_msg}");
                }
            }
        }

        match read_full_idb_state(db).await {
            Ok((loaded_opt, known_blobs)) => {
                {
                    let mut st = self.inner.borrow_mut();
                    st.known_blobs = known_blobs;
                }
                if needs_migration {
                    let _ = write_schema_marker(db).await;
                }
                loaded_opt
            }
            Err(err) => {
                let mut st = self.inner.borrow_mut();
                st.last_error = Some(err.message);
                None
            }
        }
    }

    fn init_fallback_from_legacy(&self, idb_err: &IdbOpError) -> Option<WebStorageLoadedState> {
        let loaded = web_files::load_workspace_cache()
            .and_then(|raw| migrate_legacy_web_cache_json(&raw).ok())
            .map(|(state, _)| state);
        let can_local_storage = test_local_storage_writable();
        let mut st = self.inner.borrow_mut();
        st.backend = if can_local_storage {
            WebStorageBackend::LocalStorageFallback
        } else {
            WebStorageBackend::MemoryOnly
        };
        st.last_error = Some(idb_err.message.clone());
        if let Some(ref state) = loaded {
            let mut mirror_ws = HashMap::new();
            for ws in &state.workspaces {
                mirror_ws.insert(ws.workspace_key.clone(), ws.clone());
            }
            st.fallback_mirror = Some(FallbackMirror {
                global: Some(state.global.clone()),
                workspaces: mirror_ws,
            });
        } else {
            st.fallback_mirror = Some(FallbackMirror {
                global: None,
                workspaces: HashMap::new(),
            });
        }
        loaded
    }

    /// Enqueues a granular [`WebStorageDelta`] for trailing-debounced or immediate IndexedDB commit.
    pub fn enqueue_delta(&self, delta: WebStorageDelta) {
        if delta.is_empty() {
            return;
        }
        if let Some(theme) = delta.theme_hint {
            web_files::save_theme_hint(theme);
        }

        let (backend, urgent, should_flush_now) = {
            let mut st = self.inner.borrow_mut();
            let urgent = delta.urgent;
            let now = Date::now();
            if st.pending_delta.is_empty() {
                st.first_pending_at_ms = Some(now);
            }
            st.pending_delta.merge(delta);

            let exceeded_max_wait = st
                .first_pending_at_ms
                .is_some_and(|start| (now - start) >= DEBOUNCE_MAX_WAIT_MS);
            let flush_now = !st.in_flight && (urgent || exceeded_max_wait);
            if flush_now && let Some(timer_id) = st.debounce_timer_id.take() {
                clear_window_timeout(timer_id);
            }
            (st.backend, urgent, flush_now)
        };

        if backend != WebStorageBackend::IndexedDb {
            self.flush_fallback_now();
            return;
        }

        if should_flush_now {
            self.start_async_idb_flush();
        } else if !urgent {
            self.schedule_debounced_flush();
        }
    }

    /// Re-attempts any previously failed IndexedDB write.
    pub fn retry_failed(&self) {
        let should_flush = {
            let mut st = self.inner.borrow_mut();
            if let Some(failed) = st.failed_delta.take() {
                let pending = std::mem::take(&mut st.pending_delta);
                let mut combined = failed;
                combined.merge(pending);
                combined.urgent = true;
                st.pending_delta = combined;
                st.has_unsaved_error = false;
                st.quota_exceeded = false;
                st.last_error = None;
            }
            !st.in_flight && !st.pending_delta.is_empty()
        };
        if should_flush {
            let backend = self.inner.borrow().backend;
            if backend == WebStorageBackend::IndexedDb {
                self.start_async_idb_flush();
            } else {
                self.flush_fallback_now();
            }
        }
    }

    /// Drains pending status events for the UI shell.
    pub fn drain_events(&self) -> Vec<WebStorageEvent> {
        let events_arc = Arc::clone(&self.inner.borrow().events);
        if let Ok(mut guard) = events_arc.lock() {
            std::mem::take(&mut *guard)
        } else {
            Vec::new()
        }
    }

    fn schedule_debounced_flush(&self) {
        let mut st = self.inner.borrow_mut();
        if st.debounce_timer_id.is_some() {
            return;
        }
        let ctrl = self.clone();
        let cb = Closure::once(move || {
            ctrl.inner.borrow_mut().debounce_timer_id = None;
            ctrl.start_async_idb_flush();
        });
        if let Some(window) = web_sys::window()
            && let Ok(id) = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.as_ref().unchecked_ref(),
                DEBOUNCE_TRAILING_MS,
            )
        {
            st.debounce_timer_id = Some(id);
            cb.forget();
        }
    }

    fn start_async_idb_flush(&self) {
        let Some((db, delta, encoded)) = self.prepare_idb_flush_batch() else {
            return;
        };
        let ctrl = self.clone();
        spawn_local(async move {
            ctrl.execute_idb_flush(db, delta, encoded).await;
        });
    }

    fn flush_sync_on_unload(&self) {
        let backend = self.inner.borrow().backend;
        if backend != WebStorageBackend::IndexedDb {
            self.flush_fallback_now();
            return;
        }
        let Some((db, delta, encoded)) = self.prepare_idb_flush_batch() else {
            return;
        };
        // Open the IDBTransaction, queue all put/delete requests, and attach transaction
        // listeners synchronously inside the `pagehide`/`visibilitychange` handler.
        let tx_res = queue_encoded_delta_tx(&db, &encoded);
        let ctrl = self.clone();
        match tx_res {
            Ok(tx) => {
                let tx_promise = idb_transaction_promise(&tx);
                let new_hashes: Vec<String> =
                    encoded.put_blobs.iter().map(|(h, _)| h.clone()).collect();
                let had_deletions = delta.has_deletions();
                spawn_local(async move {
                    let res = resolve_idb_promise(tx_promise, "IndexedDB write failed")
                        .await
                        .map(|_| ());
                    ctrl.finish_idb_flush(delta, new_hashes, had_deletions, res)
                        .await;
                });
            }
            Err(err) => {
                let mut st = self.inner.borrow_mut();
                st.in_flight = false;
                requeue_failed_delta(&mut st, delta, err);
                st.emit_status();
            }
        }
    }

    fn prepare_idb_flush_batch(
        &self,
    ) -> Option<(web_sys::IdbDatabase, WebStorageDelta, EncodedDelta)> {
        let mut st = self.inner.borrow_mut();
        if st.in_flight {
            return None;
        }
        if let Some(timer_id) = st.debounce_timer_id.take() {
            clear_window_timeout(timer_id);
        }
        let db = st.db.clone()?;
        let mut delta = std::mem::take(&mut st.pending_delta);
        if let Some(failed) = st.failed_delta.take() {
            let mut combined = failed;
            combined.merge(delta);
            delta = combined;
        }
        st.first_pending_at_ms = None;
        if delta.is_empty() {
            return None;
        }
        let encoded = match encode_delta_for_idb(&mut st, &delta, false) {
            Ok(enc) => enc,
            Err(err) => {
                st.has_unsaved_error = true;
                st.last_error = Some(err);
                st.failed_delta = Some(delta);
                st.emit_status();
                return None;
            }
        };
        st.in_flight = true;
        Some((db, delta, encoded))
    }

    async fn execute_idb_flush(
        &self,
        db: web_sys::IdbDatabase,
        delta: WebStorageDelta,
        encoded: EncodedDelta,
    ) {
        let new_hashes: Vec<String> = encoded.put_blobs.iter().map(|(h, _)| h.clone()).collect();
        let had_deletions = delta.has_deletions();
        let res = write_encoded_delta_to_idb(&db, &encoded).await;
        self.finish_idb_flush(delta, new_hashes, had_deletions, res)
            .await;
    }

    async fn finish_idb_flush(
        &self,
        delta: WebStorageDelta,
        new_hashes: Vec<String>,
        had_deletions: bool,
        res: Result<(), IdbOpError>,
    ) {
        let (has_more_pending, should_refresh_estimate, should_try_persist, can_gc) = {
            let mut st = self.inner.borrow_mut();
            st.in_flight = false;
            match res {
                Ok(()) => {
                    let added_blobs = !new_hashes.is_empty();
                    st.known_blobs.extend(new_hashes);
                    st.has_unsaved_error = false;
                    st.quota_exceeded = false;
                    st.last_error = None;
                    st.broadcast("write");
                    let now = Date::now();
                    let est_due = added_blobs
                        || had_deletions
                        || (now - st.last_estimate_ms) >= ESTIMATE_REFRESH_MIN_MS;
                    let try_persist = st.persisted != Some(true) && !st.tried_persist_after_write;
                    if try_persist {
                        st.tried_persist_after_write = true;
                    }
                    let more = !st.pending_delta.is_empty();
                    let can_gc = had_deletions && !more && st.peer_tabs.is_empty();
                    (more, est_due, try_persist, can_gc)
                }
                Err(err) => {
                    requeue_failed_delta(&mut st, delta, err);
                    (false, true, false, false)
                }
            }
        };

        if should_try_persist || should_refresh_estimate {
            self.refresh_storage_manager_state(should_try_persist).await;
        }
        self.inner.borrow_mut().emit_status();

        if has_more_pending {
            self.start_async_idb_flush();
        } else if can_gc {
            self.schedule_gc(GC_IDLE_DELAY_MS);
        }
    }

    fn flush_fallback_now(&self) {
        let mut st = self.inner.borrow_mut();
        if let Some(timer_id) = st.debounce_timer_id.take() {
            clear_window_timeout(timer_id);
        }
        let mut delta = std::mem::take(&mut st.pending_delta);
        if let Some(failed) = st.failed_delta.take() {
            let mut combined = failed;
            combined.merge(delta);
            delta = combined;
        }
        st.first_pending_at_ms = None;
        if delta.is_empty() {
            return;
        }
        if let Some(ref mut mirror) = st.fallback_mirror {
            apply_delta_to_fallback_mirror(mirror, &delta);
        }
        if st.backend == WebStorageBackend::MemoryOnly {
            st.has_unsaved_error = true;
            st.emit_status();
            return;
        }
        let json_res = st
            .fallback_mirror
            .as_ref()
            .map(serialize_fallback_mirror_json)
            .unwrap_or_else(|| Err("fallback mirror unavailable".to_string()));
        match json_res.and_then(|json| web_files::save_workspace_cache(&json)) {
            Ok(()) => {
                st.has_unsaved_error = false;
                st.quota_exceeded = false;
                st.last_error = None;
            }
            Err(err) => {
                let is_quota = err.to_ascii_lowercase().contains("quota");
                st.has_unsaved_error = true;
                st.quota_exceeded = is_quota;
                st.last_error = Some(err);
                st.failed_delta = Some(delta);
            }
        }
        st.emit_status();
    }

    fn schedule_gc(&self, delay_ms: i32) {
        let mut st = self.inner.borrow_mut();
        if !st.can_run_gc() || st.gc_timer_id.is_some() {
            return;
        }
        let ctrl = self.clone();
        let cb = Closure::once(move || {
            ctrl.inner.borrow_mut().gc_timer_id = None;
            let ctrl_async = ctrl.clone();
            spawn_local(async move {
                ctrl_async.run_safe_blob_gc().await;
            });
        });
        if let Some(window) = web_sys::window()
            && let Ok(id) = window.set_timeout_with_callback_and_timeout_and_arguments_0(
                cb.as_ref().unchecked_ref(),
                delay_ms,
            )
        {
            st.gc_timer_id = Some(id);
            cb.forget();
        }
    }

    async fn run_safe_blob_gc(&self) {
        let db = {
            let mut st = self.inner.borrow_mut();
            if !st.can_run_gc() {
                return;
            }
            let Some(db) = st.db.clone() else {
                return;
            };
            st.in_flight = true;
            db
        };

        let result = execute_atomic_blob_gc(&db, &self.inner).await;
        let (deleted_count, has_more_pending) = {
            let mut st = self.inner.borrow_mut();
            st.in_flight = false;
            let deleted = match result {
                Ok(Some((live_hashes, count))) => {
                    st.known_blobs = live_hashes;
                    count
                }
                _ => 0,
            };
            (deleted, !st.pending_delta.is_empty())
        };
        if deleted_count > 0 {
            self.refresh_storage_manager_state(false).await;
            self.inner.borrow_mut().emit_status();
        }
        if has_more_pending {
            self.start_async_idb_flush();
        }
    }

    async fn refresh_storage_manager_state(&self, request_persist: bool) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let navigator = window.navigator();
        let Ok(storage_val) = Reflect::get(&navigator, &JsValue::from_str("storage")) else {
            return;
        };
        let Ok(storage_mgr) = storage_val.dyn_into::<web_sys::StorageManager>() else {
            return;
        };

        if let Ok(persisted_promise) = storage_mgr.persisted()
            && let Ok(val) = JsFuture::from(persisted_promise).await
            && let Some(is_persisted) = val.as_bool()
        {
            self.inner.borrow_mut().persisted = Some(is_persisted);
            if !is_persisted
                && request_persist
                && let Ok(persist_promise) = storage_mgr.persist()
                && let Ok(granted_val) = JsFuture::from(persist_promise).await
                && let Some(granted) = granted_val.as_bool()
            {
                self.inner.borrow_mut().persisted = Some(granted);
            }
        }

        if let Ok(est_promise) = storage_mgr.estimate()
            && let Ok(est_val) = JsFuture::from(est_promise).await
        {
            let usage = Reflect::get(&est_val, &JsValue::from_str("usage"))
                .ok()
                .and_then(|v| v.as_f64())
                .filter(|v| v.is_finite() && *v >= 0.0)
                .map(|v| v.round() as u64);
            let quota = Reflect::get(&est_val, &JsValue::from_str("quota"))
                .ok()
                .and_then(|v| v.as_f64())
                .filter(|v| v.is_finite() && *v >= 0.0)
                .map(|v| v.round() as u64);
            let mut st = self.inner.borrow_mut();
            st.usage_bytes = usage;
            st.quota_bytes = quota;
            st.last_estimate_ms = Date::now();
        }
    }

    fn install_broadcast_channel(&self) {
        let Some(window) = web_sys::window() else {
            return;
        };
        let has_bc = Reflect::get(&window, &JsValue::from_str("BroadcastChannel"))
            .ok()
            .is_some_and(|v| v.is_function());
        if !has_bc {
            return;
        }
        let Ok(ch) = web_sys::BroadcastChannel::new(BROADCAST_CHANNEL_NAME) else {
            return;
        };
        let ctrl = self.clone();
        let on_msg = Closure::<dyn FnMut(web_sys::MessageEvent)>::new(
            move |event: web_sys::MessageEvent| {
                let Some(raw) = event.data().as_string() else {
                    return;
                };
                let Ok(msg) = serde_json::from_str::<TabSyncMessage>(&raw) else {
                    return;
                };
                let mut st = ctrl.inner.borrow_mut();
                if msg.tab == st.tab_id {
                    return;
                }
                let now = Date::now();
                let mut reply_here = false;
                match msg.kind.as_str() {
                    "hello" => {
                        st.peer_tabs.insert(msg.tab, now);
                        reply_here = true;
                    }
                    "here" => {
                        st.peer_tabs.insert(msg.tab, now);
                    }
                    "write" => {
                        st.peer_tabs.insert(msg.tab, now);
                        st.other_tab_wrote = true;
                    }
                    "bye" => {
                        st.peer_tabs.remove(&msg.tab);
                    }
                    _ => {}
                }
                if reply_here {
                    st.broadcast("here");
                }
                st.emit_status();
            },
        );
        ch.set_onmessage(Some(on_msg.as_ref().unchecked_ref()));
        on_msg.forget();
        {
            let mut st = self.inner.borrow_mut();
            st.channel = Some(ch);
            st.broadcast("hello");
        }
    }

    fn install_lifecycle_listeners(&self) {
        let Some(window) = web_sys::window() else {
            return;
        };
        if let Some(document) = window.document() {
            let ctrl_vis = self.clone();
            let doc_ref = document.clone();
            let on_vis = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
                if doc_ref.visibility_state() == web_sys::VisibilityState::Hidden {
                    ctrl_vis.flush_sync_on_unload();
                } else {
                    ctrl_vis.inner.borrow().broadcast("hello");
                }
            });
            let target: &web_sys::EventTarget = document.as_ref();
            let _ = target.add_event_listener_with_callback(
                "visibilitychange",
                on_vis.as_ref().unchecked_ref(),
            );
            on_vis.forget();
        }

        let ctrl_pagehide = self.clone();
        let on_pagehide = Closure::<dyn FnMut(web_sys::PageTransitionEvent)>::new(move |_| {
            ctrl_pagehide.inner.borrow().broadcast("bye");
            ctrl_pagehide.flush_sync_on_unload();
        });
        let win_target: &web_sys::EventTarget = window.as_ref();
        let _ = win_target
            .add_event_listener_with_callback("pagehide", on_pagehide.as_ref().unchecked_ref());
        on_pagehide.forget();

        let ctrl_beforeunload = self.clone();
        let on_beforeunload = Closure::<dyn FnMut(web_sys::BeforeUnloadEvent)>::new(
            move |event: web_sys::BeforeUnloadEvent| {
                if ctrl_beforeunload.inner.borrow().has_unflushed_work() {
                    event.prevent_default();
                    event.set_return_value("");
                }
            },
        );
        let _ = win_target.add_event_listener_with_callback(
            "beforeunload",
            on_beforeunload.as_ref().unchecked_ref(),
        );
        on_beforeunload.forget();
    }
}

fn requeue_failed_delta(st: &mut WebStorageInner, delta: WebStorageDelta, err: IdbOpError) {
    st.has_unsaved_error = true;
    st.quota_exceeded = err.quota_exceeded;
    st.last_error = Some(err.message);
    let mut combined = delta;
    if let Some(prev) = st.failed_delta.take() {
        let mut ordered = prev;
        ordered.merge(combined);
        combined = ordered;
    }
    st.failed_delta = Some(combined);
}

fn clear_window_timeout(timer_id: i32) {
    if let Some(window) = web_sys::window() {
        window.clear_timeout_with_handle(timer_id);
    }
}

fn set_splash_status(label: &str) {
    if let Some(window) = web_sys::window()
        && let Some(document) = window.document()
        && let Some(el) = document.get_element_by_id("splash-status-label")
    {
        el.set_text_content(Some(label));
    }
}

fn test_local_storage_writable() -> bool {
    const PROBE_KEY: &str = "papercraft_probe_v1";
    let Some(window) = web_sys::window() else {
        return false;
    };
    let Ok(Some(storage)) = window.local_storage() else {
        return false;
    };
    if storage.set_item(PROBE_KEY, "1").is_err() {
        return false;
    }
    let _ = storage.remove_item(PROBE_KEY);
    true
}

fn encode_delta_for_idb(
    st: &mut WebStorageInner,
    delta: &WebStorageDelta,
    schema_migrated: bool,
) -> Result<EncodedDelta, String> {
    let global_json = delta
        .global
        .as_ref()
        .map(|g| serde_json::to_string(g).map_err(|e| format!("failed encoding global state: {e}")))
        .transpose()?;

    let mut known_with_batch = st.known_blobs.clone();
    let mut put_blobs = Vec::new();

    let mut put_workspaces = Vec::with_capacity(delta.put_workspaces.len());
    for (ws_key, ws_meta) in &delta.put_workspaces {
        let (ws_json, blobs) =
            encode_detached_workspace(ws_meta, &mut st.hash_cache, &known_with_batch)?;
        for (h, b) in blobs {
            known_with_batch.insert(h.clone());
            put_blobs.push((h, b));
        }
        put_workspaces.push((ws_key.clone(), ws_json));
    }

    let mut put_notebooks = Vec::with_capacity(delta.put_notebooks.len());
    for ((ws_key, nb_id), doc) in &delta.put_notebooks {
        let (bytes, blobs) = encode_detached_notebook(doc, &mut st.hash_cache, &known_with_batch)?;
        for (h, b) in blobs {
            known_with_batch.insert(h.clone());
            put_blobs.push((h, b));
        }
        put_notebooks.push((scoped_item_key(ws_key, nb_id), bytes));
    }

    let mut put_trash = Vec::with_capacity(delta.put_trash.len());
    for ((ws_key, tid), (deleted_at_ms, doc)) in &delta.put_trash {
        let (doc_bytes, blobs) =
            encode_detached_notebook(doc, &mut st.hash_cache, &known_with_batch)?;
        for (h, b) in blobs {
            known_with_batch.insert(h.clone());
            put_blobs.push((h, b));
        }
        let mut packed = Vec::with_capacity(8 + doc_bytes.len());
        packed.extend_from_slice(&deleted_at_ms.to_le_bytes());
        packed.extend_from_slice(&doc_bytes);
        put_trash.push((scoped_item_key(ws_key, tid), packed));
    }

    let mut delete_workspaces: Vec<String> = delta.delete_workspaces.iter().cloned().collect();
    delete_workspaces.sort();
    let delete_notebooks = delta
        .delete_notebooks
        .iter()
        .map(|(ws, id)| scoped_item_key(ws, id))
        .collect();
    let delete_trash = delta
        .delete_trash
        .iter()
        .map(|(ws, tid)| scoped_item_key(ws, tid))
        .collect();

    Ok(EncodedDelta {
        global_json,
        delete_workspaces,
        put_workspaces,
        delete_notebooks,
        put_notebooks,
        delete_trash,
        put_trash,
        put_blobs,
        schema_migrated,
    })
}

async fn open_idb_database() -> Result<web_sys::IdbDatabase, IdbOpError> {
    let window =
        web_sys::window().ok_or_else(|| IdbOpError::plain("browser window unavailable"))?;
    let factory = window
        .indexed_db()
        .map_err(|err| IdbOpError::from_js("indexedDB unavailable", &err))?
        .ok_or_else(|| IdbOpError::plain("indexedDB unavailable"))?;
    let open_req = factory
        .open_with_u32(DB_NAME, DB_VERSION)
        .map_err(|err| IdbOpError::from_js("failed opening IndexedDB", &err))?;

    let on_upgrade = Closure::<dyn FnMut(web_sys::IdbVersionChangeEvent)>::new(
        move |event: web_sys::IdbVersionChangeEvent| {
            let Some(target) = event.target() else {
                return;
            };
            let Ok(req) = target.dyn_into::<web_sys::IdbOpenDbRequest>() else {
                return;
            };
            let Ok(res) = req.result() else {
                return;
            };
            let Ok(db) = res.dyn_into::<web_sys::IdbDatabase>() else {
                return;
            };
            let names = db.object_store_names();
            for store in ALL_STORES {
                if !names.contains(store) {
                    let _ = db.create_object_store(store);
                }
            }
        },
    );
    open_req.set_onupgradeneeded(Some(on_upgrade.as_ref().unchecked_ref()));

    let open_promise = idb_open_promise(&open_req);
    let result_val = resolve_idb_promise(open_promise, "IndexedDB open failed").await;
    open_req.set_onupgradeneeded(None);
    drop(on_upgrade);

    let val = result_val?;
    val.dyn_into::<web_sys::IdbDatabase>()
        .map_err(|_| IdbOpError::plain("IndexedDB open did not return an IDBDatabase"))
}

fn idb_open_promise(req: &web_sys::IdbOpenDbRequest) -> Promise {
    let req_clone = req.clone();
    let success_slot: EventClosureSlot = Rc::new(RefCell::new(None));
    let error_slot: EventClosureSlot = Rc::new(RefCell::new(None));
    let blocked_slot: EventClosureSlot = Rc::new(RefCell::new(None));

    let s_clear = Rc::clone(&success_slot);
    let e_clear = Rc::clone(&error_slot);
    let b_clear = Rc::clone(&blocked_slot);

    Promise::new(&mut |resolve, reject| {
        let clear_handlers = {
            let r = req_clone.clone();
            let sc = Rc::clone(&s_clear);
            let ec = Rc::clone(&e_clear);
            let bc = Rc::clone(&b_clear);
            move || {
                r.set_onsuccess(None);
                r.set_onerror(None);
                r.set_onblocked(None);
                *sc.borrow_mut() = None;
                *ec.borrow_mut() = None;
                *bc.borrow_mut() = None;
            }
        };

        let clear_ok = clear_handlers.clone();
        let r_ok = req_clone.clone();
        let rej_ok = reject.clone();
        let on_success = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            let res = r_ok.result();
            clear_ok();
            match res {
                Ok(val) => {
                    let _ = resolve.call1(&JsValue::UNDEFINED, &val);
                }
                Err(err) => {
                    let _ = rej_ok.call1(&JsValue::UNDEFINED, &err);
                }
            }
        });

        let clear_err = clear_handlers.clone();
        let rej_err = reject.clone();
        let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |evt: web_sys::Event| {
            let err_obj = evt
                .target()
                .and_then(|t| Reflect::get(&t, &JsValue::from_str("error")).ok())
                .unwrap_or_else(|| JsValue::from_str("IndexedDB open failed"));
            clear_err();
            let _ = rej_err.call1(&JsValue::UNDEFINED, &err_obj);
        });

        let clear_blk = clear_handlers;
        let on_blocked = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            clear_blk();
            let _ = reject.call1(
                &JsValue::UNDEFINED,
                &JsValue::from_str("IndexedDB upgrade blocked by another tab"),
            );
        });

        req_clone.set_onsuccess(Some(on_success.as_ref().unchecked_ref()));
        req_clone.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        req_clone.set_onblocked(Some(on_blocked.as_ref().unchecked_ref()));
        *success_slot.borrow_mut() = Some(on_success);
        *error_slot.borrow_mut() = Some(on_error);
        *blocked_slot.borrow_mut() = Some(on_blocked);
    })
}

fn idb_request_promise(req: &web_sys::IdbRequest) -> Promise {
    let req_clone = req.clone();
    let success_slot: EventClosureSlot = Rc::new(RefCell::new(None));
    let error_slot: EventClosureSlot = Rc::new(RefCell::new(None));

    let s_clear = Rc::clone(&success_slot);
    let e_clear = Rc::clone(&error_slot);

    Promise::new(&mut |resolve, reject| {
        let clear = {
            let r = req_clone.clone();
            let sc = Rc::clone(&s_clear);
            let ec = Rc::clone(&e_clear);
            move || {
                r.set_onsuccess(None);
                r.set_onerror(None);
                *sc.borrow_mut() = None;
                *ec.borrow_mut() = None;
            }
        };

        let clear_ok = clear.clone();
        let r_ok = req_clone.clone();
        let rej_ok = reject.clone();
        let on_success = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            let res = r_ok.result();
            clear_ok();
            match res {
                Ok(val) => {
                    let _ = resolve.call1(&JsValue::UNDEFINED, &val);
                }
                Err(err) => {
                    let _ = rej_ok.call1(&JsValue::UNDEFINED, &err);
                }
            }
        });

        let clear_err = clear;
        let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |evt: web_sys::Event| {
            let err_obj = evt
                .target()
                .and_then(|t| Reflect::get(&t, &JsValue::from_str("error")).ok())
                .unwrap_or_else(|| JsValue::from_str("IndexedDB request failed"));
            clear_err();
            let _ = reject.call1(&JsValue::UNDEFINED, &err_obj);
        });

        req_clone.set_onsuccess(Some(on_success.as_ref().unchecked_ref()));
        req_clone.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        *success_slot.borrow_mut() = Some(on_success);
        *error_slot.borrow_mut() = Some(on_error);
    })
}

fn idb_transaction_promise(tx: &web_sys::IdbTransaction) -> Promise {
    let tx_clone = tx.clone();
    let complete_slot: EventClosureSlot = Rc::new(RefCell::new(None));
    let error_slot: EventClosureSlot = Rc::new(RefCell::new(None));
    let abort_slot: EventClosureSlot = Rc::new(RefCell::new(None));

    let c_clear = Rc::clone(&complete_slot);
    let e_clear = Rc::clone(&error_slot);
    let a_clear = Rc::clone(&abort_slot);

    Promise::new(&mut |resolve, reject| {
        let clear = {
            let t = tx_clone.clone();
            let cc = Rc::clone(&c_clear);
            let ec = Rc::clone(&e_clear);
            let ac = Rc::clone(&a_clear);
            move || {
                t.set_oncomplete(None);
                t.set_onerror(None);
                t.set_onabort(None);
                *cc.borrow_mut() = None;
                *ec.borrow_mut() = None;
                *ac.borrow_mut() = None;
            }
        };

        let clear_ok = clear.clone();
        let on_complete = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            clear_ok();
            let _ = resolve.call1(&JsValue::UNDEFINED, &JsValue::TRUE);
        });

        let clear_err = clear.clone();
        let t_err = tx_clone.clone();
        let rej_err = reject.clone();
        let on_error = Closure::<dyn FnMut(web_sys::Event)>::new(move |evt: web_sys::Event| {
            let err_obj = t_err
                .error()
                .map(JsValue::from)
                .or_else(|| {
                    evt.target()
                        .and_then(|t| Reflect::get(&t, &JsValue::from_str("error")).ok())
                })
                .unwrap_or_else(|| JsValue::from_str("IndexedDB transaction failed"));
            clear_err();
            let _ = rej_err.call1(&JsValue::UNDEFINED, &err_obj);
        });

        let clear_abt = clear;
        let t_abt = tx_clone.clone();
        let on_abort = Closure::<dyn FnMut(web_sys::Event)>::new(move |evt: web_sys::Event| {
            let err_obj = t_abt
                .error()
                .map(JsValue::from)
                .or_else(|| {
                    evt.target()
                        .and_then(|t| Reflect::get(&t, &JsValue::from_str("error")).ok())
                })
                .unwrap_or_else(|| JsValue::from_str("IndexedDB transaction aborted"));
            clear_abt();
            let _ = reject.call1(&JsValue::UNDEFINED, &err_obj);
        });

        tx_clone.set_oncomplete(Some(on_complete.as_ref().unchecked_ref()));
        tx_clone.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        tx_clone.set_onabort(Some(on_abort.as_ref().unchecked_ref()));
        *complete_slot.borrow_mut() = Some(on_complete);
        *error_slot.borrow_mut() = Some(on_error);
        *abort_slot.borrow_mut() = Some(on_abort);
    })
}

async fn resolve_idb_promise(promise: Promise, prefix: &str) -> Result<JsValue, IdbOpError> {
    JsFuture::from(promise)
        .await
        .map_err(|err| IdbOpError::from_js(prefix, &err))
}

fn open_tx(
    db: &web_sys::IdbDatabase,
    stores: &[&str],
    mode: web_sys::IdbTransactionMode,
) -> Result<web_sys::IdbTransaction, IdbOpError> {
    let names = Array::new();
    for name in stores {
        names.push(&JsValue::from_str(name));
    }
    db.transaction_with_str_sequence_and_mode(&names, mode)
        .map_err(|err| IdbOpError::from_js("failed starting IndexedDB transaction", &err))
}

fn store_from_tx(
    tx: &web_sys::IdbTransaction,
    name: &str,
) -> Result<web_sys::IdbObjectStore, IdbOpError> {
    tx.object_store(name)
        .map_err(|err| IdbOpError::from_js(&format!("failed opening store `{name}`"), &err))
}

fn put_str_with_key(
    store: &web_sys::IdbObjectStore,
    key: &str,
    val: &str,
) -> Result<(), IdbOpError> {
    store
        .put_with_key(&JsValue::from_str(val), &JsValue::from_str(key))
        .map(|_| ())
        .map_err(|err| IdbOpError::from_js("IndexedDB put failed", &err))
}

fn put_bytes_with_key(
    store: &web_sys::IdbObjectStore,
    key: &str,
    bytes: &[u8],
) -> Result<(), IdbOpError> {
    let u8_arr = Uint8Array::from(bytes);
    store
        .put_with_key(&u8_arr.into(), &JsValue::from_str(key))
        .map(|_| ())
        .map_err(|err| IdbOpError::from_js("IndexedDB binary put failed", &err))
}

fn delete_key(store: &web_sys::IdbObjectStore, key: &str) -> Result<(), IdbOpError> {
    store
        .delete(&JsValue::from_str(key))
        .map(|_| ())
        .map_err(|err| IdbOpError::from_js("IndexedDB delete failed", &err))
}

fn delete_workspace_prefix(
    store: &web_sys::IdbObjectStore,
    ws_key: &str,
) -> Result<(), IdbOpError> {
    let (lower, upper) = workspace_key_prefix_bounds(ws_key);
    let range = web_sys::IdbKeyRange::bound(&JsValue::from_str(&lower), &JsValue::from_str(&upper))
        .map_err(|err| IdbOpError::from_js("failed creating workspace key range", &err))?;
    store
        .delete(&range.into())
        .map(|_| ())
        .map_err(|err| IdbOpError::from_js("IndexedDB range delete failed", &err))
}

fn queue_encoded_delta_tx(
    db: &web_sys::IdbDatabase,
    encoded: &EncodedDelta,
) -> Result<web_sys::IdbTransaction, IdbOpError> {
    let tx = open_tx(db, &ALL_STORES, web_sys::IdbTransactionMode::Readwrite)?;
    let meta = store_from_tx(&tx, STORE_META)?;
    let workspaces = store_from_tx(&tx, STORE_WORKSPACES)?;
    let notebooks = store_from_tx(&tx, STORE_NOTEBOOKS)?;
    let trash = store_from_tx(&tx, STORE_TRASH)?;
    let blobs = store_from_tx(&tx, STORE_BLOBS)?;

    if encoded.schema_migrated {
        let marker = serde_json::to_string(&SchemaMarker {
            version: 1,
            migrated_from_local_storage_v1: true,
        })
        .map_err(|e| IdbOpError::plain(format!("failed serializing schema marker: {e}")))?;
        put_str_with_key(&meta, META_KEY_SCHEMA, &marker)?;
    }
    if let Some(ref global_json) = encoded.global_json {
        put_str_with_key(&meta, META_KEY_GLOBAL, global_json)?;
    }

    for (hash, bytes) in &encoded.put_blobs {
        put_bytes_with_key(&blobs, hash, bytes.as_slice())?;
    }
    for ws_key in &encoded.delete_workspaces {
        delete_key(&workspaces, ws_key)?;
        delete_workspace_prefix(&notebooks, ws_key)?;
        delete_workspace_prefix(&trash, ws_key)?;
    }
    for (ws_key, ws_json) in &encoded.put_workspaces {
        put_str_with_key(&workspaces, ws_key, ws_json)?;
    }
    for key in &encoded.delete_notebooks {
        delete_key(&notebooks, key)?;
    }
    for (key, bytes) in &encoded.put_notebooks {
        put_bytes_with_key(&notebooks, key, bytes)?;
    }
    for key in &encoded.delete_trash {
        delete_key(&trash, key)?;
    }
    for (key, bytes) in &encoded.put_trash {
        put_bytes_with_key(&trash, key, bytes)?;
    }

    Ok(tx)
}

async fn write_encoded_delta_to_idb(
    db: &web_sys::IdbDatabase,
    encoded: &EncodedDelta,
) -> Result<(), IdbOpError> {
    let tx = queue_encoded_delta_tx(db, encoded)?;
    let tx_promise = idb_transaction_promise(&tx);
    resolve_idb_promise(tx_promise, "IndexedDB write failed")
        .await
        .map(|_| ())
}

async fn read_schema_marker(db: &web_sys::IdbDatabase) -> Result<Option<SchemaMarker>, IdbOpError> {
    let tx = open_tx(db, &[STORE_META], web_sys::IdbTransactionMode::Readonly)?;
    let meta = store_from_tx(&tx, STORE_META)?;
    let req = meta
        .get(&JsValue::from_str(META_KEY_SCHEMA))
        .map_err(|err| IdbOpError::from_js("failed reading schema marker", &err))?;
    let promise = idb_request_promise(&req);
    let val = resolve_idb_promise(promise, "failed reading schema marker").await?;
    Ok(val
        .as_string()
        .and_then(|s| serde_json::from_str::<SchemaMarker>(&s).ok()))
}

async fn write_schema_marker(db: &web_sys::IdbDatabase) -> Result<(), IdbOpError> {
    let tx = open_tx(db, &[STORE_META], web_sys::IdbTransactionMode::Readwrite)?;
    let tx_promise = idb_transaction_promise(&tx);
    let meta = store_from_tx(&tx, STORE_META)?;
    let marker = serde_json::to_string(&SchemaMarker {
        version: 1,
        migrated_from_local_storage_v1: true,
    })
    .map_err(|e| IdbOpError::plain(format!("failed serializing schema marker: {e}")))?;
    put_str_with_key(&meta, META_KEY_SCHEMA, &marker)?;
    resolve_idb_promise(tx_promise, "IndexedDB schema write failed")
        .await
        .map(|_| ())
}

fn read_all_store_pairs(
    store: &web_sys::IdbObjectStore,
) -> Result<(web_sys::IdbRequest, web_sys::IdbRequest), IdbOpError> {
    let keys_req = store
        .get_all_keys()
        .map_err(|err| IdbOpError::from_js("getAllKeys failed", &err))?;
    let vals_req = store
        .get_all()
        .map_err(|err| IdbOpError::from_js("getAll failed", &err))?;
    Ok((keys_req, vals_req))
}

fn js_value_to_bytes(val: &JsValue) -> Option<Vec<u8>> {
    if val.is_instance_of::<Uint8Array>() {
        return Some(Uint8Array::new(val).to_vec());
    }
    if val.is_instance_of::<js_sys::ArrayBuffer>() {
        return Some(Uint8Array::new(val).to_vec());
    }
    None
}

fn collect_store_byte_entries(keys_val: &JsValue, vals_val: &JsValue) -> Vec<(String, Vec<u8>)> {
    let keys = Array::from(keys_val);
    let vals = Array::from(vals_val);
    let len = keys.length().min(vals.length());
    let mut out = Vec::with_capacity(len as usize);
    for idx in 0..len {
        if let Some(k) = keys.get(idx).as_string()
            && let Some(bytes) = js_value_to_bytes(&vals.get(idx))
        {
            out.push((k, bytes));
        }
    }
    out
}

fn collect_store_str_entries(keys_val: &JsValue, vals_val: &JsValue) -> Vec<(String, String)> {
    let keys = Array::from(keys_val);
    let vals = Array::from(vals_val);
    let len = keys.length().min(vals.length());
    let mut out = Vec::with_capacity(len as usize);
    for idx in 0..len {
        if let Some(k) = keys.get(idx).as_string()
            && let Some(v) = vals.get(idx).as_string()
        {
            out.push((k, v));
        }
    }
    out
}

async fn read_full_idb_state(
    db: &web_sys::IdbDatabase,
) -> Result<(Option<WebStorageLoadedState>, HashSet<String>), IdbOpError> {
    let tx = open_tx(db, &ALL_STORES, web_sys::IdbTransactionMode::Readonly)?;
    let meta = store_from_tx(&tx, STORE_META)?;
    let workspaces = store_from_tx(&tx, STORE_WORKSPACES)?;
    let notebooks = store_from_tx(&tx, STORE_NOTEBOOKS)?;
    let trash = store_from_tx(&tx, STORE_TRASH)?;
    let blobs = store_from_tx(&tx, STORE_BLOBS)?;

    let global_req = meta
        .get(&JsValue::from_str(META_KEY_GLOBAL))
        .map_err(|err| IdbOpError::from_js("failed reading global state", &err))?;
    let (ws_k_req, ws_v_req) = read_all_store_pairs(&workspaces)?;
    let (nb_k_req, nb_v_req) = read_all_store_pairs(&notebooks)?;
    let (tr_k_req, tr_v_req) = read_all_store_pairs(&trash)?;
    let (bl_k_req, bl_v_req) = read_all_store_pairs(&blobs)?;

    // Attach every request's event handlers synchronously before yielding to `.await`.
    let global_p = idb_request_promise(&global_req);
    let ws_k_p = idb_request_promise(&ws_k_req);
    let ws_v_p = idb_request_promise(&ws_v_req);
    let nb_k_p = idb_request_promise(&nb_k_req);
    let nb_v_p = idb_request_promise(&nb_v_req);
    let tr_k_p = idb_request_promise(&tr_k_req);
    let tr_v_p = idb_request_promise(&tr_v_req);
    let bl_k_p = idb_request_promise(&bl_k_req);
    let bl_v_p = idb_request_promise(&bl_v_req);

    let global_val = resolve_idb_promise(global_p, "failed reading global state").await?;
    let ws_keys = resolve_idb_promise(ws_k_p, "failed reading workspace keys").await?;
    let ws_vals = resolve_idb_promise(ws_v_p, "failed reading workspace records").await?;
    let nb_keys = resolve_idb_promise(nb_k_p, "failed reading notebook keys").await?;
    let nb_vals = resolve_idb_promise(nb_v_p, "failed reading notebook records").await?;
    let tr_keys = resolve_idb_promise(tr_k_p, "failed reading trash keys").await?;
    let tr_vals = resolve_idb_promise(tr_v_p, "failed reading trash records").await?;
    let bl_keys = resolve_idb_promise(bl_k_p, "failed reading blob keys").await?;
    let bl_vals = resolve_idb_promise(bl_v_p, "failed reading blob payloads").await?;

    let Some(global_json) = global_val.as_string() else {
        return Ok((None, HashSet::new()));
    };
    let Ok(global) = serde_json::from_str::<GlobalSyncState>(&global_json) else {
        return Ok((None, HashSet::new()));
    };

    let mut blob_map: HashMap<String, Arc<Vec<u8>>> = HashMap::new();
    let mut known_blobs = HashSet::new();
    for (hash, bytes) in collect_store_byte_entries(&bl_keys, &bl_vals) {
        known_blobs.insert(hash.clone());
        blob_map.insert(hash, Arc::new(bytes));
    }

    let mut notebooks_by_ws: HashMap<String, HashMap<String, Document>> = HashMap::new();
    for (scoped, bytes) in collect_store_byte_entries(&nb_keys, &nb_vals) {
        let Some((ws_key, nb_id)) = scoped.split_once("::") else {
            continue;
        };
        if let Ok(doc) = decode_detached_notebook(&bytes, &blob_map) {
            notebooks_by_ws
                .entry(ws_key.to_string())
                .or_default()
                .insert(nb_id.to_string(), doc);
        }
    }

    let mut trash_by_ws: HashMap<String, HashMap<String, (i64, Document)>> = HashMap::new();
    for (scoped, bytes) in collect_store_byte_entries(&tr_keys, &tr_vals) {
        let Some((ws_key, tid)) = scoped.split_once("::") else {
            continue;
        };
        if bytes.len() < 8 {
            continue;
        }
        let mut ts_buf = [0u8; 8];
        ts_buf.copy_from_slice(&bytes[0..8]);
        let deleted_at_ms = i64::from_le_bytes(ts_buf);
        if let Ok(doc) = decode_detached_notebook(&bytes[8..], &blob_map) {
            trash_by_ws
                .entry(ws_key.to_string())
                .or_default()
                .insert(tid.to_string(), (deleted_at_ms, doc));
        }
    }

    let mut loaded_workspaces = Vec::new();
    for (ws_key, ws_json) in collect_store_str_entries(&ws_keys, &ws_vals) {
        let Ok(meta) = decode_detached_workspace(&ws_json, &blob_map) else {
            continue;
        };
        loaded_workspaces.push(WebStorageLoadedWorkspace {
            notebooks: notebooks_by_ws.remove(&ws_key).unwrap_or_default(),
            trash: trash_by_ws.remove(&ws_key).unwrap_or_default(),
            workspace_key: ws_key,
            meta,
        });
    }

    Ok((
        Some(WebStorageLoadedState {
            global,
            workspaces: loaded_workspaces,
        }),
        known_blobs,
    ))
}

#[derive(Default)]
struct GcBarrierState {
    ws_vals: Option<JsValue>,
    nb_vals: Option<JsValue>,
    tr_vals: Option<JsValue>,
    bl_keys: Option<JsValue>,
    completed_reads: u8,
    live_hashes: Option<HashSet<String>>,
    deleted_count: usize,
    closures: Vec<Closure<dyn FnMut(web_sys::Event)>>,
}

fn collect_live_blob_hashes(
    ws_vals: &JsValue,
    nb_vals: &JsValue,
    tr_vals: &JsValue,
) -> HashSet<String> {
    let mut live_hashes = HashSet::new();
    let ws_arr = Array::from(ws_vals);
    for idx in 0..ws_arr.length() {
        if let Some(json) = ws_arr.get(idx).as_string() {
            live_hashes.extend(extract_detached_workspace_blob_hashes(&json));
        }
    }
    let nb_arr = Array::from(nb_vals);
    for idx in 0..nb_arr.length() {
        if let Some(bytes) = js_value_to_bytes(&nb_arr.get(idx))
            && bytes.starts_with(DETACHED_DOC_MAGIC)
        {
            live_hashes.extend(extract_detached_notebook_blob_hashes(&bytes));
        }
    }
    let tr_arr = Array::from(tr_vals);
    for idx in 0..tr_arr.length() {
        if let Some(bytes) = js_value_to_bytes(&tr_arr.get(idx))
            && bytes.len() > 8
            && bytes[8..].starts_with(DETACHED_DOC_MAGIC)
        {
            live_hashes.extend(extract_detached_notebook_blob_hashes(&bytes[8..]));
        }
    }
    live_hashes
}

fn finalize_gc_deletes_in_active_tx(
    barrier: &mut GcBarrierState,
    blobs_store: &web_sys::IdbObjectStore,
    inner: &Rc<RefCell<WebStorageInner>>,
) {
    let (Some(ws_vals), Some(nb_vals), Some(tr_vals), Some(bl_keys)) = (
        barrier.ws_vals.as_ref(),
        barrier.nb_vals.as_ref(),
        barrier.tr_vals.as_ref(),
        barrier.bl_keys.as_ref(),
    ) else {
        return;
    };
    {
        let mut st = inner.borrow_mut();
        st.prune_stale_peers();
        if !st.peer_tabs.is_empty() || !st.pending_delta.is_empty() || st.failed_delta.is_some() {
            return;
        }
    }
    let live_hashes = collect_live_blob_hashes(ws_vals, nb_vals, tr_vals);
    let key_arr = Array::from(bl_keys);
    let mut deleted = 0usize;
    for idx in 0..key_arr.length() {
        if let Some(hash) = key_arr.get(idx).as_string()
            && !live_hashes.contains(&hash)
            && delete_key(blobs_store, &hash).is_ok()
        {
            deleted += 1;
        }
    }
    barrier.live_hashes = Some(live_hashes);
    barrier.deleted_count = deleted;
}

fn attach_gc_read_handler(
    req: &web_sys::IdbRequest,
    slot_idx: u8,
    barrier_rc: &Rc<RefCell<GcBarrierState>>,
    blobs_store: &web_sys::IdbObjectStore,
    inner: &Rc<RefCell<WebStorageInner>>,
) {
    let req_clone = req.clone();
    let barrier = Rc::clone(barrier_rc);
    let blobs = blobs_store.clone();
    let inner_ref = Rc::clone(inner);
    let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
        req_clone.set_onsuccess(None);
        if let Ok(val) = req_clone.result() {
            let mut guard = barrier.borrow_mut();
            match slot_idx {
                0 => guard.ws_vals = Some(val),
                1 => guard.nb_vals = Some(val),
                2 => guard.tr_vals = Some(val),
                3 => guard.bl_keys = Some(val),
                _ => {}
            }
            guard.completed_reads = guard.completed_reads.saturating_add(1);
            if guard.completed_reads == 4 {
                finalize_gc_deletes_in_active_tx(&mut guard, &blobs, &inner_ref);
            }
        }
    });
    req.set_onsuccess(Some(cb.as_ref().unchecked_ref()));
    barrier_rc.borrow_mut().closures.push(cb);
}

async fn execute_atomic_blob_gc(
    db: &web_sys::IdbDatabase,
    inner: &Rc<RefCell<WebStorageInner>>,
) -> Result<Option<(HashSet<String>, usize)>, IdbOpError> {
    let tx = open_tx(db, &GC_STORES, web_sys::IdbTransactionMode::Readwrite)?;
    let tx_promise = idb_transaction_promise(&tx);
    let workspaces = store_from_tx(&tx, STORE_WORKSPACES)?;
    let notebooks = store_from_tx(&tx, STORE_NOTEBOOKS)?;
    let trash = store_from_tx(&tx, STORE_TRASH)?;
    let blobs = store_from_tx(&tx, STORE_BLOBS)?;

    let ws_req = workspaces
        .get_all()
        .map_err(|err| IdbOpError::from_js("GC workspaces.getAll failed", &err))?;
    let nb_req = notebooks
        .get_all()
        .map_err(|err| IdbOpError::from_js("GC notebooks.getAll failed", &err))?;
    let tr_req = trash
        .get_all()
        .map_err(|err| IdbOpError::from_js("GC trash.getAll failed", &err))?;
    let bl_keys_req = blobs
        .get_all_keys()
        .map_err(|err| IdbOpError::from_js("GC blobs.getAllKeys failed", &err))?;

    let barrier = Rc::new(RefCell::new(GcBarrierState::default()));
    attach_gc_read_handler(&ws_req, 0, &barrier, &blobs, inner);
    attach_gc_read_handler(&nb_req, 1, &barrier, &blobs, inner);
    attach_gc_read_handler(&tr_req, 2, &barrier, &blobs, inner);
    attach_gc_read_handler(&bl_keys_req, 3, &barrier, &blobs, inner);

    let tx_res = resolve_idb_promise(tx_promise, "GC transaction failed").await;
    ws_req.set_onsuccess(None);
    nb_req.set_onsuccess(None);
    tr_req.set_onsuccess(None);
    bl_keys_req.set_onsuccess(None);

    let mut guard = barrier.borrow_mut();
    guard.closures.clear();
    tx_res?;
    Ok(guard
        .live_hashes
        .take()
        .map(|hashes| (hashes, guard.deleted_count)))
}

fn apply_delta_to_fallback_mirror(mirror: &mut FallbackMirror, delta: &WebStorageDelta) {
    if let Some(ref global) = delta.global {
        mirror.global = Some(global.clone());
    }
    for ws_key in &delta.delete_workspaces {
        mirror.workspaces.remove(ws_key);
    }
    for (ws_key, meta) in &delta.put_workspaces {
        let entry =
            mirror
                .workspaces
                .entry(ws_key.clone())
                .or_insert_with(|| WebStorageLoadedWorkspace {
                    workspace_key: ws_key.clone(),
                    meta: meta.clone(),
                    notebooks: HashMap::new(),
                    trash: HashMap::new(),
                });
        entry.meta = meta.clone();
    }
    for (ws_key, nb_id) in &delta.delete_notebooks {
        if let Some(ws) = mirror.workspaces.get_mut(ws_key) {
            ws.notebooks.remove(nb_id);
        }
    }
    for ((ws_key, nb_id), doc) in &delta.put_notebooks {
        let entry =
            mirror
                .workspaces
                .entry(ws_key.clone())
                .or_insert_with(|| WebStorageLoadedWorkspace {
                    workspace_key: ws_key.clone(),
                    meta: WebWorkspaceMetaSnapshot::default(),
                    notebooks: HashMap::new(),
                    trash: HashMap::new(),
                });
        entry.notebooks.insert(nb_id.clone(), (**doc).clone());
    }
    for (ws_key, tid) in &delta.delete_trash {
        if let Some(ws) = mirror.workspaces.get_mut(ws_key) {
            ws.trash.remove(tid);
        }
    }
    for ((ws_key, tid), (ts, doc)) in &delta.put_trash {
        let entry =
            mirror
                .workspaces
                .entry(ws_key.clone())
                .or_insert_with(|| WebStorageLoadedWorkspace {
                    workspace_key: ws_key.clone(),
                    meta: WebWorkspaceMetaSnapshot::default(),
                    notebooks: HashMap::new(),
                    trash: HashMap::new(),
                });
        entry.trash.insert(tid.clone(), (*ts, (**doc).clone()));
    }
}

#[derive(Serialize)]
struct LegacyFallbackBundle<'a> {
    notebooks: Vec<&'a Document>,
    active_notebook: usize,
    trash: Vec<LegacyFallbackTrash<'a>>,
    prefs: &'a papercraft_doc::UserStudioConfig,
    manifest: &'a papercraft_sync::SyncManifest,
    slot_ids: &'a [String],
    slot_dirty_hash: &'a [bool],
    slot_placeholder_blank: &'a [bool],
    prefs_dirty_pending: bool,
    last_dirty_at_ms: Option<i64>,
}

#[derive(Serialize)]
struct LegacyFallbackTrash<'a> {
    trash_id: &'a str,
    doc: &'a Document,
    deleted_at_ms: i64,
}

#[derive(Serialize)]
struct LegacyFallbackEnvelope<'a> {
    version: u32,
    global: &'a GlobalSyncState,
    local_profiles: HashMap<&'a str, LegacyFallbackBundle<'a>>,
    cloud_accounts: HashMap<&'a str, LegacyFallbackBundle<'a>>,
}

fn serialize_fallback_mirror_json(mirror: &FallbackMirror) -> Result<String, String> {
    let Some(ref global) = mirror.global else {
        return Err("missing global state for localStorage fallback".to_string());
    };
    let mut local_profiles = HashMap::new();
    let mut cloud_accounts = HashMap::new();
    for (ws_key, ws) in &mirror.workspaces {
        let mut notebooks = Vec::with_capacity(ws.meta.notebook_ids.len());
        for nb_id in &ws.meta.notebook_ids {
            if let Some(doc) = ws.notebooks.get(nb_id) {
                notebooks.push(doc);
            }
        }
        let mut trash = Vec::with_capacity(ws.meta.trash_entries.len());
        for (tid, deleted_at_ms) in &ws.meta.trash_entries {
            if let Some((ts, doc)) = ws.trash.get(tid) {
                trash.push(LegacyFallbackTrash {
                    trash_id: tid.as_str(),
                    doc,
                    deleted_at_ms: (*ts).max(*deleted_at_ms),
                });
            }
        }
        let bundle = LegacyFallbackBundle {
            notebooks,
            active_notebook: ws.meta.active_notebook,
            trash,
            prefs: &ws.meta.prefs,
            manifest: &ws.meta.manifest,
            slot_ids: &ws.meta.slot_ids,
            slot_dirty_hash: &ws.meta.slot_dirty_hash,
            slot_placeholder_blank: &ws.meta.slot_placeholder_blank,
            prefs_dirty_pending: ws.meta.prefs_dirty_pending,
            last_dirty_at_ms: ws.meta.last_dirty_at_ms,
        };
        if let Some(pid) = ws_key.strip_prefix("local:") {
            local_profiles.insert(pid, bundle);
        } else if let Some(acct_key) = ws_key.strip_prefix("cloud:") {
            cloud_accounts.insert(acct_key, bundle);
        }
    }
    serde_json::to_string(&LegacyFallbackEnvelope {
        version: 4,
        global,
        local_profiles,
        cloud_accounts,
    })
    .map_err(|e| format!("fallback JSON serialization failed: {e}"))
}
