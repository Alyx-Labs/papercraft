use std::collections::{HashMap, HashSet};
use std::ops::Range;

use papercraft_doc::{Document, NotebookPage};
use papercraft_sync_protocol::{
    AcceptedFolderSync, ConflictNotebookSync, LocalNotebookChange, LocalPrefsPush,
    MAX_PUSH_BATCH_SIZE, RemoteFolderRecord, RemoteNotebookRecord, RemotePrefsRecord,
    SyncPullResponse, SyncPushRequest, SyncPushResponse, UserProfile,
    canonical_document_all_hashes, canonical_document_hash, canonical_document_page_hashes,
    now_unix_ms,
};

use crate::client::SyncHttpClient;
use crate::merge::{
    DocumentMergeBase, MergeDecision, PageElementFingerprints, compute_document_page_elements,
    merge_documents_three_way,
};
use crate::types::{
    ActiveAccountSession, NotebookTombstone, RemoteNotebookUpdate, SyncBatch, SyncCycleOutcome,
    SyncError,
};

const TARGET_PUSH_CHUNK_BYTES: usize = 24 * 1024 * 1024;
const MAX_CONFLICT_RETRIES: u8 = 3;

#[derive(Clone)]
struct WorkingPushItem {
    doc: Document,
    base_version: u64,
    content_hash: String,
    merge_base: DocumentMergeBase,
    pre_rebase_ancestor: Option<DocumentMergeBase>,
    was_rebased: bool,
    synthesized_timestamps: bool,
    sent_page_hashes: Vec<String>,
    sent_page_elements: Vec<PageElementFingerprints>,
}

/// Pure step-by-step sync state machine shared by blocking desktop HTTP and async web `fetch`.
pub struct SyncCycleDriver {
    working_items: HashMap<String, WorkingPushItem>,
    item_order: Vec<String>,
    pending_tombstones: Vec<NotebookTombstone>,
    working_prefs_push: Option<LocalPrefsPush>,
    changes: Vec<LocalNotebookChange>,
    ranges: Vec<Range<usize>>,
    cursor: usize,
    prefs_only_pending: bool,
    sent_initial_meta: bool,
    retry_queue: Vec<String>,
    retry_round: u8,
    current_cursor: u64,
    had_push_requests: bool,
    pre_pull_user: Option<UserProfile>,
    pre_pull_folders: Vec<RemoteFolderRecord>,
    pre_pull_prefs: Option<RemotePrefsRecord>,
    accum: PushAccum,
}

impl SyncCycleDriver {
    /// Prepares chunked push requests from a captured [`SyncBatch`].
    pub fn new(batch: &SyncBatch) -> Self {
        let now = now_unix_ms();
        let mut working_items = HashMap::new();
        let mut item_order = Vec::new();
        for snap in &batch.local_docs {
            if !batch.force_overwrite && !snap.cloud_dirty && snap.base_version > 0 {
                continue;
            }
            let mut doc = snap.doc.clone();
            let synthesized_timestamps = doc.created_at_ms == 0 || doc.updated_at_ms == 0;
            normalize_push_doc(&mut doc, &snap.notebook_id, now);
            let hashes = canonical_document_all_hashes(&doc);
            let content_hash = if snap.content_hash.is_empty() {
                hashes.content_hash
            } else {
                snap.content_hash.clone()
            };
            let elements = compute_document_page_elements(&doc);
            item_order.push(snap.notebook_id.clone());
            working_items.insert(
                snap.notebook_id.clone(),
                WorkingPushItem {
                    doc,
                    base_version: snap.base_version,
                    content_hash,
                    merge_base: snap.merge_base.clone(),
                    pre_rebase_ancestor: None,
                    was_rebased: false,
                    synthesized_timestamps,
                    sent_page_hashes: hashes.page_hashes,
                    sent_page_elements: elements,
                },
            );
        }

        let mut driver = Self {
            working_items,
            item_order,
            pending_tombstones: batch.tombstones.clone(),
            working_prefs_push: batch.prefs_push.clone(),
            changes: Vec::new(),
            ranges: Vec::new(),
            cursor: 0,
            prefs_only_pending: false,
            sent_initial_meta: false,
            retry_queue: Vec::new(),
            retry_round: 0,
            current_cursor: batch.client_cursor,
            had_push_requests: false,
            pre_pull_user: None,
            pre_pull_folders: Vec::new(),
            pre_pull_prefs: None,
            accum: PushAccum::default(),
        };
        driver.rebuild_push_plan(batch, true);
        driver
    }

    /// Cursor to pass to `GET /api/v1/sync/pull?since=...` after pre-pull/pushes.
    #[must_use]
    pub fn pull_since_cursor(&self) -> u64 {
        self.current_cursor
    }

    /// Returns `true` when at least one `SyncPushRequest` was produced and processed.
    #[must_use]
    pub fn had_push_requests(&self) -> bool {
        self.had_push_requests
    }

    /// Returns `true` when another HTTP conflict pull-and-retry round is permitted.
    #[must_use]
    pub fn can_retry_http_conflict(&self) -> bool {
        self.retry_round < MAX_CONFLICT_RETRIES && !self.working_items.is_empty()
    }

    /// Captures snapshots of unacknowledged in-flight pushes so a dropped HTTP ACK can still
    /// recognize and rebase cleanly onto our own server record on the next sync attempt.
    #[must_use]
    pub fn attempted_push_bases(&self) -> HashMap<String, DocumentMergeBase> {
        if !self.had_push_requests {
            return HashMap::new();
        }
        self.working_items
            .iter()
            .map(|(id, item)| {
                (
                    id.clone(),
                    DocumentMergeBase {
                        pending_push_hash: Some(item.content_hash.clone()),
                        pending_push_page_hashes: item.sent_page_hashes.clone(),
                        pending_push_page_elements: item.sent_page_elements.clone(),
                        ..Default::default()
                    },
                )
            })
            .collect()
    }

