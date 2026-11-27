use std::collections::HashMap;
use std::ops::Range;

use papercraft_doc::NotebookPage;
use papercraft_sync_protocol::{
    AcceptedFolderSync, ConflictNotebookSync, LocalNotebookChange, MAX_PUSH_BATCH_SIZE,
    RemoteFolderRecord, RemoteNotebookRecord, RemotePrefsRecord, SyncPullResponse, SyncPushRequest,
    SyncPushResponse, canonical_document_all_hashes, canonical_document_page_hashes, now_unix_ms,
};

use crate::client::SyncHttpClient;
use crate::types::{
    ActiveAccountSession, RemoteNotebookUpdate, SyncBatch, SyncCycleOutcome, SyncError,
};

const TARGET_PUSH_CHUNK_BYTES: usize = 24 * 1024 * 1024;

/// Pure step-by-step sync state machine shared by blocking desktop HTTP and async web `fetch`.
pub struct SyncCycleDriver {
    changes: Vec<LocalNotebookChange>,
    sent_page_hashes: HashMap<String, Vec<String>>,
    ranges: Vec<Range<usize>>,
    cursor: usize,
    prefs_only_pending: bool,
    accum: PushAccum,
}

impl SyncCycleDriver {
    /// Prepares chunked push requests from a captured [`SyncBatch`].
    pub fn new(batch: &SyncBatch) -> Self {
        let (changes, sent_page_hashes) = build_pending_changes(batch);
        let ranges = plan_push_chunk_ranges(&changes, MAX_PUSH_BATCH_SIZE, TARGET_PUSH_CHUNK_BYTES);
        let prefs_only_pending =
            changes.is_empty() && (batch.prefs_push.is_some() || !batch.folder_changes.is_empty());
        Self {
            changes,
            sent_page_hashes,
            ranges,
            cursor: 0,
            prefs_only_pending,
            accum: PushAccum::default(),
        }
    }

    /// Returns the next chunked push request, or `None` once all push requests have completed.
    pub fn next_push_request(&self, batch: &SyncBatch) -> Option<SyncPushRequest> {
        if self.prefs_only_pending {
            return Some(SyncPushRequest {
                client_cursor: batch.client_cursor,
                device_name: batch.device_name.clone(),
                changes: Vec::new(),
                folders: batch.folder_changes.clone(),
                studio_prefs: batch.prefs_push.clone(),
                force_overwrite: batch.force_overwrite,
            });
        }
        let range = self.ranges.get(self.cursor)?.clone();
        let is_first = self.cursor == 0;
        let prefs_for_chunk = if is_first {
            batch.prefs_push.clone()
        } else {
            None
        };
        let folders_for_chunk = if is_first {
            batch.folder_changes.clone()
        } else {
            Vec::new()
        };
        Some(SyncPushRequest {
            client_cursor: batch.client_cursor,
            device_name: batch.device_name.clone(),
            changes: self.changes[range].to_vec(),
            folders: folders_for_chunk,
            studio_prefs: prefs_for_chunk,
            force_overwrite: batch.force_overwrite,
        })
    }

