//! Detached binary document serialization, SHA-256 blob deduplication, and
//! incremental Web storage deltas for IndexedDB persistence.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Weak};

use papercraft_doc::{
    CustomPaperTemplate, CustomTemplateKind, Document, EmbeddedImageTemplate, EmbeddedPdf,
    PdfPageBackdrop, UiTheme, UserStudioConfig, sanitize_notebook_id, sanitize_notebook_title,
};
use papercraft_format::{load_document_bytes, save_document_bytes};
use papercraft_sync::{ActiveWorkspaceTarget, GlobalSyncState, OwnerId, SyncManifest};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::Session;
use crate::library::TrashedNotebook;
use crate::sync_state::{
    CachedLibraryBundle, CachedTrashEntry, WebWorkspaceCache, cloud_account_cache_key,
    ensure_default_profile_in_global, migrate_web_cache_if_needed, sanitize_global_state,
};

pub const DETACHED_DOC_MAGIC: &[u8; 4] = b"PCDB";
pub const DETACHED_DOC_VERSION: u16 = 1;
pub const WEB_STORAGE_SCHEMA_VERSION: u32 = 1;
const MAX_DETACHED_META_BYTES: usize = 8 * 1024 * 1024;
const MAX_HASH_CACHE_ENTRIES: usize = 512;

/// Pair of `(sha256_hex, raw_bytes)` ready to be stored in the IndexedDB `blobs` store.
pub type PendingBlob = (String, Arc<Vec<u8>>);

/// Returns the IndexedDB workspace key for a local profile.
#[must_use]
pub fn local_workspace_key(profile_id: &str) -> String {
    format!("local:{profile_id}")
}

/// Returns the IndexedDB workspace key for a cached cloud account.
#[must_use]
pub fn cloud_workspace_key(server_url: &str, user_id: &str) -> String {
    format!("cloud:{}", cloud_account_cache_key(server_url, user_id))
}

/// Returns the composite IndexedDB key for a notebook or trashed item in a workspace.
#[must_use]
pub fn scoped_item_key(workspace_key: &str, item_id: &str) -> String {
    format!("{workspace_key}::{item_id}")
}

/// Returns inclusive lower and upper key bounds covering all items under `workspace_key`.
#[must_use]
pub fn workspace_key_prefix_bounds(workspace_key: &str) -> (String, String) {
    (
        format!("{workspace_key}::"),
        format!("{workspace_key}::\u{10FFFF}"),
    )
}

/// Computes the lowercase hex SHA-256 digest of `bytes`.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0F)]));
    }
    out
}

/// O(1) pointer-keyed cache of SHA-256 digests for `Arc<Vec<u8>>` payloads.
#[derive(Clone, Debug, Default)]
pub struct BlobHashCache {
    entries: HashMap<usize, (Weak<Vec<u8>>, String)>,
}