    /// Reconciles an initial pull response against dirty local snapshots BEFORE pushing,
    /// automatically fast-forwarding identical records and 3-way merging diverged edits.
    pub fn apply_pre_pull(&mut self, pull_resp: SyncPullResponse, batch: &SyncBatch) {
        self.current_cursor = self.current_cursor.max(pull_resp.server_cursor);
        self.pre_pull_user = Some(pull_resp.user.clone());
        self.pre_pull_folders = pull_resp.folders;
        self.pre_pull_prefs = pull_resp.studio_prefs.clone();

        if let Some(ref remote_prefs) = pull_resp.studio_prefs
            && let Some(ref local_prefs) = self.working_prefs_push
            && remote_prefs.content_hash == local_prefs.content_hash
        {
            self.accum.ack_prefs = Some((remote_prefs.version, remote_prefs.content_hash.clone()));
            self.working_prefs_push = None;
        }

        let tombstone_ids: HashSet<String> = self
            .pending_tombstones
            .iter()
            .map(|t| t.notebook_id.clone())
            .collect();

        for rec in pull_resp.notebooks {
            if self.working_items.contains_key(&rec.notebook_id) {
                self.reconcile_pulled_record_against_working(&rec);
            } else if !tombstone_ids.contains(&rec.notebook_id) {
                upsert_downloaded_update(&rec, &mut self.accum.downloaded_updates);
            }
        }

        self.rebuild_push_plan(batch, !self.sent_initial_meta);
    }

    /// Handles a server HTTP `409` on push by pulling the latest server state, rebasing remaining
    /// dirty items, and resetting the push plan for retry.
    pub fn apply_http_conflict_pull(
        &mut self,
        pull_resp: SyncPullResponse,
        batch: &SyncBatch,
    ) -> bool {
        if !self.can_retry_http_conflict() {
            return false;
        }
        self.retry_round = self.retry_round.saturating_add(1);
        self.apply_pre_pull(pull_resp, batch);
        !self.changes.is_empty() || self.prefs_only_pending
    }

    fn reconcile_pulled_record_against_working(&mut self, rec: &RemoteNotebookRecord) {
        let Some(mut item) = self.working_items.remove(&rec.notebook_id) else {
            return;
        };
        if rec.deleted {
            upsert_downloaded_update(rec, &mut self.accum.conflict_updates);
            return;
        }
        let Some(ref server_doc) = rec.document else {
            self.working_items.insert(rec.notebook_id.clone(), item);
            return;
        };
        match merge_documents_three_way(&item.doc, server_doc, &item.merge_base) {
            MergeDecision::FastForward(reconciled) => {
                let timestamps_ahead = !item.synthesized_timestamps
                    && (reconciled.updated_at_ms > server_doc.updated_at_ms
                        || reconciled.meta_updated_at_ms > server_doc.meta_updated_at_ms
                        || (server_doc.created_at_ms == 0 && reconciled.created_at_ms > 0));
                if timestamps_ahead {
                    record_pre_rebase_ancestor(&mut item);
                    item.doc = reconciled;
                    item.base_version = rec.version;
                    item.was_rebased = true;
                    update_item_merge_base_from_server(&mut item, server_doc, &rec.content_hash);
                    refresh_item_push_hashes(&mut item);
                    self.working_items.insert(rec.notebook_id.clone(), item);
                } else {
                    upsert_downloaded_update(rec, &mut self.accum.downloaded_updates);
                }
            }
            MergeDecision::Merged { doc, .. } => {
                record_pre_rebase_ancestor(&mut item);
                item.doc = doc;
                item.base_version = rec.version;
                item.was_rebased = true;
                update_item_merge_base_from_server(&mut item, server_doc, &rec.content_hash);
                refresh_item_push_hashes(&mut item);
                self.working_items.insert(rec.notebook_id.clone(), item);
            }
            MergeDecision::Conflict => {
                upsert_downloaded_update(rec, &mut self.accum.conflict_updates);
            }
        }
    }

    fn rebuild_push_plan(&mut self, batch: &SyncBatch, include_meta: bool) {
        let now = now_unix_ms();
        let mut changes = Vec::new();
        for id in &self.item_order {
            if let Some(item) = self.working_items.get(id) {
                changes.push(build_local_notebook_change(id, item, now));
            }
        }
        for tomb in &self.pending_tombstones {
            changes.push(LocalNotebookChange {
                notebook_id: tomb.notebook_id.clone(),
                base_version: tomb.base_version,
                deleted: true,
                updated_at_ms: tomb.deleted_at_ms,
                deleted_at_ms: Some(tomb.deleted_at_ms),
                content_hash: "deleted".to_string(),
                body_hash: String::new(),
                meta_hash: String::new(),
                meta_updated_at_ms: tomb.deleted_at_ms,
                document: None,
            });
        }
        self.changes = changes;
        self.ranges =
            plan_push_chunk_ranges(&self.changes, MAX_PUSH_BATCH_SIZE, TARGET_PUSH_CHUNK_BYTES);
        self.cursor = 0;
        self.prefs_only_pending = include_meta
            && self.changes.is_empty()
            && (self.working_prefs_push.is_some() || !batch.folder_changes.is_empty());
    }