    /// Records the server's push response and advances to the next chunk.
    pub fn record_push_chunk_response(&mut self, resp: SyncPushResponse, batch: &SyncBatch) {
        let sent_prefs = if self.prefs_only_pending || self.cursor == 0 {
            batch.prefs_push.as_ref()
        } else {
            None
        };
        record_push_response(resp, sent_prefs, &self.sent_page_hashes, &mut self.accum);
        if self.prefs_only_pending {
            self.prefs_only_pending = false;
        } else if self.cursor < self.ranges.len() {
            self.cursor += 1;
        }
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
        let downloaded_folders: Vec<RemoteFolderRecord> = pull_resp
            .folders
            .into_iter()
            .filter(|rf| {
                !self
                    .accum
                    .accepted_folders
                    .iter()
                    .any(|af| af.folder_id == rf.folder_id && af.new_version == rf.version)
            })
            .collect();
        let updated_prefs = select_pulled_prefs(
            pull_resp.studio_prefs,
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

    while let Some(req) = driver.next_push_request(batch) {
        let (resp, _) = client.push_with_refresh(session, &req)?;
        driver.record_push_chunk_response(resp, batch);
    }
    let (pull_resp, refreshed_on_pull) = client.pull_with_refresh(session, batch.client_cursor)?;
    let token_changed = refreshed_on_pull || session.access_token != initial_refresh;

    Ok(driver.finish(pull_resp, token_changed.then(|| session.clone())))
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
    accepted_folders: Vec<AcceptedFolderSync>,
    cleared_tombstones: Vec<String>,
    downloaded_updates: Vec<RemoteNotebookUpdate>,
    conflict_updates: Vec<RemoteNotebookUpdate>,
    ack_prefs: Option<(u64, String)>,
    prefs_conflict: Option<RemotePrefsRecord>,
    highest_cursor: u64,
    pushed_count: usize,
}

fn build_pending_changes(
    batch: &SyncBatch,
) -> (Vec<LocalNotebookChange>, HashMap<String, Vec<String>>) {
    let mut changes = Vec::new();
    let mut page_hashes_map = HashMap::new();
    let now = now_unix_ms();

    for snap in &batch.local_docs {
        if !batch.force_overwrite && !snap.cloud_dirty && snap.base_version > 0 {
            continue;
        }
        let mut doc = snap.doc.clone();
        if doc.id.is_empty() {
            doc.id = snap.notebook_id.clone();
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
        let hashes = canonical_document_all_hashes(&doc);
        let content_hash = if snap.content_hash.is_empty() {
            hashes.content_hash
        } else {
            snap.content_hash.clone()
        };
        page_hashes_map.insert(snap.notebook_id.clone(), hashes.page_hashes);
        let updated_at_ms = doc.updated_at_ms.max(now);
        let meta_updated_at_ms = doc.meta_updated_at_ms.max(updated_at_ms);
        changes.push(LocalNotebookChange {
            notebook_id: snap.notebook_id.clone(),
            base_version: snap.base_version,
            deleted: false,
            updated_at_ms,
            deleted_at_ms: None,
            content_hash,
            body_hash: hashes.body_hash,
            meta_hash: hashes.meta_hash,
            meta_updated_at_ms,
            document: Some(doc),
        });
    }

    for tomb in &batch.tombstones {
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
    (changes, page_hashes_map)
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

fn record_push_response(
    resp: SyncPushResponse,
    sent_prefs: Option<&papercraft_sync_protocol::LocalPrefsPush>,
    sent_page_hashes: &HashMap<String, Vec<String>>,
    accum: &mut PushAccum,
) {
    accum.highest_cursor = accum.highest_cursor.max(resp.server_cursor);
    if let Some(conflict_prefs) = resp.prefs_conflict {
        let should_take = accum
            .prefs_conflict
            .as_ref()
            .is_none_or(|cur| conflict_prefs.version >= cur.version);
        if should_take {
            accum.prefs_conflict = Some(conflict_prefs);
        }
    } else if let Some(prefs) = sent_prefs {
        accum.ack_prefs = Some((resp.prefs_version, prefs.content_hash.clone()));
    }

    for acc in resp.accepted {
        accum.pushed_count += 1;
        if acc.deleted {
            accum.cleared_tombstones.push(acc.notebook_id);
        } else {
            if let Some(hashes) = sent_page_hashes.get(&acc.notebook_id) {
                accum
                    .accepted_page_hashes
                    .insert(acc.notebook_id.clone(), hashes.clone());
            }
            accum.accepted_uploads.push(acc);
        }
    }
    accum.accepted_folders.extend(resp.accepted_folders);

    for conflict in resp.conflicts {
        handle_conflict(&conflict, accum);
    }
}

fn handle_conflict(conflict: &ConflictNotebookSync, accum: &mut PushAccum) {
    // Queue full server record into `conflict_updates` so the session engine runs
    // safe 3-way page merging or creates a conflict copy without ever dropping user edits.
    accum.cleared_tombstones.push(conflict.notebook_id.clone());
    upsert_downloaded_update(&conflict.server_record, &mut accum.conflict_updates);
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
    use super::*;
    use papercraft_doc::Document;

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
        // Tight byte budget forces 1 item per chunk even though max_count is 64.
        let ranges = plan_push_chunk_ranges(&changes, 64, single_bytes + 10);
        assert_eq!(ranges, vec![0..1, 1..2, 2..3]);

        // Even when budget is smaller than 1 item, progress is still guaranteed.
        let oversized_ranges = plan_push_chunk_ranges(&changes, 64, 1);
        assert_eq!(oversized_ranges, vec![0..1, 1..2, 2..3]);
    }
}