impl BlobHashCache {
    /// Returns the SHA-256 hex digest of `bytes`, reusing a cached digest when the same `Arc`
    /// allocation is still alive.
    pub fn hash_arc(&mut self, bytes: &Arc<Vec<u8>>) -> String {
        let ptr = Arc::as_ptr(bytes) as usize;
        if let Some((weak, hash)) = self.entries.get(&ptr)
            && weak
                .upgrade()
                .is_some_and(|existing| Arc::ptr_eq(&existing, bytes))
        {
            return hash.clone();
        }
        let hash = sha256_hex(bytes.as_slice());
        if self.entries.len() > MAX_HASH_CACHE_ENTRIES {
            self.entries.retain(|_, (weak, _)| weak.strong_count() > 0);
        }
        self.entries
            .insert(ptr, (Arc::downgrade(bytes), hash.clone()));
        hash
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DetachedNamedBlob {
    name: String,
    hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DetachedPageImageBlob {
    page_idx: usize,
    image_idx: usize,
    hash: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
struct DetachedDocBlobTable {
    #[serde(default)]
    pdf_documents: Vec<DetachedNamedBlob>,
    #[serde(default)]
    page_pdf_backdrops: Vec<(usize, PdfPageBackdrop)>,
    #[serde(default)]
    image_templates: Vec<DetachedNamedBlob>,
    #[serde(default)]
    page_image_backdrops: Vec<(usize, usize)>,
    #[serde(default)]
    page_image_blobs: Vec<DetachedPageImageBlob>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DetachedTrashMeta {
    trash_id: String,
    deleted_at_ms: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DetachedCustomTemplateMeta {
    id: String,
    name: String,
    kind: CustomTemplateKind,
    width: u32,
    height: u32,
    #[serde(default)]
    pdf_page_index: usize,
    hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DetachedElementImageBlob {
    element_idx: usize,
    image_idx: usize,
    hash: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct DetachedWorkspaceRecord {
    version: u32,
    #[serde(default)]
    active_notebook: usize,
    #[serde(default)]
    notebook_ids: Vec<String>,
    #[serde(default)]
    trash_entries: Vec<DetachedTrashMeta>,
    #[serde(default)]
    prefs: UserStudioConfig,
    #[serde(default)]
    custom_template_blobs: Vec<DetachedCustomTemplateMeta>,
    #[serde(default)]
    saved_element_image_blobs: Vec<DetachedElementImageBlob>,
    #[serde(default)]
    manifest: SyncManifest,
    #[serde(default)]
    slot_ids: Vec<String>,
    #[serde(default)]
    slot_dirty_hash: Vec<bool>,
    #[serde(default)]
    slot_placeholder_blank: Vec<bool>,
    #[serde(default)]
    prefs_dirty_pending: bool,
    #[serde(default)]
    last_dirty_at_ms: Option<i64>,
}

/// In-memory hydrated metadata snapshot for a single workspace (without notebook payloads).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WebWorkspaceMetaSnapshot {
    pub active_notebook: usize,
    pub notebook_ids: Vec<String>,
    pub trash_entries: Vec<(String, i64)>,
    pub prefs: UserStudioConfig,
    pub manifest: SyncManifest,
    pub slot_ids: Vec<String>,
    pub slot_dirty_hash: Vec<bool>,
    pub slot_placeholder_blank: Vec<bool>,
    pub prefs_dirty_pending: bool,
    pub last_dirty_at_ms: Option<i64>,
}

/// One fully hydrated workspace restored from IndexedDB or legacy `localStorage` migration.
#[derive(Clone, Debug)]
pub struct WebStorageLoadedWorkspace {
    pub workspace_key: String,
    pub meta: WebWorkspaceMetaSnapshot,
    pub notebooks: HashMap<String, Document>,
    pub trash: HashMap<String, (i64, Document)>,
}

/// Full hydrated browser state passed into [`Session::open_web_storage`].
#[derive(Clone, Debug)]
pub struct WebStorageLoadedState {
    pub global: GlobalSyncState,
    pub workspaces: Vec<WebStorageLoadedWorkspace>,
}

/// Granular incremental batch of IndexedDB writes and deletions emitted by [`Session`].
#[derive(Clone, Debug, Default)]
pub struct WebStorageDelta {
    pub global: Option<GlobalSyncState>,
    pub put_workspaces: HashMap<String, WebWorkspaceMetaSnapshot>,
    pub delete_workspaces: HashSet<String>,
    pub put_notebooks: HashMap<(String, String), Arc<Document>>,
    pub delete_notebooks: HashSet<(String, String)>,
    pub put_trash: HashMap<(String, String), (i64, Arc<Document>)>,
    pub delete_trash: HashSet<(String, String)>,
    pub theme_hint: Option<UiTheme>,
    pub urgent: bool,
}

impl WebStorageDelta {
    /// Returns `true` when this delta carries no mutations.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.global.is_none()
            && self.put_workspaces.is_empty()
            && self.delete_workspaces.is_empty()
            && self.put_notebooks.is_empty()
            && self.delete_notebooks.is_empty()
            && self.put_trash.is_empty()
            && self.delete_trash.is_empty()
            && self.theme_hint.is_none()
    }

    /// Returns `true` when this delta removes any workspace, notebook, or trashed item.
    #[must_use]
    pub fn has_deletions(&self) -> bool {
        !self.delete_workspaces.is_empty()
            || !self.delete_notebooks.is_empty()
            || !self.delete_trash.is_empty()
    }

    /// Coalesces `newer` onto `self`, ensuring deletions cancel superseded puts and vice versa.
    pub fn merge(&mut self, newer: Self) {
        if let Some(global) = newer.global {
            self.global = Some(global);
        }
        for ws_key in newer.delete_workspaces {
            self.put_workspaces.remove(&ws_key);
            self.put_notebooks.retain(|(ws, _), _| ws != &ws_key);
            self.delete_notebooks.retain(|(ws, _)| ws != &ws_key);
            self.put_trash.retain(|(ws, _), _| ws != &ws_key);
            self.delete_trash.retain(|(ws, _)| ws != &ws_key);
            self.delete_workspaces.insert(ws_key);
        }
        for (ws_key, meta) in newer.put_workspaces {
            self.delete_workspaces.remove(&ws_key);
            self.put_workspaces.insert(ws_key, meta);
        }
        for key in newer.delete_notebooks {
            self.put_notebooks.remove(&key);
            self.delete_notebooks.insert(key);
        }
        for (key, doc) in newer.put_notebooks {
            self.delete_notebooks.remove(&key);
            self.put_notebooks.insert(key, doc);
        }
        for key in newer.delete_trash {
            self.put_trash.remove(&key);
            self.delete_trash.insert(key);
        }
        for (key, entry) in newer.put_trash {
            self.delete_trash.remove(&key);
            self.put_trash.insert(key, entry);
        }
        if let Some(theme) = newer.theme_hint {
            self.theme_hint = Some(theme);
        }
        self.urgent |= newer.urgent;
    }
}

fn record_blob(
    bytes: &Arc<Vec<u8>>,
    cache: &mut BlobHashCache,
    known_blobs: &HashSet<String>,
    emitted_hashes: &mut HashSet<String>,
    out_blobs: &mut Vec<PendingBlob>,
) -> String {
    let hash = cache.hash_arc(bytes);
    if !known_blobs.contains(&hash) && emitted_hashes.insert(hash.clone()) {
        out_blobs.push((hash.clone(), Arc::clone(bytes)));
    }
    hash
}

fn strip_document_pdfs_and_templates(
    stripped: &mut Document,
    table: &mut DetachedDocBlobTable,
    cache: &mut BlobHashCache,
    known_blobs: &HashSet<String>,
    emitted_hashes: &mut HashSet<String>,
    out_blobs: &mut Vec<PendingBlob>,
) {
    let mut pdf_remap: Vec<Option<usize>> = Vec::with_capacity(stripped.pdf_documents.len());
    for pdf in &stripped.pdf_documents {
        let clean = pdf.sanitized();
        if clean.bytes.is_empty() {
            pdf_remap.push(None);
            continue;
        }
        let hash = record_blob(&clean.bytes, cache, known_blobs, emitted_hashes, out_blobs);
        let new_idx = table.pdf_documents.len();
        table.pdf_documents.push(DetachedNamedBlob {
            name: clean.name,
            hash,
        });
        pdf_remap.push(Some(new_idx));
    }

    let mut img_tpl_remap: Vec<Option<usize>> = Vec::with_capacity(stripped.image_templates.len());
    for tpl in &stripped.image_templates {
        let clean = tpl.sanitized();
        if clean.png_bytes.is_empty() {
            img_tpl_remap.push(None);
            continue;
        }
        let hash = record_blob(
            &clean.png_bytes,
            cache,
            known_blobs,
            emitted_hashes,
            out_blobs,
        );
        let new_idx = table.image_templates.len();
        table.image_templates.push(DetachedNamedBlob {
            name: clean.name,
            hash,
        });
        img_tpl_remap.push(Some(new_idx));
    }

    for (page_idx, page) in stripped.pages.iter_mut().enumerate() {
        if let Some(b) = page.pdf_backdrop.take()
            && let Some(new_pdf_idx) = pdf_remap.get(b.pdf_index).copied().flatten()
        {
            table.page_pdf_backdrops.push((
                page_idx,
                PdfPageBackdrop {
                    pdf_index: new_pdf_idx,
                    page_index: b.page_index,
                },
            ));
        }
        if let Some(old_tpl_idx) = page.custom_image_backdrop.take()
            && let Some(new_tpl_idx) = img_tpl_remap.get(old_tpl_idx).copied().flatten()
        {
            table.page_image_backdrops.push((page_idx, new_tpl_idx));
        }
    }

    stripped.pdf_documents.clear();
    stripped.pdf_backdrop = None;
    stripped.image_templates.clear();
    stripped.custom_image_backdrop = None;
}

/// Encodes a [`Document`] into the compact `PCDB` detached binary envelope for IndexedDB,
/// returning `(detached_bytes, missing_blobs)` where `missing_blobs` skips any digest already
/// present in `known_blobs`.
pub fn encode_detached_notebook(
    doc: &Document,
    cache: &mut BlobHashCache,
    known_blobs: &HashSet<String>,
) -> Result<(Vec<u8>, Vec<PendingBlob>), String> {
    let mut stripped = doc.clone();
    stripped.sync_active_to_pages();

    let mut table = DetachedDocBlobTable::default();
    let mut out_blobs = Vec::new();
    let mut emitted_hashes = HashSet::new();

    strip_document_pdfs_and_templates(
        &mut stripped,
        &mut table,
        cache,
        known_blobs,
        &mut emitted_hashes,
        &mut out_blobs,
    );

    for (page_idx, page) in stripped.pages.iter_mut().enumerate() {
        for (image_idx, img_arc) in page.images.iter_mut().enumerate() {
            if img_arc.png_bytes.is_empty() {
                continue;
            }
            let hash = record_blob(
                &img_arc.png_bytes,
                cache,
                known_blobs,
                &mut emitted_hashes,
                &mut out_blobs,
            );
            table.page_image_blobs.push(DetachedPageImageBlob {
                page_idx,
                image_idx,
                hash,
            });
            Arc::make_mut(img_arc).png_bytes = Arc::new(Vec::new());
        }
    }

    stripped.load_active_from_pages();
    let v2_bytes = save_document_bytes(&stripped)
        .map_err(|err| format!("failed encoding detached notebook body: {err}"))?;
    let meta_json = serde_json::to_vec(&table)
        .map_err(|err| format!("failed encoding detached blob table: {err}"))?;
    let meta_len = u32::try_from(meta_json.len())
        .map_err(|_| "detached blob table exceeds 4 GiB limit".to_string())?;

    let mut out = Vec::with_capacity(10 + meta_json.len() + v2_bytes.len());
    out.extend_from_slice(DETACHED_DOC_MAGIC);
    out.extend_from_slice(&DETACHED_DOC_VERSION.to_le_bytes());
    out.extend_from_slice(&meta_len.to_le_bytes());
    out.extend_from_slice(&meta_json);
    out.extend_from_slice(&v2_bytes);
    Ok((out, out_blobs))
}

fn parse_detached_header(raw: &[u8]) -> Result<(DetachedDocBlobTable, &[u8]), String> {
    if raw.len() < 10 || &raw[0..4] != DETACHED_DOC_MAGIC {
        return Err("not a PCDB detached notebook stream".to_string());
    }
    let version = u16::from_le_bytes([raw[4], raw[5]]);
    if version != DETACHED_DOC_VERSION {
        return Err(format!("unsupported PCDB detached version {version}"));
    }
    let meta_len = u32::from_le_bytes([raw[6], raw[7], raw[8], raw[9]]) as usize;
    if meta_len > MAX_DETACHED_META_BYTES {
        return Err("PCDB detached metadata exceeds safety limit".to_string());
    }
    let meta_end = 10usize
        .checked_add(meta_len)
        .filter(|&end| end <= raw.len())
        .ok_or_else(|| "truncated PCDB detached metadata".to_string())?;
    let table: DetachedDocBlobTable = serde_json::from_slice(&raw[10..meta_end])
        .map_err(|err| format!("corrupt PCDB detached metadata: {err}"))?;
    Ok((table, &raw[meta_end..]))
}

/// Decodes a `PCDB` detached binary notebook (or legacy `.pcraft` stream) and rehydrates all
/// embedded PDFs, image templates, and page images from `blobs`.
pub fn decode_detached_notebook(
    raw: &[u8],
    blobs: &HashMap<String, Arc<Vec<u8>>>,
) -> Result<Document, String> {
    if !raw.starts_with(DETACHED_DOC_MAGIC) {
        return load_document_bytes(raw).map_err(|err| err.to_string());
    }
    let (table, v2_bytes) = parse_detached_header(raw)?;
    let mut doc = load_document_bytes(v2_bytes)
        .map_err(|err| format!("failed decoding detached notebook body: {err}"))?;

    doc.pdf_documents = table
        .pdf_documents
        .into_iter()
        .map(|entry| {
            let bytes = blobs
                .get(&entry.hash)
                .map(Arc::clone)
                .unwrap_or_else(|| Arc::new(Vec::new()));
            EmbeddedPdf {
                name: entry.name,
                bytes,
            }
        })
        .collect();
    for (page_idx, backdrop) in table.page_pdf_backdrops {
        if let Some(page) = doc.pages.get_mut(page_idx)
            && doc
                .pdf_documents
                .get(backdrop.pdf_index)
                .is_some_and(|pdf| !pdf.bytes.is_empty())
        {
            page.pdf_backdrop = Some(backdrop);
        }
    }

    doc.image_templates = table
        .image_templates
        .into_iter()
        .map(|entry| {
            let png_bytes = blobs
                .get(&entry.hash)
                .map(Arc::clone)
                .unwrap_or_else(|| Arc::new(Vec::new()));
            EmbeddedImageTemplate {
                name: entry.name,
                png_bytes,
            }
        })
        .collect();
    for (page_idx, tpl_idx) in table.page_image_backdrops {
        if let Some(page) = doc.pages.get_mut(page_idx)
            && doc
                .image_templates
                .get(tpl_idx)
                .is_some_and(|tpl| !tpl.png_bytes.is_empty())
        {
            page.custom_image_backdrop = Some(tpl_idx);
        }
    }

    for entry in table.page_image_blobs {
        if let Some(page) = doc.pages.get_mut(entry.page_idx)
            && let Some(img_arc) = page.images.get_mut(entry.image_idx)
            && let Some(bytes) = blobs.get(&entry.hash)
        {
            Arc::make_mut(img_arc).png_bytes = Arc::clone(bytes);
        }
    }
    for page in &mut doc.pages {
        page.images.retain(|im| !im.png_bytes.is_empty());
    }
    doc.load_active_from_pages();
    Ok(doc)
}

/// Fast header-only scan extracting every SHA-256 blob key referenced by a `PCDB` notebook payload
/// without decompressing its `.pcraft` v2 stroke body.
#[must_use]
pub fn extract_detached_notebook_blob_hashes(raw: &[u8]) -> Vec<String> {
    let Ok((table, _)) = parse_detached_header(raw) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(
        table.pdf_documents.len() + table.image_templates.len() + table.page_image_blobs.len(),
    );
    for entry in table.pdf_documents {
        out.push(entry.hash);
    }
    for entry in table.image_templates {
        out.push(entry.hash);
    }
    for entry in table.page_image_blobs {
        out.push(entry.hash);
    }
    out
}

fn build_detached_workspace_record(
    meta: &WebWorkspaceMetaSnapshot,
    cache: &mut BlobHashCache,
    known_blobs: &HashSet<String>,
    emit_missing_blobs: bool,
) -> Result<(String, String, Vec<PendingBlob>), String> {
    let mut out_blobs = Vec::new();
    let mut emitted_hashes = HashSet::new();
    let mut stripped_prefs = meta.prefs.clone();

    let mut custom_template_blobs = Vec::with_capacity(stripped_prefs.custom_templates.len());
    for tpl in stripped_prefs.custom_templates.drain(..) {
        let Some(clean) = tpl.sanitized() else {
            continue;
        };
        let hash = if emit_missing_blobs {
            record_blob(
                &clean.bytes,
                cache,
                known_blobs,
                &mut emitted_hashes,
                &mut out_blobs,
            )
        } else {
            cache.hash_arc(&clean.bytes)
        };
        custom_template_blobs.push(DetachedCustomTemplateMeta {
            id: clean.id,
            name: clean.name,
            kind: clean.kind,
            width: clean.width,
            height: clean.height,
            pdf_page_index: clean.pdf_page_index,
            hash,
        });
    }

    let mut saved_element_image_blobs = Vec::new();
    for (element_idx, el) in stripped_prefs.saved_elements.iter_mut().enumerate() {
        for (image_idx, im) in el.images.iter_mut().enumerate() {
            if im.png_bytes.is_empty() {
                continue;
            }
            let hash = if emit_missing_blobs {
                record_blob(
                    &im.png_bytes,
                    cache,
                    known_blobs,
                    &mut emitted_hashes,
                    &mut out_blobs,
                )
            } else {
                cache.hash_arc(&im.png_bytes)
            };
            saved_element_image_blobs.push(DetachedElementImageBlob {
                element_idx,
                image_idx,
                hash,
            });
            im.png_bytes = Arc::new(Vec::new());
        }
    }

    let trash_entries: Vec<DetachedTrashMeta> = meta
        .trash_entries
        .iter()
        .map(|(trash_id, deleted_at_ms)| DetachedTrashMeta {
            trash_id: trash_id.clone(),
            deleted_at_ms: *deleted_at_ms,
        })
        .collect();

    let structural_key = serde_json::to_string(&(
        meta.active_notebook,
        &meta.notebook_ids,
        &trash_entries,
        &stripped_prefs,
        &custom_template_blobs,
        &saved_element_image_blobs,
    ))
    .map_err(|err| format!("failed encoding workspace structural key: {err}"))?;

    let record = DetachedWorkspaceRecord {
        version: WEB_STORAGE_SCHEMA_VERSION,
        active_notebook: meta.active_notebook,
        notebook_ids: meta.notebook_ids.clone(),
        trash_entries,
        prefs: stripped_prefs,
        custom_template_blobs,
        saved_element_image_blobs,
        manifest: meta.manifest.clone(),
        slot_ids: meta.slot_ids.clone(),
        slot_dirty_hash: meta.slot_dirty_hash.clone(),
        slot_placeholder_blank: meta.slot_placeholder_blank.clone(),
        prefs_dirty_pending: meta.prefs_dirty_pending,
        last_dirty_at_ms: meta.last_dirty_at_ms,
    };

    let json = serde_json::to_string(&record)
        .map_err(|err| format!("failed encoding detached workspace metadata: {err}"))?;
    Ok((json, structural_key, out_blobs))
}

/// Serializes `meta` into compact JSON with custom paper template payloads and saved element
/// image payloads detached into `blobs`.
pub fn encode_detached_workspace(
    meta: &WebWorkspaceMetaSnapshot,
    cache: &mut BlobHashCache,
    known_blobs: &HashSet<String>,
) -> Result<(String, Vec<PendingBlob>), String> {
    let (json, _, out_blobs) = build_detached_workspace_record(meta, cache, known_blobs, true)?;
    Ok((json, out_blobs))
}

/// Computes the deterministic detached JSON and structural key used by the engine diff tracker
/// without allocating missing-blob outputs.
pub(crate) fn detached_workspace_diff_keys(
    meta: &WebWorkspaceMetaSnapshot,
    cache: &mut BlobHashCache,
) -> Result<(String, String), String> {
    let empty = HashSet::new();
    let (json, structural_key, _) = build_detached_workspace_record(meta, cache, &empty, false)?;
    Ok((json, structural_key))
}

/// Decodes a detached workspace JSON record and rehydrates custom templates and saved element
/// images from `blobs` before sanitizing preferences.
pub fn decode_detached_workspace(
    json: &str,
    blobs: &HashMap<String, Arc<Vec<u8>>>,
) -> Result<WebWorkspaceMetaSnapshot, String> {
    let mut record: DetachedWorkspaceRecord = serde_json::from_str(json)
        .map_err(|err| format!("corrupt detached workspace metadata: {err}"))?;

    for meta_tpl in record.custom_template_blobs {
        if let Some(bytes) = blobs.get(&meta_tpl.hash) {
            record.prefs.custom_templates.push(CustomPaperTemplate {
                id: meta_tpl.id,
                name: meta_tpl.name,
                kind: meta_tpl.kind,
                width: meta_tpl.width,
                height: meta_tpl.height,
                pdf_page_index: meta_tpl.pdf_page_index,
                bytes: Arc::clone(bytes),
            });
        }
    }

    for entry in record.saved_element_image_blobs {
        if let Some(el) = record.prefs.saved_elements.get_mut(entry.element_idx)
            && let Some(im) = el.images.get_mut(entry.image_idx)
            && let Some(bytes) = blobs.get(&entry.hash)
        {
            im.png_bytes = Arc::clone(bytes);
        }
    }
    for el in &mut record.prefs.saved_elements {
        el.images.retain(|im| !im.png_bytes.is_empty());
    }

    let trash_entries = record
        .trash_entries
        .into_iter()
        .map(|entry| (entry.trash_id, entry.deleted_at_ms))
        .collect();

    Ok(WebWorkspaceMetaSnapshot {
        active_notebook: record.active_notebook,
        notebook_ids: record.notebook_ids,
        trash_entries,
        prefs: record.prefs.sanitized(),
        manifest: record.manifest,
        slot_ids: record.slot_ids,
        slot_dirty_hash: record.slot_dirty_hash,
        slot_placeholder_blank: record.slot_placeholder_blank,
        prefs_dirty_pending: record.prefs_dirty_pending,
        last_dirty_at_ms: record.last_dirty_at_ms,
    })
}

/// Extracts every SHA-256 blob key referenced by a detached workspace metadata JSON string.
#[must_use]
pub fn extract_detached_workspace_blob_hashes(json: &str) -> Vec<String> {
    let Ok(record) = serde_json::from_str::<DetachedWorkspaceRecord>(json) else {
        return Vec::new();
    };
    let mut out = Vec::with_capacity(
        record.custom_template_blobs.len() + record.saved_element_image_blobs.len(),
    );
    for entry in record.custom_template_blobs {
        out.push(entry.hash);
    }
    for entry in record.saved_element_image_blobs {
        out.push(entry.hash);
    }
    out
}

struct BundleConversion {
    loaded: WebStorageLoadedWorkspace,
    notebooks: Vec<(String, Arc<Document>)>,
    trash: Vec<(String, i64, Arc<Document>)>,
}

fn build_loaded_workspace_from_bundle(
    workspace_key: String,
    bundle: &CachedLibraryBundle,
) -> BundleConversion {
    let mut notebook_ids = Vec::with_capacity(bundle.notebooks.len());
    let mut notebooks_map = HashMap::with_capacity(bundle.notebooks.len());
    let mut delta_notebooks = Vec::with_capacity(bundle.notebooks.len());
    let mut slot_ids = bundle.slot_ids.clone();
    slot_ids.resize_with(bundle.notebooks.len(), String::new);

    for (idx, raw_doc) in bundle.notebooks.iter().enumerate() {
        let mut doc = raw_doc.clone();
        let clean_id = sanitize_notebook_id(&doc.id);
        let slot_id = sanitize_notebook_id(slot_ids.get(idx).map_or("", String::as_str));
        let nb_id = if !clean_id.is_empty() {
            clean_id
        } else if !slot_id.is_empty() {
            slot_id
        } else {
            Uuid::new_v4().to_string()
        };
        doc.id = nb_id.clone();
        doc.title = sanitize_notebook_title(&doc.title);
        doc.sync_active_to_pages();
        doc.load_active_from_pages();
        slot_ids[idx] = nb_id.clone();
        notebook_ids.push(nb_id.clone());
        delta_notebooks.push((nb_id.clone(), Arc::new(doc.clone())));
        notebooks_map.insert(nb_id, doc);
    }

    let mut trash_entries = Vec::with_capacity(bundle.trash.len());
    let mut trash_map = HashMap::with_capacity(bundle.trash.len());
    let mut delta_trash = Vec::with_capacity(bundle.trash.len());
    for entry in &bundle.trash {
        let trash_id = if entry.trash_id.trim().is_empty() {
            Uuid::new_v4().to_string()
        } else {
            entry.trash_id.clone()
        };
        let mut doc = entry.doc.clone();
        doc.id = sanitize_notebook_id(&doc.id);
        if doc.id.is_empty() {
            doc.id = Uuid::new_v4().to_string();
        }
        doc.title = sanitize_notebook_title(&doc.title);
        doc.sync_active_to_pages();
        doc.load_active_from_pages();
        trash_entries.push((trash_id.clone(), entry.deleted_at_ms));
        delta_trash.push((trash_id.clone(), entry.deleted_at_ms, Arc::new(doc.clone())));
        trash_map.insert(trash_id, (entry.deleted_at_ms, doc));
    }

    let meta = WebWorkspaceMetaSnapshot {
        active_notebook: bundle.active_notebook,
        notebook_ids,
        trash_entries,
        prefs: bundle.prefs.clone().sanitized(),
        manifest: bundle.manifest.clone(),
        slot_ids,
        slot_dirty_hash: bundle.slot_dirty_hash.clone(),
        slot_placeholder_blank: bundle.slot_placeholder_blank.clone(),
        prefs_dirty_pending: bundle.prefs_dirty_pending,
        last_dirty_at_ms: bundle.last_dirty_at_ms,
    };

    let loaded_ws = WebStorageLoadedWorkspace {
        workspace_key,
        meta,
        notebooks: notebooks_map,
        trash: trash_map,
    };
    BundleConversion {
        loaded: loaded_ws,
        notebooks: delta_notebooks,
        trash: delta_trash,
    }
}

fn resolve_active_theme_hint(
    global: &GlobalSyncState,
    workspaces: &[WebStorageLoadedWorkspace],
) -> Option<UiTheme> {
    let active_key = match &global.active_workspace {
        ActiveWorkspaceTarget::Cloud {
            server_url,
            user_id,
        } => cloud_workspace_key(server_url, user_id),
        ActiveWorkspaceTarget::Local => local_workspace_key(&global.active_local_profile_id),
    };
    workspaces
        .iter()
        .find(|ws| ws.workspace_key == active_key)
        .map(|ws| ws.meta.prefs.theme)
}

/// Parses a legacy `localStorage` `papercraft_workspace_v1` JSON blob (`v1`–`v4`) and converts
/// every local profile, cached cloud account, notebook, and trash entry into an IndexedDB-ready
/// loaded state plus an urgent migration delta.
pub fn migrate_legacy_web_cache_json(
    raw_json: &str,
) -> Result<(WebStorageLoadedState, WebStorageDelta), String> {
    let mut cache: WebWorkspaceCache = serde_json::from_str(raw_json)
        .map_err(|err| format!("failed to parse legacy web workspace JSON: {err}"))?;
    if !matches!(cache.version, 1..=4) {
        return Err(format!(
            "unsupported legacy web workspace version {}",
            cache.version
        ));
    }
    sanitize_global_state(&mut cache.global);
    migrate_web_cache_if_needed(&mut cache);
    if !cache.global.needs_initial_nickname || !cache.local_profiles.is_empty() {
        let _ = ensure_default_profile_in_global(&mut cache.global);
    }

    let mut loaded_workspaces =
        Vec::with_capacity(cache.local_profiles.len() + cache.cloud_accounts.len());
    let mut delta = WebStorageDelta {
        global: Some(cache.global.clone()),
        urgent: true,
        ..Default::default()
    };

    for (pid, bundle) in &cache.local_profiles {
        let ws_key = local_workspace_key(pid);
        let conv = build_loaded_workspace_from_bundle(ws_key.clone(), bundle);
        delta
            .put_workspaces
            .insert(ws_key.clone(), conv.loaded.meta.clone());
        for (nb_id, doc) in conv.notebooks {
            delta.put_notebooks.insert((ws_key.clone(), nb_id), doc);
        }
        for (trash_id, deleted_at_ms, doc) in conv.trash {
            delta
                .put_trash
                .insert((ws_key.clone(), trash_id), (deleted_at_ms, doc));
        }
        loaded_workspaces.push(conv.loaded);
    }

    for (acct_key, bundle) in &cache.cloud_accounts {
        let ws_key = format!("cloud:{acct_key}");
        let conv = build_loaded_workspace_from_bundle(ws_key.clone(), bundle);
        delta
            .put_workspaces
            .insert(ws_key.clone(), conv.loaded.meta.clone());
        for (nb_id, doc) in conv.notebooks {
            delta.put_notebooks.insert((ws_key.clone(), nb_id), doc);
        }
        for (trash_id, deleted_at_ms, doc) in conv.trash {
            delta
                .put_trash
                .insert((ws_key.clone(), trash_id), (deleted_at_ms, doc));
        }
        loaded_workspaces.push(conv.loaded);
    }

    delta.theme_hint = resolve_active_theme_hint(&cache.global, &loaded_workspaces);

    Ok((
        WebStorageLoadedState {
            global: cache.global,
            workspaces: loaded_workspaces,
        },
        delta,
    ))
}

#[derive(Clone, Debug, Default)]
pub(crate) struct TrackedWorkspaceState {
    pub(crate) last_meta_json: Option<String>,
    pub(crate) last_structural_json: Option<String>,
    pub(crate) notebooks: HashMap<String, u64>,
    pub(crate) trash_ids: HashSet<String>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct WebPersistTracker {
    pub(crate) hash_cache: BlobHashCache,
    pub(crate) last_global_json: Option<String>,
    pub(crate) last_theme: Option<UiTheme>,
    pub(crate) workspaces: HashMap<String, TrackedWorkspaceState>,
}

pub(crate) fn workspace_bundle_to_loaded_meta(
    ws: &WebStorageLoadedWorkspace,
) -> CachedLibraryBundle {
    let mut notebooks = Vec::with_capacity(ws.meta.notebook_ids.len());
    let mut seen_ids = HashSet::new();
    for nb_id in &ws.meta.notebook_ids {
        if let Some(doc) = ws.notebooks.get(nb_id) {
            notebooks.push(doc.clone());
            seen_ids.insert(nb_id.clone());
        }
    }
    let mut extra_ids: Vec<&String> = ws
        .notebooks
        .keys()
        .filter(|id| !seen_ids.contains(*id))
        .collect();
    extra_ids.sort();
    for nb_id in extra_ids {
        if let Some(doc) = ws.notebooks.get(nb_id) {
            notebooks.push(doc.clone());
        }
    }

    let mut trash = Vec::with_capacity(ws.meta.trash_entries.len());
    let mut seen_trash = HashSet::new();
    for (trash_id, deleted_at_ms) in &ws.meta.trash_entries {
        if let Some((stored_ts, doc)) = ws.trash.get(trash_id) {
            trash.push(CachedTrashEntry {
                trash_id: trash_id.clone(),
                doc: doc.clone(),
                deleted_at_ms: (*stored_ts).max(*deleted_at_ms),
            });
            seen_trash.insert(trash_id.clone());
        }
    }
    let mut extra_trash: Vec<&String> = ws
        .trash
        .keys()
        .filter(|id| !seen_trash.contains(*id))
        .collect();
    extra_trash.sort();
    for trash_id in extra_trash {
        if let Some((deleted_at_ms, doc)) = ws.trash.get(trash_id) {
            trash.push(CachedTrashEntry {
                trash_id: trash_id.clone(),
                doc: doc.clone(),
                deleted_at_ms: *deleted_at_ms,
            });
        }
    }

    CachedLibraryBundle {
        notebooks,
        active_notebook: ws.meta.active_notebook,
        trash,
        prefs: ws.meta.prefs.clone(),
        manifest: ws.meta.manifest.clone(),
        slot_ids: ws.meta.slot_ids.clone(),
        slot_dirty_hash: ws.meta.slot_dirty_hash.clone(),
        slot_placeholder_blank: ws.meta.slot_placeholder_blank.clone(),
        prefs_dirty_pending: ws.meta.prefs_dirty_pending,
        last_dirty_at_ms: ws.meta.last_dirty_at_ms,
    }
}

pub(crate) fn build_tracker_from_loaded_state(state: &WebStorageLoadedState) -> WebPersistTracker {
    let mut tracker = WebPersistTracker {
        last_global_json: serde_json::to_string(&state.global).ok(),
        last_theme: resolve_active_theme_hint(&state.global, &state.workspaces),
        ..Default::default()
    };
    for ws in &state.workspaces {
        let (meta_json, structural_json) =
            match detached_workspace_diff_keys(&ws.meta, &mut tracker.hash_cache) {
                Ok((m, s)) => (Some(m), Some(s)),
                Err(_) => (None, None),
            };
        let notebooks = ws.notebooks.keys().map(|id| (id.clone(), 0u64)).collect();
        let trash_ids = ws.trash.keys().cloned().collect();
        tracker.workspaces.insert(
            ws.workspace_key.clone(),
            TrackedWorkspaceState {
                last_meta_json: meta_json,
                last_structural_json: structural_json,
                notebooks,
                trash_ids,
            },
        );
    }
    tracker
}

impl Session {
    /// Returns the IndexedDB key for the currently loaded workspace, or `None` when signed out
    /// in the transient profile-picker state.
    pub fn active_web_workspace_key(&self) -> Option<String> {
        if !self.sync.web_cache_enabled {
            return None;
        }
        if let Some(ref acct) = self.sync.active_server_session {
            return Some(cloud_workspace_key(&acct.server_url, &acct.user.user_id));
        }
        match self.sync.manifest.owner_id {
            Some(OwnerId::LocalProfile(ref pid)) if !pid.is_empty() => {
                Some(local_workspace_key(pid))
            }
            Some(OwnerId::CloudUser(ref uid)) if !uid.is_empty() => {
                Some(cloud_workspace_key(&self.sync.global.server_url, uid))
            }
            _ => None,
        }
    }

    pub(crate) fn ensure_active_web_ids(&mut self) {
        let count = self.notebooks.len();
        self.sync.slot_ids.resize_with(count, String::new);
        for idx in 0..count {
            let mut id = sanitize_notebook_id(&self.notebooks[idx].id);
            if id.is_empty() {
                id = sanitize_notebook_id(&self.sync.slot_ids[idx]);
            }
            if id.is_empty() {
                id = Uuid::new_v4().to_string();
            }
            if self.notebooks[idx].id != id {
                let mut updated = (*self.notebooks[idx]).clone();
                updated.id = id.clone();
                let arc = Arc::new(updated);
                self.notebooks[idx] = Arc::clone(&arc);
                if idx == self.active_notebook {
                    self.doc = arc;
                }
            }
            if self.sync.slot_ids[idx] != id {
                self.sync.slot_ids[idx] = id;
            }
        }
        for entry in &mut self.trashed_notebooks {
            if entry.trash_id.trim().is_empty() {
                entry.trash_id = Uuid::new_v4().to_string();
            }
            if entry.doc.id.trim().is_empty() {
                let mut updated = (*entry.doc).clone();
                updated.id = Uuid::new_v4().to_string();
                entry.doc = Arc::new(updated);
            }
        }
    }

    fn build_active_web_meta_snapshot(&self) -> WebWorkspaceMetaSnapshot {
        let notebook_ids = self
            .notebooks
            .iter()
            .map(|doc| sanitize_notebook_id(&doc.id))
            .collect();
        let trash_entries = self
            .trashed_notebooks
            .iter()
            .map(|entry| (entry.trash_id.clone(), entry.deleted_at_ms))
            .collect();
        WebWorkspaceMetaSnapshot {
            active_notebook: self.active_notebook,
            notebook_ids,
            trash_entries,
            prefs: self.prefs.clone(),
            manifest: self.sync.manifest.clone(),
            slot_ids: self.sync.slot_ids.clone(),
            slot_dirty_hash: self.sync.slot_dirty_hash.clone(),
            slot_placeholder_blank: self.sync.slot_placeholder_blank.clone(),
            prefs_dirty_pending: self.sync.prefs_dirty_pending,
            last_dirty_at_ms: self.sync.last_dirty_at_ms,
        }
    }

    /// Captures any uncollected mutations in the active workspace into `sync.pending_web_delta`
    /// without serializing unchanged notebooks or inactive workspaces.
    pub fn capture_active_web_delta_into_pending(&mut self, urgent_override: bool) {
        if !self.sync.web_cache_enabled {
            return;
        }
        self.sync_active_notebook_slot();
        self.sync_flush_dirty_flags_to_manifest();
        self.sync_reconcile_folders();
        self.ensure_active_web_ids();

        let mut delta = WebStorageDelta {
            urgent: urgent_override,
            ..Default::default()
        };

        if let Ok(global_json) = serde_json::to_string(&self.sync.global)
            && self.sync.web_tracker.last_global_json.as_deref() != Some(global_json.as_str())
        {
            self.sync.web_tracker.last_global_json = Some(global_json);
            delta.global = Some(self.sync.global.clone());
            delta.urgent = true;
        }

        if let Some(ws_key) = self.active_web_workspace_key() {
            if self.sync.web_tracker.last_theme != Some(self.prefs.theme) {
                self.sync.web_tracker.last_theme = Some(self.prefs.theme);
                delta.theme_hint = Some(self.prefs.theme);
                delta.urgent = true;
            }

            let meta = self.build_active_web_meta_snapshot();
            let tracker = &mut self.sync.web_tracker;
            let diff_keys = detached_workspace_diff_keys(&meta, &mut tracker.hash_cache).ok();
            let tracked_ws = tracker.workspaces.entry(ws_key.clone()).or_default();

            if let Some((meta_json, structural_json)) = diff_keys {
                if tracked_ws.last_meta_json.as_deref() != Some(meta_json.as_str()) {
                    tracked_ws.last_meta_json = Some(meta_json);
                    delta.put_workspaces.insert(ws_key.clone(), meta);
                }
                if tracked_ws.last_structural_json.as_deref() != Some(structural_json.as_str()) {
                    tracked_ws.last_structural_json = Some(structural_json);
                    delta.urgent = true;
                }
            }

            let count = self.notebooks.len();
            self.library.sync_len(count);
            self.sync.slot_edit_gen.resize(count, 0);
            let mut live_nb_ids = HashSet::with_capacity(count);

            for idx in 0..count {
                let mut doc_snapshot = (*self.notebooks[idx]).clone();
                doc_snapshot.sync_active_to_pages();
                let nb_id = sanitize_notebook_id(&doc_snapshot.id);
                let edit_gen = self.sync.slot_edit_gen[idx];
                let slot_dirty = self.library.slot_dirty[idx];
                let tracked_gen = tracked_ws.notebooks.get(&nb_id).copied();
                let is_new = tracked_gen.is_none();
                let changed = slot_dirty || tracked_gen != Some(edit_gen);

                if changed {
                    let doc_arc = Arc::new(doc_snapshot);
                    self.notebooks[idx] = Arc::clone(&doc_arc);
                    if idx == self.active_notebook {
                        self.doc = Arc::clone(&doc_arc);
                    }
                    delta
                        .put_notebooks
                        .insert((ws_key.clone(), nb_id.clone()), doc_arc);
                    self.library.slot_dirty[idx] = false;
                    tracked_ws.notebooks.insert(nb_id.clone(), edit_gen);
                    if is_new {
                        delta.urgent = true;
                    }
                }
                live_nb_ids.insert(nb_id);
            }

            let removed_nb_ids: Vec<String> = tracked_ws
                .notebooks
                .keys()
                .filter(|id| !live_nb_ids.contains(*id))
                .cloned()
                .collect();
            for nb_id in removed_nb_ids {
                tracked_ws.notebooks.remove(&nb_id);
                delta.delete_notebooks.insert((ws_key.clone(), nb_id));
                delta.urgent = true;
            }

            let mut live_trash_ids = HashSet::with_capacity(self.trashed_notebooks.len());
            for entry in &self.trashed_notebooks {
                let tid = entry.trash_id.clone();
                if !tracked_ws.trash_ids.contains(&tid) {
                    let mut doc_snapshot = (*entry.doc).clone();
                    doc_snapshot.sync_active_to_pages();
                    delta.put_trash.insert(
                        (ws_key.clone(), tid.clone()),
                        (entry.deleted_at_ms, Arc::new(doc_snapshot)),
                    );
                    tracked_ws.trash_ids.insert(tid.clone());
                    delta.urgent = true;
                }
                live_trash_ids.insert(tid);
            }

            let removed_trash_ids: Vec<String> = tracked_ws
                .trash_ids
                .iter()
                .filter(|id| !live_trash_ids.contains(*id))
                .cloned()
                .collect();
            for tid in removed_trash_ids {
                tracked_ws.trash_ids.remove(&tid);
                delta.delete_trash.insert((ws_key.clone(), tid));
                delta.urgent = true;
            }
        }

        if !delta.is_empty() {
            self.sync.pending_web_delta.merge(delta);
        }
    }

    /// Returns `true` when any in-memory or queued web storage changes have not yet been drained
    /// via [`Session::take_web_storage_delta`].
    #[must_use]
    pub fn has_pending_web_storage_changes(&self) -> bool {
        if !self.sync.web_cache_enabled {
            return false;
        }
        if !self.sync.pending_web_delta.is_empty() || self.library.has_dirty() {
            return true;
        }
        if let Ok(global_json) = serde_json::to_string(&self.sync.global)
            && self.sync.web_tracker.last_global_json.as_deref() != Some(global_json.as_str())
        {
            return true;
        }
        let Some(ws_key) = self.active_web_workspace_key() else {
            return false;
        };
        if self.sync.web_tracker.last_theme != Some(self.prefs.theme) {
            return true;
        }
        let Some(tracked_ws) = self.sync.web_tracker.workspaces.get(&ws_key) else {
            return true;
        };
        if self.notebooks.len() != tracked_ws.notebooks.len()
            || self.trashed_notebooks.len() != tracked_ws.trash_ids.len()
        {
            return true;
        }
        for (idx, doc) in self.notebooks.iter().enumerate() {
            let nb_id = sanitize_notebook_id(&doc.id);
            let edit_gen = self.sync.slot_edit_gen.get(idx).copied().unwrap_or(0);
            if tracked_ws.notebooks.get(&nb_id).copied() != Some(edit_gen) {
                return true;
            }
        }
        for entry in &self.trashed_notebooks {
            if entry.trash_id.trim().is_empty() || !tracked_ws.trash_ids.contains(&entry.trash_id) {
                return true;
            }
        }
        false
    }

    /// Captures all pending active-workspace mutations and drains the accumulated [`WebStorageDelta`]
    /// for asynchronous IndexedDB persistence.
    pub fn take_web_storage_delta(&mut self, force_urgent: bool) -> WebStorageDelta {
        self.capture_active_web_delta_into_pending(force_urgent);
        std::mem::take(&mut self.sync.pending_web_delta)
    }

    /// Re-queues a failed [`WebStorageDelta`] into the session's pending queue without discarding
    /// unsaved work.
    pub fn requeue_web_storage_delta(&mut self, failed: WebStorageDelta) {
        if !self.sync.web_cache_enabled || failed.is_empty() {
            return;
        }
        let mut combined = failed;
        let current = std::mem::take(&mut self.sync.pending_web_delta);
        combined.merge(current);
        self.sync.pending_web_delta = combined;
    }

    /// Returns the full active-workspace trash retention period in milliseconds (30 days).
    #[must_use]
    pub const fn trash_retention_ms() -> i64 {
        30 * 24 * 60 * 60 * 1000
    }
}

pub(crate) fn trashed_to_cached_entry(entry: &TrashedNotebook) -> CachedTrashEntry {
    CachedTrashEntry {
        trash_id: entry.trash_id.clone(),
        doc: (*entry.doc).clone(),
        deleted_at_ms: entry.deleted_at_ms,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;
    use papercraft_doc::{
        EmbeddedImageTemplate, EmbeddedPdf, InkStroke, PageImage, PaperColor, PaperTemplate,
        PdfPageBackdrop, PenSettings, SavedElement, StrokeSample,
    };
    use papercraft_format::{load_package_bytes, save_package_bytes};
    use papercraft_sync::SyncCycleDriver;
    use papercraft_sync_protocol::{
        RemoteNotebookRecord, SyncPullResponse, UserProfile, canonical_document_body_hash,
        canonical_document_hash, canonical_document_meta_hash,
    };
    use serde_json::json;

    fn sample_pdf_bytes(tag: u8) -> Arc<Vec<u8>> {
        let mut bytes = b"%PDF-1.4\n%".to_vec();
        bytes.resize(32 * 1024, tag);
        bytes.extend_from_slice(b"\n%%EOF\n");
        Arc::new(bytes)
    }

    fn sample_png_bytes(tag: u8) -> Arc<Vec<u8>> {
        let mut bytes = vec![
            137u8, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, b'I', b'H', b'D', b'R',
        ];
        bytes.resize(4096, tag);
        Arc::new(bytes)
    }

    fn build_rich_test_doc(id: &str, title: &str) -> Document {
        let mut doc = Document::new(1000, 1400);
        doc.id = id.to_string();
        doc.title = title.to_string();
        doc.paper_template = PaperTemplate::Grid;
        doc.paper_color = PaperColor::WarmCream;
        let pdf = sample_pdf_bytes(b'P');
        let tpl = sample_png_bytes(b'T');
        let page_img = sample_png_bytes(b'I');
        doc.pdf_documents.push(EmbeddedPdf {
            name: "Lecture.pdf".to_string(),
            bytes: pdf,
        });
        doc.image_templates.push(EmbeddedImageTemplate {
            name: "GridBG".to_string(),
            png_bytes: tpl,
        });
        doc.pdf_backdrop = Some(PdfPageBackdrop {
            pdf_index: 0,
            page_index: 2,
        });
        let stroke_id = doc.alloc_stroke_id();
        let pts = vec![
            StrokeSample::from_xy_pressure(20.0, 30.0, 0.5),
            StrokeSample::from_xy_pressure(140.0, 90.0, 0.8),
        ];
        doc.strokes.push(Arc::new(InkStroke::new(
            stroke_id,
            PenSettings::fountain_pen(),
            pts.clone(),
            pts,
        )));
        doc.images.push(Arc::new(PageImage {
            x: 50.0,
            y: 60.0,
            width: 200.0,
            height: 150.0,
            angle_degrees: 12.0,
            pixel_width: 400,
            pixel_height: 300,
            png_bytes: page_img,
            z_order: 5,
        }));
        doc.sync_active_to_pages();
        doc.add_page(None, None).unwrap();
        doc.custom_image_backdrop = Some(0);
        doc.sync_active_to_pages();
        doc.select_page(0);
        doc
    }

    fn persist_delta_into_fake_idb(
        delta: WebStorageDelta,
        cache: &mut BlobHashCache,
        meta_global: &mut Option<GlobalSyncState>,
        workspaces: &mut HashMap<String, String>,
        notebooks: &mut HashMap<String, Vec<u8>>,
        trash: &mut HashMap<String, (i64, Vec<u8>)>,
        blobs: &mut HashMap<String, Arc<Vec<u8>>>,
    ) {
        if let Some(g) = delta.global {
            *meta_global = Some(g);
        }
        for ws_key in delta.delete_workspaces {
            workspaces.remove(&ws_key);
            let prefix = format!("{ws_key}::");
            notebooks.retain(|k, _| !k.starts_with(&prefix));
            trash.retain(|k, _| !k.starts_with(&prefix));
        }
        let known: HashSet<String> = blobs.keys().cloned().collect();
        for (ws_key, ws_meta) in delta.put_workspaces {
            let (ws_json, new_blobs) = encode_detached_workspace(&ws_meta, cache, &known).unwrap();
            for (h, b) in new_blobs {
                blobs.insert(h, b);
            }
            workspaces.insert(ws_key, ws_json);
        }
        for (ws_key, nb_id) in delta.delete_notebooks {
            notebooks.remove(&scoped_item_key(&ws_key, &nb_id));
        }
        for ((ws_key, nb_id), doc) in delta.put_notebooks {
            let known_now: HashSet<String> = blobs.keys().cloned().collect();
            let (bytes, new_blobs) = encode_detached_notebook(&doc, cache, &known_now).unwrap();
            for (h, b) in new_blobs {
                blobs.insert(h, b);
            }
            notebooks.insert(scoped_item_key(&ws_key, &nb_id), bytes);
        }
        for (ws_key, tid) in delta.delete_trash {
            trash.remove(&scoped_item_key(&ws_key, &tid));
        }
        for ((ws_key, tid), (deleted_at_ms, doc)) in delta.put_trash {
            let known_now: HashSet<String> = blobs.keys().cloned().collect();
            let (bytes, new_blobs) = encode_detached_notebook(&doc, cache, &known_now).unwrap();
            for (h, b) in new_blobs {
                blobs.insert(h, b);
            }
            trash.insert(scoped_item_key(&ws_key, &tid), (deleted_at_ms, bytes));
        }
    }

    fn load_state_from_fake_idb(
        global: &GlobalSyncState,
        workspaces: &HashMap<String, String>,
        notebooks: &HashMap<String, Vec<u8>>,
        trash: &HashMap<String, (i64, Vec<u8>)>,
        blobs: &HashMap<String, Arc<Vec<u8>>>,
    ) -> WebStorageLoadedState {
        let mut loaded_ws = Vec::new();
        for (ws_key, ws_json) in workspaces {
            let meta = decode_detached_workspace(ws_json, blobs).unwrap();
            let prefix = format!("{ws_key}::");
            let mut ws_notebooks = HashMap::new();
            for (k, raw) in notebooks {
                if let Some(nb_id) = k.strip_prefix(&prefix) {
                    ws_notebooks.insert(
                        nb_id.to_string(),
                        decode_detached_notebook(raw, blobs).unwrap(),
                    );
                }
            }
            let mut ws_trash = HashMap::new();
            for (k, (ts, raw)) in trash {
                if let Some(tid) = k.strip_prefix(&prefix) {
                    ws_trash.insert(
                        tid.to_string(),
                        (*ts, decode_detached_notebook(raw, blobs).unwrap()),
                    );
                }
            }
            loaded_ws.push(WebStorageLoadedWorkspace {
                workspace_key: ws_key.clone(),
                meta,
                notebooks: ws_notebooks,
                trash: ws_trash,
            });
        }
        WebStorageLoadedState {
            global: global.clone(),
            workspaces: loaded_ws,
        }
    }

    #[test]
    fn detached_notebook_and_workspace_roundtrip_strips_and_rehydrates_blobs() {
        let doc = build_rich_test_doc("nb-rich", "Calculus PDF");
        let mut cache = BlobHashCache::default();
        let known = HashSet::new();

        let (detached_bytes, blobs_vec) =
            encode_detached_notebook(&doc, &mut cache, &known).unwrap();
        assert!(detached_bytes.starts_with(DETACHED_DOC_MAGIC));
        assert_eq!(blobs_vec.len(), 3);
        assert!(
            detached_bytes.len() < 4096,
            "expected detached PCDB body without PDF/PNGs to be small, got {} bytes",
            detached_bytes.len()
        );

        let extracted = extract_detached_notebook_blob_hashes(&detached_bytes);
        assert_eq!(extracted.len(), 3);
        for (h, _) in &blobs_vec {
            assert!(extracted.contains(h));
        }

        // Second encode with populated known_blobs emits zero new blobs.
        let known2: HashSet<String> = blobs_vec.iter().map(|(h, _)| h.clone()).collect();
        let (_, empty_blobs) = encode_detached_notebook(&doc, &mut cache, &known2).unwrap();
        assert!(empty_blobs.is_empty());

        let blob_map: HashMap<String, Arc<Vec<u8>>> = blobs_vec.into_iter().collect();
        let rehydrated = decode_detached_notebook(&detached_bytes, &blob_map).unwrap();
        assert_eq!(rehydrated.id, doc.id);
        assert_eq!(rehydrated.pdf_documents, doc.pdf_documents);
        assert_eq!(rehydrated.image_templates, doc.image_templates);
        assert_eq!(rehydrated.pages[0].pdf_backdrop, doc.pages[0].pdf_backdrop);
        assert_eq!(
            rehydrated.pages[1].custom_image_backdrop,
            doc.pages[1].custom_image_backdrop
        );
        assert_eq!(
            rehydrated.pages[0].images[0].png_bytes,
            doc.pages[0].images[0].png_bytes
        );
        assert_eq!(rehydrated.pages[0].images[0].z_order, 5);

        // Workspace prefs round-trip (custom templates + saved element images).
        let mut prefs = UserStudioConfig::default();
        prefs.custom_templates.push(CustomPaperTemplate {
            id: "tpl-custom".to_string(),
            name: "Exam Sheet".to_string(),
            kind: CustomTemplateKind::Pdf,
            width: 800,
            height: 1000,
            pdf_page_index: 0,
            bytes: sample_pdf_bytes(b'C'),
        });
        prefs.saved_elements.push(SavedElement {
            id: "el-img".to_string(),
            name: "Logo".to_string(),
            built_in: false,
            width: 100.0,
            height: 100.0,
            images: vec![PageImage {
                png_bytes: sample_png_bytes(b'E'),
                ..PageImage::default()
            }],
            ..SavedElement::default()
        });
        let meta = WebWorkspaceMetaSnapshot {
            notebook_ids: vec!["nb-rich".to_string()],
            prefs: prefs.clone(),
            ..Default::default()
        };
        let (ws_json, ws_blobs) = encode_detached_workspace(&meta, &mut cache, &known).unwrap();
        assert_eq!(ws_blobs.len(), 2);
        let ws_hashes = extract_detached_workspace_blob_hashes(&ws_json);
        assert_eq!(ws_hashes.len(), 2);
        let mut all_blobs = blob_map;
        for (h, b) in ws_blobs {
            all_blobs.insert(h, b);
        }
        let decoded_ws = decode_detached_workspace(&ws_json, &all_blobs).unwrap();
        assert_eq!(decoded_ws.prefs.custom_templates, prefs.custom_templates);
        assert_eq!(
            decoded_ws.prefs.saved_elements.last().unwrap().images[0].png_bytes,
            prefs.saved_elements.last().unwrap().images[0].png_bytes
        );
    }

    #[test]
    fn rehydrated_session_exports_complete_pcraft_pcraftpkg_and_sync_payloads() {
        // Safeguard #2: verify zero BlobRef/PCDB leakage into exports or sync push payloads.
        let mut session = Session::open_web_storage(None);
        session.complete_first_access_nickname("Tester").unwrap();
        let acct = papercraft_sync::ActiveAccountSession {
            server_url: "http://127.0.0.1:8787".to_string(),
            user: UserProfile {
                user_id: "u-blob".to_string(),
                username: "blobuser".to_string(),
                email: Some("blob@example.com".to_string()),
                display_name: "Blob User".to_string(),
                created_at_ms: 100,
                must_change_password: false,
            },
            access_token: "tok".to_string(),
            refresh_token: "ref".to_string(),
            access_expires_at_ms: 9_999_999_999_000,
            refresh_expires_at_ms: 9_999_999_999_000,
            device_name: "Web".to_string(),
            last_auth_ms: 100,
        };
        let doc = build_rich_test_doc("nb-cloud-rich", "Cloud Rich");
        let expected_pdf = doc.pdf_documents[0].bytes.clone();
        let expected_tpl = doc.image_templates[0].png_bytes.clone();
        let expected_img = doc.pages[0].images[0].png_bytes.clone();

        let pull = SyncPullResponse {
            server_cursor: 1,
            user: acct.user.clone(),
            notebooks: vec![RemoteNotebookRecord {
                notebook_id: doc.id.clone(),
                version: 1,
                cursor: 1,
                deleted: false,
                updated_at_ms: 1000,
                deleted_at_ms: None,
                title: doc.title.clone(),
                content_hash: canonical_document_hash(&doc),
                body_hash: canonical_document_body_hash(&doc),
                meta_hash: canonical_document_meta_hash(&doc),
                meta_updated_at_ms: 1000,
                document: Some(doc),
            }],
            folders: Vec::new(),
            studio_prefs: None,
        };
        session.enter_online_server_session(acct, pull);
        session.prefs.custom_templates.push(CustomPaperTemplate {
            id: "tpl-sync".to_string(),
            name: "Custom Sync Template".to_string(),
            kind: CustomTemplateKind::Image,
            width: 800,
            height: 1000,
            pdf_page_index: 0,
            bytes: sample_png_bytes(b'S'),
        });
        session.sync_mark_prefs_dirty();

        let delta = session.take_web_storage_delta(true);
        let mut cache = BlobHashCache::default();
        let mut meta_global = None;
        let mut workspaces = HashMap::new();
        let mut notebooks = HashMap::new();
        let mut trash = HashMap::new();
        let mut blobs = HashMap::new();
        persist_delta_into_fake_idb(
            delta,
            &mut cache,
            &mut meta_global,
            &mut workspaces,
            &mut notebooks,
            &mut trash,
            &mut blobs,
        );

        let loaded = load_state_from_fake_idb(
            meta_global.as_ref().unwrap(),
            &workspaces,
            &notebooks,
            &trash,
            &blobs,
        );
        let mut reopened = Session::open_web_storage(Some(loaded));
        assert!(!reopened.has_pending_web_storage_changes());

        // 1. Standalone .pcraft binary export
        let pcraft_bytes = papercraft_format::save_document_bytes(&reopened.doc).unwrap();
        assert!(!pcraft_bytes.starts_with(DETACHED_DOC_MAGIC));
        let decoded_pcraft = load_document_bytes(&pcraft_bytes).unwrap();
        assert_eq!(decoded_pcraft.pdf_documents[0].bytes, expected_pdf);
        assert_eq!(decoded_pcraft.image_templates[0].png_bytes, expected_tpl);
        assert_eq!(decoded_pcraft.pages[0].images[0].png_bytes, expected_img);

        // 2. Complete .pcraftpkg export
        let pkg_bytes = reopened.export_package_bytes().unwrap();
        let decoded_pkg = load_package_bytes(&pkg_bytes).unwrap();
        assert_eq!(
            decoded_pkg.notebooks[0].pdf_documents[0].bytes,
            expected_pdf
        );
        assert_eq!(
            decoded_pkg.prefs.custom_templates[0].bytes,
            sample_png_bytes(b'S')
        );
        let roundtrip_pkg = save_package_bytes(&decoded_pkg).unwrap();
        assert!(!roundtrip_pkg.is_empty());

        // 3. Sync push payload after a local stroke
        reopened
            .execute(
                "stroke.draw",
                &json!({
                    "points": [
                        { "x": 10.0, "y": 10.0, "pressure": 0.5 },
                        { "x": 40.0, "y": 40.0, "pressure": 0.7 }
                    ]
                }),
            )
            .unwrap();
        let batch = reopened.prepare_sync_batch().unwrap();
        let driver = SyncCycleDriver::new(&batch);
        let push_req = driver.next_push_request(&batch).unwrap();
        assert_eq!(push_req.changes.len(), 1);
        let pushed_doc = push_req.changes[0].document.as_ref().unwrap();
        assert_eq!(pushed_doc.pdf_documents[0].bytes, expected_pdf);
        assert_eq!(pushed_doc.image_templates[0].png_bytes, expected_tpl);
        assert_eq!(pushed_doc.pages[0].images[0].png_bytes, expected_img);
    }

    #[test]
    fn granular_delta_emits_only_modified_notebook_and_tracks_trash_and_deletions() {
        let mut session = Session::open_web_storage(None);
        session.complete_first_access_nickname("DeltaUser").unwrap();

        let mut cache = BlobHashCache::default();
        let mut meta_global = None;
        let mut workspaces = HashMap::new();
        let mut notebooks = HashMap::new();
        let mut trash = HashMap::new();
        let mut blobs = HashMap::new();

        let initial = session.take_web_storage_delta(false);
        assert!(!initial.is_empty());
        assert_eq!(initial.put_notebooks.len(), 6);
        persist_delta_into_fake_idb(
            initial,
            &mut cache,
            &mut meta_global,
            &mut workspaces,
            &mut notebooks,
            &mut trash,
            &mut blobs,
        );
        assert!(!session.has_pending_web_storage_changes());

        // Draw a stroke in notebook 0 -> ONLY notebook 0 is emitted and delta.urgent is false!
        session
            .execute(
                "stroke.draw",
                &json!({
                    "points": [
                        { "x": 12.0, "y": 14.0, "pressure": 0.6 },
                        { "x": 88.0, "y": 64.0, "pressure": 0.7 }
                    ]
                }),
            )
            .unwrap();
        assert!(session.has_pending_web_storage_changes());
        let stroke_delta = session.take_web_storage_delta(false);
        assert_eq!(stroke_delta.put_notebooks.len(), 1);
        assert!(!stroke_delta.urgent);
        persist_delta_into_fake_idb(
            stroke_delta,
            &mut cache,
            &mut meta_global,
            &mut workspaces,
            &mut notebooks,
            &mut trash,
            &mut blobs,
        );
        assert!(!session.has_pending_web_storage_changes());

        // Move notebook 0 to Trash -> delete_notebooks + put_trash.
        let deleted_nb_id = session.notebooks[0].id.clone();
        session
            .execute("notebook.delete", &json!({ "index": 0 }))
            .unwrap();
        let delete_delta = session.take_web_storage_delta(false);
        assert!(delete_delta.urgent);
        assert_eq!(delete_delta.delete_notebooks.len(), 1);
        assert!(
            delete_delta
                .delete_notebooks
                .iter()
                .any(|(_, id)| id == &deleted_nb_id)
        );
        assert_eq!(delete_delta.put_trash.len(), 1);
        persist_delta_into_fake_idb(
            delete_delta,
            &mut cache,
            &mut meta_global,
            &mut workspaces,
            &mut notebooks,
            &mut trash,
            &mut blobs,
        );

        // Restore from Trash -> delete_trash + put_notebooks.
        session
            .execute("notebook.restore", &json!({ "index": 0 }))
            .unwrap();
        let restore_delta = session.take_web_storage_delta(false);
        assert!(restore_delta.urgent);
        assert_eq!(restore_delta.delete_trash.len(), 1);
        assert_eq!(restore_delta.put_notebooks.len(), 1);
        persist_delta_into_fake_idb(
            restore_delta,
            &mut cache,
            &mut meta_global,
            &mut workspaces,
            &mut notebooks,
            &mut trash,
            &mut blobs,
        );

        // Create second local profile, switch to it, then delete the first profile.
        let first_pid = session
            .active_local_profile()
            .unwrap()
            .local_profile_id
            .clone();
        let second = session.create_local_profile("Second", true).unwrap();
        assert_ne!(first_pid, second.local_profile_id);
        let switch_delta = session.take_web_storage_delta(false);
        persist_delta_into_fake_idb(
            switch_delta,
            &mut cache,
            &mut meta_global,
            &mut workspaces,
            &mut notebooks,
            &mut trash,
            &mut blobs,
        );
        session.delete_local_profile(&first_pid).unwrap();
        let del_prof_delta = session.take_web_storage_delta(false);
        assert!(
            del_prof_delta
                .delete_workspaces
                .contains(&local_workspace_key(&first_pid))
        );
    }

    #[test]
    fn legacy_web_cache_migration_preserves_profiles_cloud_trash_and_theme() {
        let mut legacy = Session::open_web_cache(None);
        legacy.complete_first_access_nickname("LegacyUser").unwrap();
        legacy
            .execute("theme.set", &json!({ "theme": "Dark" }))
            .unwrap();
        legacy
            .execute(
                "stroke.draw",
                &json!({
                    "points": [
                        { "x": 10.0, "y": 20.0, "pressure": 0.5 },
                        { "x": 50.0, "y": 60.0, "pressure": 0.8 }
                    ]
                }),
            )
            .unwrap();
        legacy
            .execute("notebook.delete", &json!({ "index": 1 }))
            .unwrap();
        let raw_json = legacy.export_web_cache_json().unwrap();

        let (loaded, delta) = migrate_legacy_web_cache_json(&raw_json).unwrap();
        assert!(delta.urgent);
        assert_eq!(delta.theme_hint, Some(UiTheme::Dark));
        assert_eq!(loaded.workspaces.len(), 1);
        assert_eq!(loaded.workspaces[0].meta.prefs.theme, UiTheme::Dark);
        assert_eq!(loaded.workspaces[0].notebooks.len(), 5);
        assert_eq!(loaded.workspaces[0].trash.len(), 1);

        let reopened = Session::open_web_storage(Some(loaded));
        assert_eq!(reopened.prefs.theme, UiTheme::Dark);
        assert_eq!(reopened.notebooks.len(), 5);
        assert_eq!(reopened.trashed_notebooks.len(), 1);
        assert!(!reopened.notebooks[0].strokes.is_empty());
    }
}