    fn rebuild_retry_push_plan(&mut self) {
        let now = now_unix_ms();
        let retry_ids = std::mem::take(&mut self.retry_queue);
        let mut seen = HashSet::new();
        let mut changes = Vec::new();
        for id in retry_ids {
            if !seen.insert(id.clone()) {
                continue;
            }
            if let Some(item) = self.working_items.get(&id) {
                changes.push(build_local_notebook_change(&id, item, now));
            }
        }
        self.changes = changes;
        self.ranges =
            plan_push_chunk_ranges(&self.changes, MAX_PUSH_BATCH_SIZE, TARGET_PUSH_CHUNK_BYTES);
        self.cursor = 0;
        self.prefs_only_pending = false;
    }

    /// Returns the next chunked push request, or `None` once all push requests have completed.
    pub fn next_push_request(&self, batch: &SyncBatch) -> Option<SyncPushRequest> {
        if self.prefs_only_pending {
            return Some(SyncPushRequest {
                client_cursor: self.current_cursor,
                device_name: batch.device_name.clone(),
                changes: Vec::new(),
                folders: batch.folder_changes.clone(),
                studio_prefs: self.working_prefs_push.clone(),
                force_overwrite: batch.force_overwrite,
            });
        }
        let range = self.ranges.get(self.cursor)?.clone();
        let include_meta = !self.sent_initial_meta && self.cursor == 0;
        let prefs_for_chunk = if include_meta {
            self.working_prefs_push.clone()
        } else {
            None
        };
        let folders_for_chunk = if include_meta {
            batch.folder_changes.clone()
        } else {
            Vec::new()
        };
        Some(SyncPushRequest {
            client_cursor: self.current_cursor,
            device_name: batch.device_name.clone(),
            changes: self.changes[range].to_vec(),
            folders: folders_for_chunk,
            studio_prefs: prefs_for_chunk,
            force_overwrite: batch.force_overwrite,
        })
    }

    /// Records the server's push response, automatically queuing any conflict-merged notebooks for
    /// seamless retry before advancing to the next chunk.
    pub fn record_push_chunk_response(&mut self, resp: SyncPushResponse, batch: &SyncBatch) {
        self.had_push_requests = true;
        self.current_cursor = self.current_cursor.max(resp.server_cursor);
        let include_meta = self.prefs_only_pending || (!self.sent_initial_meta && self.cursor == 0);
        if include_meta {
            self.sent_initial_meta = true;
        }
        let sent_prefs = if include_meta {
            self.working_prefs_push.as_ref()
        } else {
            None
        };

        self.accum.highest_cursor = self.accum.highest_cursor.max(resp.server_cursor);
        record_prefs_and_folders(&resp, sent_prefs, &mut self.accum);

        for acc in resp.accepted {
            self.accum.pushed_count += 1;
            if acc.deleted {
                self.pending_tombstones
                    .retain(|t| t.notebook_id != acc.notebook_id);
                self.accum.cleared_tombstones.push(acc.notebook_id);
            } else {
                if let Some(item) = self.working_items.remove(&acc.notebook_id) {
                    self.accum
                        .accepted_page_hashes
                        .insert(acc.notebook_id.clone(), item.sent_page_hashes);
                    self.accum
                        .accepted_page_elements
                        .insert(acc.notebook_id.clone(), item.sent_page_elements);
                    if item.was_rebased {
                        if let Some(ancestor) = item.pre_rebase_ancestor {
                            self.accum
                                .rebased_ancestors
                                .insert(acc.notebook_id.clone(), ancestor);
                        }
                        self.accum
                            .rebased_docs
                            .insert(acc.notebook_id.clone(), item.doc);
                    }
                }
                self.accum.accepted_uploads.push(acc);
            }
        }

        for conflict in resp.conflicts {
            self.resolve_or_queue_retry(conflict, batch.force_overwrite);
        }

        if self.prefs_only_pending {
            self.prefs_only_pending = false;
        } else if self.cursor < self.ranges.len() {
            self.cursor += 1;
        }

        if !self.prefs_only_pending
            && self.cursor >= self.ranges.len()
            && !self.retry_queue.is_empty()
            && self.retry_round < MAX_CONFLICT_RETRIES
        {
            self.retry_round = self.retry_round.saturating_add(1);
            self.rebuild_retry_push_plan();
        }
    }

    fn resolve_or_queue_retry(&mut self, conflict: ConflictNotebookSync, force_overwrite: bool) {
        if !force_overwrite
            && self.retry_round < MAX_CONFLICT_RETRIES
            && !conflict.server_record.deleted
            && let Some(ref server_doc) = conflict.server_record.document
            && let Some(mut item) = self.working_items.remove(&conflict.notebook_id)
        {
            match merge_documents_three_way(&item.doc, server_doc, &item.merge_base) {
                MergeDecision::FastForward(_) => {
                    self.accum
                        .cleared_tombstones
                        .push(conflict.notebook_id.clone());
                    upsert_downloaded_update(
                        &conflict.server_record,
                        &mut self.accum.downloaded_updates,
                    );
                    return;
                }
                MergeDecision::Merged { doc, .. } => {
                    record_pre_rebase_ancestor(&mut item);
                    item.doc = doc;
                    item.base_version = conflict.server_record.version;
                    item.was_rebased = true;
                    update_item_merge_base_from_server(
                        &mut item,
                        server_doc,
                        &conflict.server_record.content_hash,
                    );
                    refresh_item_push_hashes(&mut item);
                    self.working_items
                        .insert(conflict.notebook_id.clone(), item);
                    self.retry_queue.push(conflict.notebook_id);
                    return;
                }
                MergeDecision::Conflict => {}
            }
        }
        self.pending_tombstones
            .retain(|t| t.notebook_id != conflict.notebook_id);
        handle_conflict(&conflict, &mut self.accum);
    }

    /// Finishes the cycle directly from the stored `pre_pull` response when no push was required.
    pub fn finish_from_pre_pull(
        mut self,
        refreshed_session: Option<ActiveAccountSession>,
    ) -> SyncCycleOutcome {
        let user = self.pre_pull_user.take().unwrap_or(UserProfile {
            user_id: String::new(),
            email: None,
            username: String::new(),
            display_name: String::new(),
            created_at_ms: 0,
            must_change_password: false,
        });
        let pull_resp = SyncPullResponse {
            server_cursor: self.current_cursor,
            user,
            notebooks: Vec::new(),
            folders: std::mem::take(&mut self.pre_pull_folders),
            studio_prefs: self.pre_pull_prefs.take(),
        };
        self.finish(pull_resp, refreshed_session)
    }

    /// Merges the final pull payload with recorded push acknowledgements into [`SyncCycleOutcome`].
    pub fn finish(
        mut self,
        pull_resp: SyncPullResponse,
        refreshed_session: Option<ActiveAccountSession>,
    ) -> SyncCycleOutcome {
        merge_pulled_records(
            &pull_resp.notebooks,
            &self.accum.accepted_uploads,
            &self.accum.cleared_tombstones,
            &self.accum.conflict_updates,
            &mut self.accum.downloaded_updates,
        );
        let downloaded_folders = merge_pulled_folders(
            &self.pre_pull_folders,
            pull_resp.folders,
            &self.accum.accepted_folders,
        );
        let merged_pull_prefs = match (self.pre_pull_prefs.take(), pull_resp.studio_prefs) {
            (Some(a), Some(b)) => Some(if b.version >= a.version { b } else { a }),
            (None, Some(b)) => Some(b),
            (Some(a), None) => Some(a),
            (None, None) => None,
        };
        let updated_prefs = select_pulled_prefs(
            merged_pull_prefs,
            self.accum.prefs_conflict,
            self.accum.ack_prefs.as_ref(),
        );
        let new_cursor = pull_resp.server_cursor.max(self.accum.highest_cursor);

        SyncCycleOutcome {
            new_cursor,
            refreshed_session,
            server_user: pull_resp.user,
            accepted_uploads: self.accum.accepted_uploads,
            accepted_page_hashes: self.accum.accepted_page_hashes,
            accepted_page_elements: self.accum.accepted_page_elements,
            rebased_docs: self.accum.rebased_docs,
            rebased_ancestors: self.accum.rebased_ancestors,
            accepted_folders: self.accum.accepted_folders,
            cleared_tombstones: self.accum.cleared_tombstones,
            pulled_count: self
                .accum
                .downloaded_updates
                .len()
                .saturating_add(self.accum.conflict_updates.len()),
            downloaded_updates: self.accum.downloaded_updates,
            conflict_updates: self.accum.conflict_updates,
            downloaded_folders,
            updated_prefs,
            ack_prefs: self.accum.ack_prefs,
            pushed_count: self.accum.pushed_count,
            synced_at_ms: now_unix_ms(),
        }
    }
}

pub fn run_sync_cycle(
    client: &SyncHttpClient,
    session: &mut ActiveAccountSession,
    batch: &SyncBatch,
) -> Result<SyncCycleOutcome, SyncError> {
    let initial_refresh = session.access_token.clone();
    let mut driver = SyncCycleDriver::new(batch);
    let mut did_pre_pull = false;

    if !batch.force_overwrite {
        let (pre_pull, _) = client.pull_with_refresh(session, batch.client_cursor)?;
        driver.apply_pre_pull(pre_pull, batch);
        did_pre_pull = true;
    }

    while let Some(req) = driver.next_push_request(batch) {
        match client.push_with_refresh(session, &req) {
            Ok((resp, _)) => driver.record_push_chunk_response(resp, batch),
            Err(SyncError::Server { status: 409, .. }) if driver.can_retry_http_conflict() => {
                let (pull_resp, _) =
                    client.pull_with_refresh(session, driver.pull_since_cursor())?;
                if !driver.apply_http_conflict_pull(pull_resp, batch) {
                    break;
                }
            }
            Err(err) => return Err(err),
        }
    }

    if driver.had_push_requests() || !did_pre_pull {
        let (pull_resp, refreshed_on_pull) =
            client.pull_with_refresh(session, driver.pull_since_cursor())?;
        let token_changed = refreshed_on_pull || session.access_token != initial_refresh;
        Ok(driver.finish(pull_resp, token_changed.then(|| session.clone())))
    } else {
        let token_changed = session.access_token != initial_refresh;
        Ok(driver.finish_from_pre_pull(token_changed.then(|| session.clone())))
    }
}

/// Converts a full `since=0` pull response into a [`SyncCycleOutcome`] for live server session hydration.
#[must_use]
pub fn outcome_from_full_pull(
    pull_resp: SyncPullResponse,
    refreshed_session: Option<ActiveAccountSession>,
) -> SyncCycleOutcome {
    let downloaded_updates: Vec<RemoteNotebookUpdate> = pull_resp
        .notebooks
        .into_iter()
        .map(|record| {
            let page_hashes = record
                .document
                .as_ref()
                .map(canonical_document_page_hashes)
                .unwrap_or_default();
            RemoteNotebookUpdate {
                notebook_id: record.notebook_id,
                version: record.version,
                cursor: record.cursor,
                deleted: record.deleted,
                updated_at_ms: record.updated_at_ms,
                deleted_at_ms: record.deleted_at_ms,
                content_hash: record.content_hash,
                body_hash: record.body_hash,
                meta_hash: record.meta_hash,
                meta_updated_at_ms: record.meta_updated_at_ms,
                document: record.document,
                page_hashes,
            }
        })
        .collect();
    SyncCycleOutcome {
        new_cursor: pull_resp.server_cursor,
        refreshed_session,
        server_user: pull_resp.user,
        accepted_uploads: Vec::new(),
        accepted_page_hashes: HashMap::new(),
        accepted_page_elements: HashMap::new(),
        rebased_docs: HashMap::new(),
        rebased_ancestors: HashMap::new(),
        accepted_folders: Vec::new(),
        cleared_tombstones: Vec::new(),
        pulled_count: downloaded_updates.len(),
        downloaded_updates,
        conflict_updates: Vec::new(),
        downloaded_folders: pull_resp.folders,
        updated_prefs: pull_resp.studio_prefs,
        ack_prefs: None,
        pushed_count: 0,
        synced_at_ms: now_unix_ms(),
    }
}

#[derive(Default)]
struct PushAccum {
    accepted_uploads: Vec<papercraft_sync_protocol::AcceptedNotebookSync>,
    accepted_page_hashes: HashMap<String, Vec<String>>,
    accepted_page_elements: HashMap<String, Vec<PageElementFingerprints>>,
    rebased_docs: HashMap<String, Document>,
    rebased_ancestors: HashMap<String, DocumentMergeBase>,
    accepted_folders: Vec<AcceptedFolderSync>,
    cleared_tombstones: Vec<String>,
    downloaded_updates: Vec<RemoteNotebookUpdate>,
    conflict_updates: Vec<RemoteNotebookUpdate>,
    ack_prefs: Option<(u64, String)>,
    prefs_conflict: Option<RemotePrefsRecord>,
    highest_cursor: u64,
    pushed_count: usize,
}

fn normalize_push_doc(doc: &mut Document, notebook_id: &str, now: i64) {
    if doc.id.is_empty() {
        doc.id = notebook_id.to_string();
    }
    if doc.created_at_ms == 0 {
        doc.created_at_ms = now;
    }
    if doc.updated_at_ms == 0 {
        doc.updated_at_ms = doc.created_at_ms.max(now);
    }
    if doc.meta_updated_at_ms == 0 {
        doc.meta_updated_at_ms = doc.updated_at_ms;
    }
    doc.sync_active_to_pages();
    doc.load_active_from_pages();
}

fn record_pre_rebase_ancestor(item: &mut WorkingPushItem) {
    if item.pre_rebase_ancestor.is_none() {
        item.pre_rebase_ancestor = Some(DocumentMergeBase {
            page_hashes: item.sent_page_hashes.clone(),
            page_elements: item.sent_page_elements.clone(),
            ..Default::default()
        });
    }
}

fn update_item_merge_base_from_server(
    item: &mut WorkingPushItem,
    server_doc: &Document,
    server_hash: &str,
) {
    item.merge_base.page_hashes = canonical_document_page_hashes(server_doc);
    item.merge_base.page_elements = compute_document_page_elements(server_doc);
    item.merge_base.pending_push_hash = Some(if server_hash.is_empty() {
        canonical_document_hash(server_doc)
    } else {
        server_hash.to_string()
    });
    item.merge_base.pending_push_page_hashes = item.merge_base.page_hashes.clone();
    item.merge_base.pending_push_page_elements = item.merge_base.page_elements.clone();
}

fn refresh_item_push_hashes(item: &mut WorkingPushItem) {
    item.doc.sync_active_to_pages();
    item.doc.load_active_from_pages();
    let hashes = canonical_document_all_hashes(&item.doc);
    item.content_hash = hashes.content_hash;
    item.sent_page_hashes = hashes.page_hashes;
    item.sent_page_elements = compute_document_page_elements(&item.doc);
}

fn build_local_notebook_change(
    notebook_id: &str,
    item: &WorkingPushItem,
    now: i64,
) -> LocalNotebookChange {
    let hashes = canonical_document_all_hashes(&item.doc);
    let content_hash = if item.content_hash.is_empty() {
        hashes.content_hash
    } else {
        item.content_hash.clone()
    };
    let updated_at_ms = item.doc.updated_at_ms.max(now);
    let meta_updated_at_ms = item.doc.meta_updated_at_ms.max(updated_at_ms);
    LocalNotebookChange {
        notebook_id: notebook_id.to_string(),
        base_version: item.base_version,
        deleted: false,
        updated_at_ms,
        deleted_at_ms: None,
        content_hash,
        body_hash: hashes.body_hash,
        meta_hash: hashes.meta_hash,
        meta_updated_at_ms,
        document: Some(item.doc.clone()),
    }
}

fn estimate_page_json_bytes(page: &NotebookPage) -> usize {
    let mut total = 160usize;
    for stroke in &page.strokes {
        total = total.saturating_add(160usize.saturating_add(stroke.smoothed_points.len() * 48));
    }
    for text in &page.texts {
        total = total.saturating_add(160usize.saturating_add(text.text.len()));
    }
    for shape in &page.shapes {
        total = total.saturating_add(128usize.saturating_add(shape.points.len() * 24));
    }
    for img in &page.images {
        total = total.saturating_add(160usize.saturating_add((img.png_bytes.len() * 4) / 3));
    }
    total
}

fn estimate_change_bytes(change: &LocalNotebookChange) -> usize {
    let mut total = 256usize
        .saturating_add(change.notebook_id.len())
        .saturating_add(change.content_hash.len())
        .saturating_add(change.body_hash.len())
        .saturating_add(change.meta_hash.len());
    let Some(ref doc) = change.document else {
        return total.max(1);
    };
    total = total
        .saturating_add(512)
        .saturating_add(doc.title.len())
        .saturating_add(doc.folder_path.len())
        .saturating_add(doc.id.len());
    for tag in &doc.tags {
        total = total.saturating_add(tag.len().saturating_add(4));
    }
    for pdf in &doc.pdf_documents {
        total = total
            .saturating_add(64)
            .saturating_add(pdf.name.len())
            .saturating_add((pdf.bytes.len() * 4) / 3);
    }
    for tpl in &doc.image_templates {
        total = total
            .saturating_add(64)
            .saturating_add(tpl.name.len())
            .saturating_add((tpl.png_bytes.len() * 4) / 3);
    }
    for idx in 0..doc.page_count() {
        if let Some(page_bytes) = doc.with_resolved_page(idx, estimate_page_json_bytes) {
            total = total.saturating_add(page_bytes);
        }
    }
    total.max(1)
}

fn plan_push_chunk_ranges(
    changes: &[LocalNotebookChange],
    max_count: usize,
    max_bytes: usize,
) -> Vec<Range<usize>> {
    if changes.is_empty() {
        return Vec::new();
    }
    let count_cap = max_count.max(1);
    let byte_cap = max_bytes.max(1);
    let mut ranges = Vec::new();
    let mut start = 0;

    while start < changes.len() {
        let mut end = start;
        let mut chunk_bytes = 0usize;
        while end < changes.len() && (end - start) < count_cap {
            let item_bytes = estimate_change_bytes(&changes[end]);
            if end > start && chunk_bytes.saturating_add(item_bytes) > byte_cap {
                break;
            }
            chunk_bytes = chunk_bytes.saturating_add(item_bytes);
            end += 1;
        }
        ranges.push(start..end);
        start = end;
    }

    ranges
}

fn record_prefs_and_folders(
    resp: &SyncPushResponse,
    sent_prefs: Option<&LocalPrefsPush>,
    accum: &mut PushAccum,
) {
    if let Some(ref conflict_prefs) = resp.prefs_conflict {
        let should_take = accum
            .prefs_conflict
            .as_ref()
            .is_none_or(|cur| conflict_prefs.version >= cur.version);
        if should_take {
            accum.prefs_conflict = Some(conflict_prefs.clone());
        }
    } else if let Some(prefs) = sent_prefs {
        accum.ack_prefs = Some((resp.prefs_version, prefs.content_hash.clone()));
    }
    accum
        .accepted_folders
        .extend(resp.accepted_folders.iter().cloned());
}

fn handle_conflict(conflict: &ConflictNotebookSync, accum: &mut PushAccum) {
    // Queue unmerged server record into `conflict_updates` so the session engine preserves
    // both versions via a single non-cascading Conflict Copy.
    accum.cleared_tombstones.push(conflict.notebook_id.clone());
    upsert_downloaded_update(&conflict.server_record, &mut accum.conflict_updates);
}

fn merge_pulled_folders(
    pre_pull_folders: &[RemoteFolderRecord],
    final_folders: Vec<RemoteFolderRecord>,
    accepted_folders: &[AcceptedFolderSync],
) -> Vec<RemoteFolderRecord> {
    let mut by_id: HashMap<String, RemoteFolderRecord> = HashMap::new();
    for rf in pre_pull_folders.iter().cloned().chain(final_folders) {
        if accepted_folders
            .iter()
            .any(|af| af.folder_id == rf.folder_id && af.new_version == rf.version)
        {
            continue;
        }
        match by_id.get(&rf.folder_id) {
            Some(existing) if existing.version > rf.version => {}
            _ => {
                by_id.insert(rf.folder_id.clone(), rf);
            }
        }
    }
    let mut out: Vec<RemoteFolderRecord> = by_id.into_values().collect();
    out.sort_by_key(|f| f.cursor);
    out
}

fn merge_pulled_records(
    records: &[RemoteNotebookRecord],
    accepted: &[papercraft_sync_protocol::AcceptedNotebookSync],
    cleared_tombstones: &[String],
    conflict_updates: &[RemoteNotebookUpdate],
    target: &mut Vec<RemoteNotebookUpdate>,
) {
    for record in records {
        let is_own_upload = !record.deleted
            && accepted.iter().any(|acc| {
                acc.notebook_id == record.notebook_id && acc.new_version == record.version
            });
        let is_own_delete = record.deleted
            && cleared_tombstones
                .iter()
                .any(|id| id == &record.notebook_id);
        let already_in_conflicts = conflict_updates
            .iter()
            .any(|c| c.notebook_id == record.notebook_id && c.version >= record.version);
        if is_own_upload || is_own_delete || already_in_conflicts {
            continue;
        }
        upsert_downloaded_update(record, target);
    }
}

fn upsert_downloaded_update(record: &RemoteNotebookRecord, target: &mut Vec<RemoteNotebookUpdate>) {
    let page_hashes = record
        .document
        .as_ref()
        .map(canonical_document_page_hashes)
        .unwrap_or_default();
    let candidate = RemoteNotebookUpdate {
        notebook_id: record.notebook_id.clone(),
        version: record.version,
        cursor: record.cursor,
        deleted: record.deleted,
        updated_at_ms: record.updated_at_ms,
        deleted_at_ms: record.deleted_at_ms,
        content_hash: record.content_hash.clone(),
        body_hash: record.body_hash.clone(),
        meta_hash: record.meta_hash.clone(),
        meta_updated_at_ms: record.meta_updated_at_ms,
        document: record.document.clone(),
        page_hashes,
    };

    if let Some(existing) = target
        .iter_mut()
        .find(|u| u.notebook_id == candidate.notebook_id)
    {
        if candidate.version >= existing.version {
            *existing = candidate;
        }
        return;
    }
    target.push(candidate);
}

fn select_pulled_prefs(
    pulled: Option<RemotePrefsRecord>,
    conflict: Option<RemotePrefsRecord>,
    ack_prefs: Option<&(u64, String)>,
) -> Option<RemotePrefsRecord> {
    let best = match (pulled, conflict) {
        (Some(p), Some(c)) => Some(if c.version > p.version { c } else { p }),
        (Some(p), None) => Some(p),
        (None, Some(c)) => Some(c),
        (None, None) => None,
    }?;
    if let Some((acked_ver, acked_hash)) = ack_prefs
        && best.version <= *acked_ver
        && best.content_hash == *acked_hash
    {
        return None;
    }
    Some(best)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use papercraft_doc::{Document, InkStroke, PenSettings, StrokeId, StrokeSample};
    use papercraft_sync_protocol::{
        AcceptedNotebookSync, canonical_document_body_hash, canonical_document_hash,
        canonical_document_meta_hash,
    };

    use crate::types::LocalNotebookSnapshot;

    fn make_stroke(id: u64, x1: f64, y1: f64, x2: f64, y2: f64) -> Arc<InkStroke> {
        let pts = vec![
            StrokeSample::from_xy_pressure(x1, y1, 0.5),
            StrokeSample::from_xy_pressure(x2, y2, 0.7),
        ];
        Arc::new(InkStroke::new(
            StrokeId(id),
            PenSettings::fountain_pen(),
            pts.clone(),
            pts,
        ))
    }

    fn remote_record_for(doc: &Document, version: u64, cursor: u64) -> RemoteNotebookRecord {
        RemoteNotebookRecord {
            notebook_id: doc.id.clone(),
            version,
            cursor,
            deleted: false,
            updated_at_ms: doc.updated_at_ms.max(1000),
            deleted_at_ms: None,
            title: doc.title.clone(),
            content_hash: canonical_document_hash(doc),
            body_hash: canonical_document_body_hash(doc),
            meta_hash: canonical_document_meta_hash(doc),
            meta_updated_at_ms: doc.meta_updated_at_ms.max(1000),
            document: Some(doc.clone()),
        }
    }

    #[test]
    fn plan_push_chunk_ranges_respects_byte_budget_while_keeping_single_large_items() {
        let small = LocalNotebookChange {
            notebook_id: "small".to_string(),
            base_version: 0,
            deleted: false,
            updated_at_ms: 1,
            deleted_at_ms: None,
            content_hash: "h".to_string(),
            body_hash: String::new(),
            meta_hash: String::new(),
            meta_updated_at_ms: 1,
            document: Some(Document::default()),
        };
        let single_bytes = estimate_change_bytes(&small);
        assert!(single_bytes > 50);

        let changes = vec![small.clone(), small.clone(), small];
        let ranges = plan_push_chunk_ranges(&changes, 64, single_bytes + 10);
        assert_eq!(ranges, vec![0..1, 1..2, 2..3]);

        let oversized_ranges = plan_push_chunk_ranges(&changes, 64, 1);
        assert_eq!(oversized_ranges, vec![0..1, 1..2, 2..3]);
    }

    #[test]
    fn driver_pre_pull_merges_same_page_strokes_before_push_and_skips_fast_forward() {
        let mut base = Document::new(1200, 1600);
        base.id = "nb-1".to_string();
        base.title = "Shared".to_string();
        base.load_active_from_pages();

        let merge_base = DocumentMergeBase {
            page_hashes: canonical_document_page_hashes(&base),
            page_elements: compute_document_page_elements(&base),
            ..Default::default()
        };

        let mut local = base.clone();
        let l_sid = local.alloc_stroke_id();
        local
            .strokes
            .push(make_stroke(l_sid.0, 20.0, 20.0, 50.0, 50.0));
        local.sync_active_to_pages();

        let mut remote = base.clone();
        let r_sid = remote.alloc_stroke_id();
        remote
            .strokes
            .push(make_stroke(r_sid.0, 200.0, 200.0, 260.0, 240.0));
        remote.sync_active_to_pages();

        let mut identical_local = base.clone();
        identical_local.id = "nb-ff".to_string();
        identical_local.created_at_ms = 1000;
        identical_local.updated_at_ms = 1000;
        identical_local.meta_updated_at_ms = 1000;
        let identical_remote = identical_local.clone();

        let batch = SyncBatch {
            client_cursor: 1,
            device_name: "DevA".to_string(),
            local_docs: vec![
                LocalNotebookSnapshot {
                    notebook_id: "nb-1".to_string(),
                    base_version: 1,
                    cloud_dirty: true,
                    content_hash: String::new(),
                    doc: local,
                    merge_base,
                },
                LocalNotebookSnapshot {
                    notebook_id: "nb-ff".to_string(),
                    base_version: 1,
                    cloud_dirty: true,
                    content_hash: String::new(),
                    doc: identical_local,
                    merge_base: DocumentMergeBase::default(),
                },
            ],
            tombstones: Vec::new(),
            folder_changes: Vec::new(),
            prefs_push: None,
            force_overwrite: false,
        };

        let mut driver = SyncCycleDriver::new(&batch);
        let pre_pull = SyncPullResponse {
            server_cursor: 3,
            user: UserProfile {
                user_id: "u1".to_string(),
                email: Some("a@b.com".to_string()),
                username: "alice".to_string(),
                display_name: "Alice".to_string(),
                created_at_ms: 1,
                must_change_password: false,
            },
            notebooks: vec![
                remote_record_for(&remote, 2, 2),
                remote_record_for(&identical_remote, 2, 3),
            ],
            folders: Vec::new(),
            studio_prefs: None,
        };
        driver.apply_pre_pull(pre_pull, &batch);

        // `nb-ff` was fast-forwarded (no upload needed); `nb-1` was rebased onto version 2 with both strokes!
        let push_req = driver
            .next_push_request(&batch)
            .expect("should push merged nb-1");
        assert_eq!(push_req.changes.len(), 1);
        assert_eq!(push_req.changes[0].notebook_id, "nb-1");
        assert_eq!(push_req.changes[0].base_version, 2);
        let pushed_doc = push_req.changes[0].document.as_ref().unwrap();
        assert_eq!(pushed_doc.pages[0].strokes.len(), 2);

        driver.record_push_chunk_response(
            SyncPushResponse {
                server_cursor: 4,
                accepted: vec![AcceptedNotebookSync {
                    notebook_id: "nb-1".to_string(),
                    new_version: 3,
                    cursor: 4,
                    deleted: false,
                    content_hash: canonical_document_hash(pushed_doc),
                    body_hash: canonical_document_body_hash(pushed_doc),
                    meta_hash: canonical_document_meta_hash(pushed_doc),
                }],
                conflicts: Vec::new(),
                accepted_folders: Vec::new(),
                folder_conflicts: Vec::new(),
                prefs_version: 0,
                prefs_conflict: None,
            },
            &batch,
        );
        assert!(driver.next_push_request(&batch).is_none());

        let outcome = driver.finish_from_pre_pull(None);
        assert!(outcome.conflict_updates.is_empty());
        assert_eq!(
            outcome.rebased_docs.get("nb-1").unwrap().pages[0]
                .strokes
                .len(),
            2
        );
        assert!(
            outcome
                .downloaded_updates
                .iter()
                .any(|u| u.notebook_id == "nb-ff" && u.version == 2)
        );
    }

    #[test]
    fn driver_retries_push_conflict_with_automatic_three_way_merge() {
        let mut base = Document::new(1200, 1600);
        base.id = "nb-race".to_string();
        base.load_active_from_pages();

        let merge_base = DocumentMergeBase {
            page_hashes: canonical_document_page_hashes(&base),
            page_elements: compute_document_page_elements(&base),
            ..Default::default()
        };

        let mut local = base.clone();
        let ls = local.alloc_stroke_id();
        local
            .strokes
            .push(make_stroke(ls.0, 10.0, 10.0, 30.0, 30.0));
        local.sync_active_to_pages();

        let mut concurrent_remote = base;
        let rs = concurrent_remote.alloc_stroke_id();
        concurrent_remote
            .strokes
            .push(make_stroke(rs.0, 400.0, 400.0, 460.0, 460.0));
        concurrent_remote.sync_active_to_pages();

        let batch = SyncBatch {
            client_cursor: 1,
            device_name: "DevA".to_string(),
            local_docs: vec![LocalNotebookSnapshot {
                notebook_id: "nb-race".to_string(),
                base_version: 1,
                cloud_dirty: true,
                content_hash: String::new(),
                doc: local,
                merge_base,
            }],
            tombstones: Vec::new(),
            folder_changes: Vec::new(),
            prefs_push: None,
            force_overwrite: false,
        };

        let mut driver = SyncCycleDriver::new(&batch);
        let req1 = driver.next_push_request(&batch).unwrap();
        assert_eq!(req1.changes[0].base_version, 1);

        // Simulate race: another device committed v2 right between pull and push!
        driver.record_push_chunk_response(
            SyncPushResponse {
                server_cursor: 2,
                accepted: Vec::new(),
                conflicts: vec![ConflictNotebookSync {
                    notebook_id: "nb-race".to_string(),
                    server_record: remote_record_for(&concurrent_remote, 2, 2),
                }],
                accepted_folders: Vec::new(),
                folder_conflicts: Vec::new(),
                prefs_version: 0,
                prefs_conflict: None,
            },
            &batch,
        );

        // Driver automatically schedules retry round with base_version=2 and merged strokes!
        let req2 = driver
            .next_push_request(&batch)
            .expect("conflict must auto-queue retry push");
        assert_eq!(req2.changes.len(), 1);
        assert_eq!(req2.changes[0].base_version, 2);
        let merged_doc = req2.changes[0].document.as_ref().unwrap();
        assert_eq!(merged_doc.pages[0].strokes.len(), 2);

        driver.record_push_chunk_response(
            SyncPushResponse {
                server_cursor: 3,
                accepted: vec![AcceptedNotebookSync {
                    notebook_id: "nb-race".to_string(),
                    new_version: 3,
                    cursor: 3,
                    deleted: false,
                    content_hash: canonical_document_hash(merged_doc),
                    body_hash: canonical_document_body_hash(merged_doc),
                    meta_hash: canonical_document_meta_hash(merged_doc),
                }],
                conflicts: Vec::new(),
                accepted_folders: Vec::new(),
                folder_conflicts: Vec::new(),
                prefs_version: 0,
                prefs_conflict: None,
            },
            &batch,
        );
        assert!(driver.next_push_request(&batch).is_none());

        let outcome = driver.finish_from_pre_pull(None);
        assert!(outcome.conflict_updates.is_empty());
        assert_eq!(outcome.accepted_uploads.len(), 1);
        assert_eq!(outcome.accepted_uploads[0].new_version, 3);
        assert_eq!(
            outcome.rebased_docs.get("nb-race").unwrap().pages[0]
                .strokes
                .len(),
            2
        );
    }
}
