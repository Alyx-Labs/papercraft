//! Deterministic 3-way document, page, and element-level merge engine shared by
//! Cloud Sync (Desktop & Web) and live cross-tab IndexedDB synchronization.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use papercraft_doc::{
    Document, EmbeddedImageTemplate, EmbeddedPdf, GeometricShape, InkStroke,
    MAX_DOCUMENT_IMAGE_TEMPLATE_BYTES, MAX_DOCUMENT_IMAGE_TEMPLATES, NotebookPage, PageImage,
    StrokeId, TextItem, sanitize_notebook_title,
};
use papercraft_sync_protocol::{
    canonical_document_body_hash, canonical_document_hash, canonical_page_hash, now_unix_ms,
    sha256_hex,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const SHORT_HASH_HEX_LEN: usize = 32;
const SAME_ELEMENT_OVERLAP_MARGIN_PT: f64 = 12.0;

/// Compact per-page structural and element-level fingerprints used as a 3-way merge ancestor.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageElementFingerprints {
    pub page_props_hash: String,
    #[serde(default)]
    pub bookmarked: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub strokes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub texts: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shapes: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<String>,
}

/// Common-ancestor metadata carried alongside dirty notebooks for 3-way merge and ACK recovery.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentMergeBase {
    #[serde(default)]
    pub page_hashes: Vec<String>,
    #[serde(default)]
    pub page_elements: Vec<PageElementFingerprints>,
    #[serde(default)]
    pub pending_push_hash: Option<String>,
    #[serde(default)]
    pub pending_push_page_hashes: Vec<String>,
    #[serde(default)]
    pub pending_push_page_elements: Vec<PageElementFingerprints>,
}

/// Strategy used when reconciling two diverged document revisions.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MergeKind {
    MetadataOnly,
    RebasedPendingPush,
    PageLevel,
    ElementLevel,
}

/// Result of attempting a deterministic 3-way merge between `local` and `remote`.
#[derive(Clone, Debug, PartialEq)]
pub enum MergeDecision {
    /// Both documents already contain identical content; timestamps were reconciled.
    FastForward(Document),
    /// Both documents merged cleanly without losing any work.
    Merged { doc: Document, kind: MergeKind },
    /// Irreconcilable concurrent edits on the same element or page properties.
    Conflict,
}

fn short_digest(hasher: Sha256) -> String {
    let full = sha256_hex(&hasher.finalize());
    full.chars().take(SHORT_HASH_HEX_LEN).collect()
}

fn hash_len_slice(hasher: &mut Sha256, bytes: &[u8]) {
    Digest::update(hasher, (bytes.len() as u64).to_le_bytes());
    Digest::update(hasher, bytes);
}

fn canonical_page_props_hash(page: &NotebookPage) -> String {
    let mut hasher = Sha256::new();
    Digest::update(&mut hasher, b"papercraft-page-props-v1");
    Digest::update(&mut hasher, page.width.clamp(64, 8192).to_le_bytes());
    Digest::update(&mut hasher, page.height.clamp(64, 8192).to_le_bytes());
    if let Ok(tpl) = serde_json::to_vec(&page.paper_template) {
        hash_len_slice(&mut hasher, &tpl);
    }
    if let Ok(col) = serde_json::to_vec(&page.paper_color) {
        hash_len_slice(&mut hasher, &col);
    }
    match page.pdf_backdrop {
        Some(b) => {
            Digest::update(&mut hasher, [1u8]);
            Digest::update(&mut hasher, (b.pdf_index as u64).to_le_bytes());
            Digest::update(&mut hasher, (b.page_index as u64).to_le_bytes());
        }
        None => Digest::update(&mut hasher, [0u8]),
    }
    Digest::update(&mut hasher, [page.backdrop_rotation % 4]);
    match page.custom_image_backdrop {
        Some(idx) => {
            Digest::update(&mut hasher, [1u8]);
            Digest::update(&mut hasher, (idx as u64).to_le_bytes());
        }
        None => Digest::update(&mut hasher, [0u8]),
    }
    match page.custom_template_id.as_deref() {
        Some(id) => {
            Digest::update(&mut hasher, [1u8]);
            hash_len_slice(&mut hasher, id.as_bytes());
        }
        None => Digest::update(&mut hasher, [0u8]),
    }
    short_digest(hasher)
}

fn canonical_stroke_element_hash(stroke: &InkStroke) -> String {
    let mut hasher = Sha256::new();
    Digest::update(&mut hasher, b"papercraft-el-stroke-v1");
    if let Ok(pen_bytes) = serde_json::to_vec(&stroke.pen.sanitized()) {
        hash_len_slice(&mut hasher, &pen_bytes);
    }
    Digest::update(
        &mut hasher,
        (stroke.smoothed_points.len() as u64).to_le_bytes(),
    );
    for sample in stroke.smoothed_points.iter() {
        let q = sample.quantize();
        Digest::update(&mut hasher, q.x_centipx.to_le_bytes());
        Digest::update(&mut hasher, q.y_centipx.to_le_bytes());
        Digest::update(&mut hasher, q.pressure_q16.to_le_bytes());
        Digest::update(&mut hasher, q.tilt_x_cdeg.to_le_bytes());
        Digest::update(&mut hasher, q.tilt_y_cdeg.to_le_bytes());
        Digest::update(&mut hasher, q.rot_cdeg.to_le_bytes());
        Digest::update(&mut hasher, q.time_ms.to_le_bytes());
    }
    short_digest(hasher)
}

fn canonical_text_element_hash(text: &TextItem) -> String {
    let mut hasher = Sha256::new();
    Digest::update(&mut hasher, b"papercraft-el-text-v1");
    let mut clean = text.sanitized();
    clean.z_order = 0;
    if let Ok(bytes) = serde_json::to_vec(&clean) {
        hash_len_slice(&mut hasher, &bytes);
    }
    short_digest(hasher)
}

fn canonical_shape_element_hash(shape: &GeometricShape) -> String {
    let mut hasher = Sha256::new();
    Digest::update(&mut hasher, b"papercraft-el-shape-v1");
    let mut clean = shape.sanitized();
    clean.z_order = 0;
    if let Ok(bytes) = serde_json::to_vec(&clean) {
        hash_len_slice(&mut hasher, &bytes);
    }
    short_digest(hasher)
}

fn canonical_image_element_hash(img: &PageImage) -> String {
    let mut hasher = Sha256::new();
    Digest::update(&mut hasher, b"papercraft-el-image-v1");
    let clean = img.sanitized();
    Digest::update(&mut hasher, clean.x.to_bits().to_le_bytes());
    Digest::update(&mut hasher, clean.y.to_bits().to_le_bytes());
    Digest::update(&mut hasher, clean.width.to_bits().to_le_bytes());
    Digest::update(&mut hasher, clean.height.to_bits().to_le_bytes());
    Digest::update(&mut hasher, clean.angle_degrees.to_bits().to_le_bytes());
    Digest::update(&mut hasher, clean.pixel_width.to_le_bytes());
    Digest::update(&mut hasher, clean.pixel_height.to_le_bytes());
    hash_len_slice(&mut hasher, &clean.png_bytes);
    short_digest(hasher)
}

/// Computes compact element-level fingerprints for a single resolved [`NotebookPage`].
#[must_use]
pub fn compute_page_elements(page: &NotebookPage) -> PageElementFingerprints {
    PageElementFingerprints {
        page_props_hash: canonical_page_props_hash(page),
        bookmarked: page.bookmarked,
        strokes: page
            .strokes
            .iter()
            .map(|s| canonical_stroke_element_hash(s))
            .collect(),
        texts: page
            .texts
            .iter()
            .map(|t| canonical_text_element_hash(t))
            .collect(),
        shapes: page
            .shapes
            .iter()
            .map(|s| canonical_shape_element_hash(s))
            .collect(),
        images: page
            .images
            .iter()
            .map(|im| canonical_image_element_hash(im))
            .collect(),
    }
}

/// Computes element-level fingerprints across every resolved page of `doc`.
#[must_use]
pub fn compute_document_page_elements(doc: &Document) -> Vec<PageElementFingerprints> {
    let count = doc.page_count();
    let mut out = Vec::with_capacity(count);
    for idx in 0..count {
        if let Some(fp) = doc.with_resolved_page(idx, compute_page_elements) {
            out.push(fp);
        }
    }
    out
}

/// Strips any trailing `" (Conflict Copy)"` or `" (Conflict Copy N)"` suffixes so conflict copies
/// never cascade into `"Title (Conflict Copy) (Conflict Copy)"`.
#[must_use]
pub fn strip_conflict_copy_suffix(title: &str) -> String {
    let mut cur = sanitize_notebook_title(title);
    for _ in 0..16 {
        let trimmed = cur.trim_end();
        let lower = trimmed.to_ascii_lowercase();
        if let Some(prefix) = trimmed
            .strip_suffix(")")
            .and_then(|_| lower.rfind(" (conflict copy"))
        {
            let tail = &lower[prefix..];
            let inner = tail
                .trim_start_matches(" (conflict copy")
                .trim_end_matches(')');
            let valid_suffix = inner.is_empty()
                || (inner.starts_with(' ')
                    && inner.trim().chars().all(|c| c.is_ascii_digit())
                    && !inner.trim().is_empty());
            if valid_suffix {
                let next = trimmed[..prefix].trim_end();
                if !next.is_empty() {
                    cur = next.to_string();
                    continue;
                }
            }
        }
        break;
    }
    sanitize_notebook_title(&cur)
}

fn reconcile_timestamps_into(target: &mut Document, a: &Document, b: &Document) {
    target.created_at_ms = match (a.created_at_ms, b.created_at_ms) {
        (0, x) | (x, 0) => x,
        (x, y) => x.min(y),
    };
    target.updated_at_ms = a.updated_at_ms.max(b.updated_at_ms);
    target.meta_updated_at_ms = a.meta_updated_at_ms.max(b.meta_updated_at_ms);
}

fn apply_lww_metadata(target: &mut Document, local: &Document, remote: &Document) {
    let winner = if local.meta_updated_at_ms > remote.meta_updated_at_ms {
        local
    } else {
        remote
    };
    target.title = winner.title.clone();
    target.cover = winner.cover;
    target.folder_path = winner.folder_path.clone();
    target.favorite = winner.favorite;
    target.tags = winner.tags.clone();
    target.meta_updated_at_ms = local.meta_updated_at_ms.max(remote.meta_updated_at_ms);
}

fn attachments_match(local: &Document, remote: &Document) -> bool {
    if local.pdf_documents.len() != remote.pdf_documents.len()
        || local.image_templates.len() != remote.image_templates.len()
    {
        return false;
    }
    local
        .pdf_documents
        .iter()
        .zip(&remote.pdf_documents)
        .all(|(l, r)| l.name == r.name && l.bytes == r.bytes)
        && local
            .image_templates
            .iter()
            .zip(&remote.image_templates)
            .all(|(l, r)| l.name == r.name && l.png_bytes == r.png_bytes)
}

struct MergedAttachments {
    pdfs: Vec<EmbeddedPdf>,
    templates: Vec<EmbeddedImageTemplate>,
    local_pdf_remap: Vec<Option<usize>>,
    local_tpl_remap: Vec<Option<usize>>,
}

fn merge_attachments(local: &Document, remote: &Document) -> Option<MergedAttachments> {
    let mut pdfs: Vec<EmbeddedPdf> = remote
        .pdf_documents
        .iter()
        .map(EmbeddedPdf::sanitized)
        .filter(|p| !p.bytes.is_empty())
        .collect();
    let mut local_pdf_remap = Vec::with_capacity(local.pdf_documents.len());
    for raw_pdf in &local.pdf_documents {
        let clean = raw_pdf.sanitized();
        if clean.bytes.is_empty() {
            local_pdf_remap.push(None);
            continue;
        }
        if let Some(idx) = pdfs
            .iter()
            .position(|existing| existing.name == clean.name && existing.bytes == clean.bytes)
        {
            local_pdf_remap.push(Some(idx));
        } else {
            let idx = pdfs.len();
            pdfs.push(clean);
            local_pdf_remap.push(Some(idx));
        }
    }

    let mut templates: Vec<EmbeddedImageTemplate> = remote
        .image_templates
        .iter()
        .map(EmbeddedImageTemplate::sanitized)
        .filter(|t| !t.png_bytes.is_empty())
        .collect();
    let mut total_tpl_bytes: usize = templates.iter().map(|t| t.png_bytes.len()).sum();
    let mut local_tpl_remap = Vec::with_capacity(local.image_templates.len());
    for raw_tpl in &local.image_templates {
        let clean = raw_tpl.sanitized();
        if clean.png_bytes.is_empty() {
            local_tpl_remap.push(None);
            continue;
        }
        if let Some(idx) = templates.iter().position(|existing| {
            existing.name == clean.name && existing.png_bytes == clean.png_bytes
        }) {
            local_tpl_remap.push(Some(idx));
        } else {
            if templates.len() >= MAX_DOCUMENT_IMAGE_TEMPLATES
                || total_tpl_bytes.saturating_add(clean.png_bytes.len())
                    > MAX_DOCUMENT_IMAGE_TEMPLATE_BYTES
            {
                return None;
            }
            total_tpl_bytes = total_tpl_bytes.saturating_add(clean.png_bytes.len());
            let idx = templates.len();
            templates.push(clean);
            local_tpl_remap.push(Some(idx));
        }
    }

    Some(MergedAttachments {
        pdfs,
        templates,
        local_pdf_remap,
        local_tpl_remap,
    })
}

fn remap_page_attachments(
    page: &mut NotebookPage,
    pdf_remap: &[Option<usize>],
    tpl_remap: &[Option<usize>],
) {
    if let Some(ref mut backdrop) = page.pdf_backdrop {
        match pdf_remap.get(backdrop.pdf_index).copied().flatten() {
            Some(new_idx) => backdrop.pdf_index = new_idx,
            None => page.pdf_backdrop = None,
        }
    }
    if let Some(old_tpl) = page.custom_image_backdrop {
        page.custom_image_backdrop = tpl_remap.get(old_tpl).copied().flatten();
    }
}

fn multiset_counts(hashes: &[String]) -> HashMap<&str, usize> {
    let mut counts = HashMap::with_capacity(hashes.len());
    for h in hashes {
        *counts.entry(h.as_str()).or_insert(0) += 1;
    }
    counts
}

fn is_hash_multiset_subset(sub: &[String], sup: &[String]) -> bool {
    if sub.len() > sup.len() {
        return false;
    }
    let sub_counts = multiset_counts(sub);
    let sup_counts = multiset_counts(sup);
    sub_counts
        .iter()
        .all(|(h, &cnt)| sup_counts.get(h).copied().unwrap_or(0) >= cnt)
}

fn is_page_element_subset(sub: &PageElementFingerprints, sup: &PageElementFingerprints) -> bool {
    sub.page_props_hash == sup.page_props_hash
        && is_hash_multiset_subset(&sub.strokes, &sup.strokes)
        && is_hash_multiset_subset(&sub.texts, &sup.texts)
        && is_hash_multiset_subset(&sub.shapes, &sup.shapes)
        && is_hash_multiset_subset(&sub.images, &sup.images)
}

fn is_document_element_subset(sub: &Document, sup: &Document) -> bool {
    if !attachments_match(sub, sup) {
        return false;
    }
    let sub_pages = sub.resolved_pages();
    let sup_pages = sup.resolved_pages();
    if sub_pages.len() > sup_pages.len() {
        return false;
    }
    for (sub_p, sup_p) in sub_pages.iter().zip(&sup_pages) {
        let sub_fp = compute_page_elements(sub_p);
        let sup_fp = compute_page_elements(sup_p);
        if !is_page_element_subset(&sub_fp, &sup_fp) {
            return false;
        }
    }
    true
}

fn is_ancestor_subset_of_remote(
    ancestor: &[PageElementFingerprints],
    remote: &[PageElementFingerprints],
) -> bool {
    if ancestor.len() > remote.len() {
        return false;
    }
    ancestor
        .iter()
        .zip(remote)
        .all(|(a, r)| is_page_element_subset(a, r))
}

#[derive(Clone, Copy)]
struct ElementBounds {
    min_x: f64,
    min_y: f64,
    max_x: f64,
    max_y: f64,
}

impl ElementBounds {
    fn from_xywh(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self {
            min_x: x.min(x + w),
            min_y: y.min(y + h),
            max_x: x.max(x + w),
            max_y: y.max(y + h),
        }
    }

    fn overlaps(self, other: Self, margin: f64) -> bool {
        self.min_x - margin <= other.max_x
            && self.max_x + margin >= other.min_x
            && self.min_y - margin <= other.max_y
            && self.max_y + margin >= other.min_y
    }
}

fn text_bounds(text: &TextItem) -> ElementBounds {
    let width = f64::from(text.max_width.unwrap_or(360.0).clamp(40.0, 2000.0));
    let line_count = text.text.lines().count().max(1) as f64;
    let height = (f64::from(text.size.clamp(8.0, 200.0))
        * f64::from(text.line_spacing.clamp(0.8, 3.0))
        * line_count)
        .max(32.0);
    ElementBounds::from_xywh(text.x, text.y, width, height)
}

fn shape_bounds(shape: &GeometricShape) -> ElementBounds {
    if !shape.points.is_empty() {
        let mut min_x = f64::INFINITY;
        let mut min_y = f64::INFINITY;
        let mut max_x = f64::NEG_INFINITY;
        let mut max_y = f64::NEG_INFINITY;
        for pt in &shape.points {
            if pt.x.is_finite() && pt.y.is_finite() {
                min_x = min_x.min(pt.x);
                min_y = min_y.min(pt.y);
                max_x = max_x.max(pt.x);
                max_y = max_y.max(pt.y);
            }
        }
        if min_x.is_finite() {
            return ElementBounds {
                min_x,
                min_y,
                max_x,
                max_y,
            };
        }
    }
    ElementBounds {
        min_x: shape.x1.min(shape.x2),
        min_y: shape.y1.min(shape.y2),
        max_x: shape.x1.max(shape.x2),
        max_y: shape.y1.max(shape.y2),
    }
}

fn image_bounds(image: &PageImage) -> ElementBounds {
    ElementBounds::from_xywh(image.x, image.y, image.width, image.height)
}

struct MergedList<T> {
    items: Vec<T>,
    has_conflict: bool,
}

fn merge_element_list<T: Clone, B>(
    base_hashes: &[String],
    local_items: &[T],
    local_hashes: &[String],
    remote_items: &[T],
    remote_hashes: &[String],
    bounds_fn: Option<B>,
) -> MergedList<T>
where
    B: Fn(&T) -> ElementBounds,
{
    let base_counts = multiset_counts(base_hashes);
    let local_counts = multiset_counts(local_hashes);
    let remote_counts = multiset_counts(remote_hashes);

    // Check if both sides deleted/replaced a common base item.
    let mut both_removed_base = false;
    for (&h, &b_cnt) in &base_counts {
        let l_cnt = local_counts.get(h).copied().unwrap_or(0);
        let r_cnt = remote_counts.get(h).copied().unwrap_or(0);
        if l_cnt < b_cnt && r_cnt < b_cnt {
            both_removed_base = true;
            break;
        }
    }

    // Surviving base items: keep min(l_cnt, r_cnt) copies per base hash (remote order first).
    let mut target_surviving: HashMap<&str, usize> = HashMap::new();
    for (&h, &b_cnt) in &base_counts {
        let l_cnt = local_counts.get(h).copied().unwrap_or(0);
        let r_cnt = remote_counts.get(h).copied().unwrap_or(0);
        let keep = b_cnt.min(l_cnt).min(r_cnt);
        if keep > 0 {
            target_surviving.insert(h, keep);
        }
    }

    // Track consumed counts as we scan remote & local so additions are exact.
    let mut remote_seen: HashMap<&str, usize> = HashMap::new();
    let mut surviving_kept: HashMap<&str, usize> = HashMap::new();
    let mut out = Vec::with_capacity(local_items.len().max(remote_items.len()));
    let mut remote_added = Vec::new();

    for (item, hash) in remote_items.iter().zip(remote_hashes) {
        let h = hash.as_str();
        let idx_in_remote = {
            let entry = remote_seen.entry(h).or_insert(0);
            let cur = *entry;
            *entry += 1;
            cur
        };
        let b_cnt = base_counts.get(h).copied().unwrap_or(0);
        let keep_target = target_surviving.get(h).copied().unwrap_or(0);
        let kept_so_far = surviving_kept.get(h).copied().unwrap_or(0);
        if idx_in_remote < b_cnt {
            if kept_so_far < keep_target {
                surviving_kept.insert(h, kept_so_far + 1);
                out.push(item.clone());
            }
        } else {
            remote_added.push(item.clone());
        }
    }

    let mut local_seen: HashMap<&str, usize> = HashMap::new();
    let mut local_added = Vec::new();
    for (item, hash) in local_items.iter().zip(local_hashes) {
        let h = hash.as_str();
        let idx_in_local = {
            let entry = local_seen.entry(h).or_insert(0);
            let cur = *entry;
            *entry += 1;
            cur
        };
        let b_cnt = base_counts.get(h).copied().unwrap_or(0);
        if idx_in_local >= b_cnt {
            local_added.push(item.clone());
        }
    }

    // For positioned items (text boxes, shapes, images), detect overlapping concurrent edits
    // on a modified/deleted base item so we never stack conflicting text boxes over each other.
    if both_removed_base
        && !local_added.is_empty()
        && !remote_added.is_empty()
        && let Some(ref get_bounds) = bounds_fn
    {
        for l_item in &local_added {
            let lb = get_bounds(l_item);
            for r_item in &remote_added {
                let rb = get_bounds(r_item);
                if lb.overlaps(rb, SAME_ELEMENT_OVERLAP_MARGIN_PT) {
                    return MergedList {
                        items: Vec::new(),
                        has_conflict: true,
                    };
                }
            }
        }
    }

    out.extend(remote_added);
    out.extend(local_added);
    MergedList {
        items: out,
        has_conflict: false,
    }
}

fn bump_new_item_z_orders(page: &mut NotebookPage, base_count: (usize, usize, usize, usize)) {
    let mut next_z = page.max_z_order().saturating_add(1).max(1);
    for stroke_arc in page.strokes.iter_mut().skip(base_count.0) {
        if stroke_arc.z_order == 0 || stroke_arc.z_order < next_z {
            Arc::make_mut(stroke_arc).z_order = next_z;
            next_z = next_z.saturating_add(1);
        }
    }
    for shape_arc in page.shapes.iter_mut().skip(base_count.1) {
        if shape_arc.z_order == 0 || shape_arc.z_order < next_z {
            Arc::make_mut(shape_arc).z_order = next_z;
            next_z = next_z.saturating_add(1);
        }
    }
    for text_arc in page.texts.iter_mut().skip(base_count.2) {
        if text_arc.z_order == 0 || text_arc.z_order < next_z {
            Arc::make_mut(text_arc).z_order = next_z;
            next_z = next_z.saturating_add(1);
        }
    }
    for img_arc in page.images.iter_mut().skip(base_count.3) {
        if img_arc.z_order == 0 || img_arc.z_order < next_z {
            Arc::make_mut(img_arc).z_order = next_z;
            next_z = next_z.saturating_add(1);
        }
    }
}

fn try_merge_single_page(
    local_page: &NotebookPage,
    remote_page: &NotebookPage,
    base_fp: &PageElementFingerprints,
) -> Option<NotebookPage> {
    let local_fp = compute_page_elements(local_page);
    let remote_fp = compute_page_elements(remote_page);

    // Reconcile page structural properties (template, color, backdrop, dimensions).
    let l_props_changed = local_fp.page_props_hash != base_fp.page_props_hash;
    let r_props_changed = remote_fp.page_props_hash != base_fp.page_props_hash;
    if l_props_changed && r_props_changed && local_fp.page_props_hash != remote_fp.page_props_hash {
        return None;
    }
    let props_source = if l_props_changed {
        local_page
    } else {
        remote_page
    };

    // Reconcile bookmark flag via 3-way toggle.
    let bookmarked = if local_page.bookmarked != base_fp.bookmarked {
        local_page.bookmarked
    } else {
        remote_page.bookmarked
    };

    let strokes = merge_element_list(
        &base_fp.strokes,
        &local_page.strokes,
        &local_fp.strokes,
        &remote_page.strokes,
        &remote_fp.strokes,
        None::<fn(&Arc<InkStroke>) -> ElementBounds>,
    );
    if strokes.has_conflict {
        return None;
    }

    let texts = merge_element_list(
        &base_fp.texts,
        &local_page.texts,
        &local_fp.texts,
        &remote_page.texts,
        &remote_fp.texts,
        Some(|t: &Arc<TextItem>| text_bounds(t)),
    );
    if texts.has_conflict {
        return None;
    }

    let shapes = merge_element_list(
        &base_fp.shapes,
        &local_page.shapes,
        &local_fp.shapes,
        &remote_page.shapes,
        &remote_fp.shapes,
        Some(|s: &Arc<GeometricShape>| shape_bounds(s)),
    );
    if shapes.has_conflict {
        return None;
    }

    let images = merge_element_list(
        &base_fp.images,
        &local_page.images,
        &local_fp.images,
        &remote_page.images,
        &remote_fp.images,
        Some(|im: &Arc<PageImage>| image_bounds(im)),
    );
    if images.has_conflict {
        return None;
    }

    let mut merged = NotebookPage {
        width: props_source.width.clamp(64, 8192),
        height: props_source.height.clamp(64, 8192),
        paper_template: props_source.paper_template,
        paper_color: props_source.paper_color,
        strokes: strokes.items,
        texts: texts.items,
        shapes: shapes.items,
        images: images.items,
        pdf_backdrop: props_source.pdf_backdrop,
        bookmarked,
        backdrop_rotation: props_source.backdrop_rotation % 4,
        custom_image_backdrop: props_source.custom_image_backdrop,
        custom_template_id: props_source.custom_template_id.clone(),
    };
    let base_counts = (
        base_fp.strokes.len().min(merged.strokes.len()),
        base_fp.shapes.len().min(merged.shapes.len()),
        base_fp.texts.len().min(merged.texts.len()),
        base_fp.images.len().min(merged.images.len()),
    );
    bump_new_item_z_orders(&mut merged, base_counts);
    Some(merged)
}

fn deduplicate_stroke_ids(doc: &mut Document, min_next_id: u64) {
    let mut used = HashSet::new();
    let mut max_seen = 0u64;
    for page in &doc.pages {
        for stroke in &page.strokes {
            max_seen = max_seen.max(stroke.id.0);
        }
    }
    let mut next_id = min_next_id.max(max_seen.saturating_add(1)).max(1);
    for page in &mut doc.pages {
        for stroke_arc in &mut page.strokes {
            let sid = stroke_arc.id.0;
            if sid == 0 || !used.insert(sid) {
                let assigned = next_id;
                next_id = next_id.saturating_add(1);
                used.insert(assigned);
                Arc::make_mut(stroke_arc).id = StrokeId(assigned);
            }
        }
    }
    doc.next_stroke_id = next_id;
}

struct MergedPagesResult {
    pages: Vec<NotebookPage>,
    used_element_merge: bool,
}

fn merge_pages_three_way(
    local: &Document,
    remote: &Document,
    attachments: &MergedAttachments,
    effective_page_hashes: &[String],
    effective_page_elements: &[PageElementFingerprints],
) -> Option<MergedPagesResult> {
    let mut local_pages = local.resolved_pages();
    let remote_pages = remote.resolved_pages();
    for page in &mut local_pages {
        remap_page_attachments(
            page,
            &attachments.local_pdf_remap,
            &attachments.local_tpl_remap,
        );
    }

    let base_len = effective_page_hashes.len();
    let local_hashes: Vec<String> = local_pages.iter().map(canonical_page_hash).collect();
    let remote_hashes: Vec<String> = remote_pages.iter().map(canonical_page_hash).collect();
    let shared_len = local_pages.len().min(remote_pages.len()).min(base_len);
    let mut merged_pages = Vec::with_capacity(local_pages.len().max(remote_pages.len()));
    let mut used_element_merge = false;

    for i in 0..shared_len {
        let base_h = &effective_page_hashes[i];
        let l_changed = local_hashes[i] != *base_h;
        let r_changed = remote_hashes[i] != *base_h;
        if l_changed && r_changed && local_hashes[i] != remote_hashes[i] {
            let merged_page = if let Some(base_fp) = effective_page_elements.get(i)
                && let Some(mp) = try_merge_single_page(&local_pages[i], &remote_pages[i], base_fp)
            {
                mp
            } else {
                let l_fp = compute_page_elements(&local_pages[i]);
                let r_fp = compute_page_elements(&remote_pages[i]);
                if is_page_element_subset(&r_fp, &l_fp) {
                    local_pages[i].clone()
                } else if is_page_element_subset(&l_fp, &r_fp) {
                    remote_pages[i].clone()
                } else {
                    return None;
                }
            };
            merged_pages.push(merged_page);
            used_element_merge = true;
        } else if l_changed {
            merged_pages.push(local_pages[i].clone());
        } else {
            merged_pages.push(remote_pages[i].clone());
        }
    }

    append_deleted_tail_edits(
        &local_pages,
        &local_hashes,
        &remote_pages,
        &remote_hashes,
        effective_page_hashes,
        shared_len,
        &mut merged_pages,
    );
    append_new_tail_pages(
        &local_pages,
        &local_hashes,
        &remote_pages,
        &remote_hashes,
        base_len,
        &mut merged_pages,
    );

    Some(MergedPagesResult {
        pages: merged_pages,
        used_element_merge,
    })
}

fn append_deleted_tail_edits(
    local_pages: &[NotebookPage],
    local_hashes: &[String],
    remote_pages: &[NotebookPage],
    remote_hashes: &[String],
    effective_page_hashes: &[String],
    shared_len: usize,
    merged_pages: &mut Vec<NotebookPage>,
) {
    let base_len = effective_page_hashes.len();
    if local_pages.len() < base_len && remote_pages.len() < base_len {
        let max_tail = local_pages.len().max(remote_pages.len());
        for i in shared_len..max_tail {
            if let Some(lp) = local_pages.get(i)
                && local_hashes[i] != effective_page_hashes[i]
            {
                merged_pages.push(lp.clone());
            } else if let Some(rp) = remote_pages.get(i)
                && remote_hashes[i] != effective_page_hashes[i]
            {
                merged_pages.push(rp.clone());
            }
        }
    } else if local_pages.len() < base_len {
        for i in local_pages.len()..base_len.min(remote_pages.len()) {
            if remote_hashes[i] != effective_page_hashes[i] {
                merged_pages.push(remote_pages[i].clone());
            }
        }
    } else if remote_pages.len() < base_len {
        for i in remote_pages.len()..base_len.min(local_pages.len()) {
            if local_hashes[i] != effective_page_hashes[i] {
                merged_pages.push(local_pages[i].clone());
            }
        }
    }
}

fn append_new_tail_pages(
    local_pages: &[NotebookPage],
    local_hashes: &[String],
    remote_pages: &[NotebookPage],
    remote_hashes: &[String],
    base_len: usize,
    merged_pages: &mut Vec<NotebookPage>,
) {
    let mut appended_hashes: HashSet<String> = HashSet::new();
    for i in base_len..remote_pages.len() {
        appended_hashes.insert(remote_hashes[i].clone());
        merged_pages.push(remote_pages[i].clone());
    }
    for i in base_len..local_pages.len() {
        if !appended_hashes.contains(&local_hashes[i]) {
            merged_pages.push(local_pages[i].clone());
        }
    }
}

/// Runs a deterministic 3-way merge across metadata, attachments, pages, and same-page elements.
#[must_use]
pub fn merge_documents_three_way(
    local: &Document,
    remote: &Document,
    base: &DocumentMergeBase,
) -> MergeDecision {
    let local_hash = canonical_document_hash(local);
    let remote_hash = canonical_document_hash(remote);
    if local_hash == remote_hash {
        let mut reconciled = remote.clone();
        reconcile_timestamps_into(&mut reconciled, local, remote);
        reconciled.current_page = local
            .current_page
            .min(reconciled.page_count().saturating_sub(1));
        reconciled.sync_active_to_pages();
        reconciled.load_active_from_pages();
        return MergeDecision::FastForward(reconciled);
    }

    if canonical_document_body_hash(local) == canonical_document_body_hash(remote) {
        let mut merged = remote.clone();
        apply_lww_metadata(&mut merged, local, remote);
        reconcile_timestamps_into(&mut merged, local, remote);
        merged.current_page = local
            .current_page
            .min(merged.page_count().saturating_sub(1));
        merged.sync_active_to_pages();
        merged.load_active_from_pages();
        return MergeDecision::Merged {
            doc: merged,
            kind: MergeKind::MetadataOnly,
        };
    }

    // Idempotent ACK recovery: if `remote` matches our last unacked in-flight snapshot OR is an
    // exact element-level prefix of `local` when no prior server version existed, keep `local`.
    let remote_is_unacked_push = base
        .pending_push_hash
        .as_deref()
        .is_some_and(|h| h == remote_hash)
        || (base.page_hashes.is_empty() && is_document_element_subset(remote, local));
    if remote_is_unacked_push {
        let mut rebased = local.clone();
        reconcile_timestamps_into(&mut rebased, local, remote);
        rebased.id = if !remote.id.is_empty() {
            remote.id.clone()
        } else {
            local.id.clone()
        };
        rebased.sync_active_to_pages();
        rebased.load_active_from_pages();
        return MergeDecision::Merged {
            doc: rebased,
            kind: MergeKind::RebasedPendingPush,
        };
    }

    let remote_elements = compute_document_page_elements(remote);
    let (effective_page_hashes, effective_page_elements): (&[String], &[PageElementFingerprints]) =
        if !base.page_hashes.is_empty() {
            (&base.page_hashes, &base.page_elements)
        } else if !base.pending_push_page_hashes.is_empty()
            && !base.pending_push_page_elements.is_empty()
            && is_ancestor_subset_of_remote(&base.pending_push_page_elements, &remote_elements)
        {
            (
                &base.pending_push_page_hashes,
                &base.pending_push_page_elements,
            )
        } else {
            (&[], &[])
        };

    if effective_page_hashes.is_empty() {
        return MergeDecision::Conflict;
    }

    let Some(attachments) = merge_attachments(local, remote) else {
        return MergeDecision::Conflict;
    };
    let Some(page_merge) = merge_pages_three_way(
        local,
        remote,
        &attachments,
        effective_page_hashes,
        effective_page_elements,
    ) else {
        return MergeDecision::Conflict;
    };

    let mut merged = if local.meta_updated_at_ms > remote.meta_updated_at_ms {
        local.clone()
    } else {
        remote.clone()
    };
    merged.id = if !remote.id.is_empty() {
        remote.id.clone()
    } else {
        local.id.clone()
    };
    apply_lww_metadata(&mut merged, local, remote);
    reconcile_timestamps_into(&mut merged, local, remote);
    merged.updated_at_ms = merged.updated_at_ms.max(now_unix_ms());
    merged.pdf_documents = attachments.pdfs;
    merged.image_templates = attachments.templates;
    merged.pages = page_merge.pages;
    if merged.pages.is_empty() {
        merged.pages.push(NotebookPage::new(
            merged.width,
            merged.height,
            merged.paper_template,
            merged.paper_color,
        ));
    }
    deduplicate_stroke_ids(&mut merged, local.next_stroke_id.max(remote.next_stroke_id));
    merged.current_page = local.current_page.min(merged.pages.len().saturating_sub(1));
    merged.load_active_from_pages();

    MergeDecision::Merged {
        doc: merged,
        kind: if page_merge.used_element_merge {
            MergeKind::ElementLevel
        } else {
            MergeKind::PageLevel
        },
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use papercraft_doc::{
        PaperColor, PaperTemplate, PdfPageBackdrop, PenSettings, ShapeKind, StrokeSample,
    };
    use papercraft_sync_protocol::canonical_document_page_hashes;

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

    fn make_text(content: &str, x: f64, y: f64) -> Arc<TextItem> {
        Arc::new(TextItem {
            text: content.to_string(),
            x,
            y,
            size: 24.0,
            ..TextItem::default()
        })
    }

    fn base_from_doc(doc: &Document) -> DocumentMergeBase {
        DocumentMergeBase {
            page_hashes: canonical_document_page_hashes(doc),
            page_elements: compute_document_page_elements(doc),
            ..Default::default()
        }
    }

    #[test]
    fn fast_forward_reconciles_timestamps_without_conflict() {
        let mut a = Document::new(1000, 1400);
        a.id = "nb-ff".to_string();
        a.title = "Same".to_string();
        a.created_at_ms = 500;
        a.updated_at_ms = 2000;
        a.pages[0]
            .strokes
            .push(make_stroke(1, 10.0, 10.0, 50.0, 50.0));
        a.next_stroke_id = 2;
        a.load_active_from_pages();

        let mut b = a.clone();
        b.created_at_ms = 900;
        b.updated_at_ms = 1200;

        let base = DocumentMergeBase::default();
        match merge_documents_three_way(&a, &b, &base) {
            MergeDecision::FastForward(doc) => {
                assert_eq!(doc.created_at_ms, 500);
                assert_eq!(doc.updated_at_ms, 2000);
                assert_eq!(doc.pages[0].strokes.len(), 1);
            }
            other => panic!("expected FastForward, got {other:?}"),
        }
    }

    #[test]
    fn same_page_concurrent_strokes_merge_and_deduplicate_stroke_ids() {
        let mut base_doc = Document::new(1200, 1600);
        base_doc.id = "nb-same-page".to_string();
        base_doc.title = "Shared Page".to_string();
        let s_base = base_doc.alloc_stroke_id();
        base_doc.pages[0]
            .strokes
            .push(make_stroke(s_base.0, 10.0, 10.0, 20.0, 20.0));
        base_doc.load_active_from_pages();
        let base = base_from_doc(&base_doc);

        // Both sides independently allocate StrokeId(2) on Page 0!
        let mut local = base_doc.clone();
        let l_id = local.alloc_stroke_id();
        local
            .strokes
            .push(make_stroke(l_id.0, 100.0, 100.0, 160.0, 120.0));
        local.sync_active_to_pages();

        let mut remote = base_doc;
        let r_id = remote.alloc_stroke_id();
        remote
            .strokes
            .push(make_stroke(r_id.0, 300.0, 300.0, 360.0, 340.0));
        remote.sync_active_to_pages();

        match merge_documents_three_way(&local, &remote, &base) {
            MergeDecision::Merged { doc, kind } => {
                assert_eq!(kind, MergeKind::ElementLevel);
                assert_eq!(doc.pages[0].strokes.len(), 3);
                let ids: HashSet<u64> = doc.pages[0].strokes.iter().map(|s| s.id.0).collect();
                assert_eq!(ids.len(), 3, "merged stroke IDs must be strictly unique");
                assert!(doc.next_stroke_id >= 4);
            }
            other => panic!("expected element-level merge, got {other:?}"),
        }
    }

    #[test]
    fn one_side_erases_base_stroke_while_other_adds_stroke_merges_cleanly() {
        let mut base_doc = Document::new(1200, 1600);
        base_doc.id = "nb-erase".to_string();
        let s1 = base_doc.alloc_stroke_id();
        let s2 = base_doc.alloc_stroke_id();
        base_doc.pages[0]
            .strokes
            .push(make_stroke(s1.0, 10.0, 10.0, 20.0, 20.0));
        base_doc.pages[0]
            .strokes
            .push(make_stroke(s2.0, 40.0, 40.0, 50.0, 50.0));
        base_doc.load_active_from_pages();
        let base = base_from_doc(&base_doc);

        // Local erases stroke 1.
        let mut local = base_doc.clone();
        local.strokes.remove(0);
        local.sync_active_to_pages();

        // Remote keeps both base strokes and adds stroke 3.
        let mut remote = base_doc;
        let s3 = remote.alloc_stroke_id();
        remote
            .strokes
            .push(make_stroke(s3.0, 200.0, 200.0, 250.0, 220.0));
        remote.sync_active_to_pages();

        match merge_documents_three_way(&local, &remote, &base) {
            MergeDecision::Merged { doc, kind } => {
                assert_eq!(kind, MergeKind::ElementLevel);
                // Base stroke 2 survives + Remote stroke 3 survives; erased stroke 1 stays erased!
                assert_eq!(doc.pages[0].strokes.len(), 2);
            }
            other => panic!("expected element-level merge, got {other:?}"),
        }
    }

    #[test]
    fn concurrent_same_text_box_or_template_change_returns_conflict() {
        let mut base_doc = Document::new(1200, 1600);
        base_doc.id = "nb-text-conflict".to_string();
        base_doc.pages[0]
            .texts
            .push(make_text("Original note", 80.0, 80.0));
        base_doc.load_active_from_pages();
        let base = base_from_doc(&base_doc);

        // Overlapping edits to the same base text item -> Conflict.
        let mut local = base_doc.clone();
        Arc::make_mut(&mut local.texts[0]).text = "Local edit".to_string();
        local.sync_active_to_pages();

        let mut remote = base_doc.clone();
        Arc::make_mut(&mut remote.texts[0]).text = "Remote edit".to_string();
        remote.sync_active_to_pages();

        assert_eq!(
            merge_documents_three_way(&local, &remote, &base),
            MergeDecision::Conflict
        );

        // Non-overlapping independent text boxes on the same page -> merge cleanly!
        let mut local_new = base_doc.clone();
        local_new.texts.push(make_text("Left column", 50.0, 300.0));
        local_new.sync_active_to_pages();

        let mut remote_new = base_doc.clone();
        remote_new
            .texts
            .push(make_text("Right column", 600.0, 300.0));
        remote_new.sync_active_to_pages();

        match merge_documents_three_way(&local_new, &remote_new, &base) {
            MergeDecision::Merged { doc, .. } => {
                assert_eq!(doc.pages[0].texts.len(), 3);
            }
            other => panic!("expected disjoint texts to merge, got {other:?}"),
        }

        // Divergent page template changes on the same page -> Conflict.
        let mut local_tpl = base_doc.clone();
        local_tpl.paper_template = PaperTemplate::Grid;
        local_tpl.sync_active_to_pages();

        let mut remote_tpl = base_doc;
        remote_tpl.paper_template = PaperTemplate::Dotted;
        remote_tpl.paper_color = PaperColor::WarmCream;
        remote_tpl.sync_active_to_pages();

        assert_eq!(
            merge_documents_three_way(&local_tpl, &remote_tpl, &base),
            MergeDecision::Conflict
        );
    }

    #[test]
    fn concurrent_attachment_appends_remap_pdf_backdrops_and_merge_pages() {
        let mut base_doc = Document::new(1200, 1600);
        base_doc.id = "nb-attach".to_string();
        base_doc.pages.push(NotebookPage::new(
            1200,
            1600,
            PaperTemplate::Blank,
            PaperColor::PureWhite,
        ));
        base_doc.load_active_from_pages();
        let base = base_from_doc(&base_doc);

        let mut local = base_doc.clone();
        local.pdf_documents.push(EmbeddedPdf {
            name: "Local.pdf".to_string(),
            bytes: Arc::new(b"%PDF-local".to_vec()),
        });
        local.pages[0].pdf_backdrop = Some(PdfPageBackdrop {
            pdf_index: 0,
            page_index: 0,
        });
        local.load_active_from_pages();

        let mut remote = base_doc;
        remote.pdf_documents.push(EmbeddedPdf {
            name: "Remote.pdf".to_string(),
            bytes: Arc::new(b"%PDF-remote".to_vec()),
        });
        remote.pages[1].pdf_backdrop = Some(PdfPageBackdrop {
            pdf_index: 0,
            page_index: 0,
        });
        remote.load_active_from_pages();

        match merge_documents_three_way(&local, &remote, &base) {
            MergeDecision::Merged { doc, .. } => {
                assert_eq!(doc.pdf_documents.len(), 2);
                assert_eq!(doc.pdf_documents[0].name, "Remote.pdf");
                assert_eq!(doc.pdf_documents[1].name, "Local.pdf");
                assert_eq!(doc.pages[0].pdf_backdrop.unwrap().pdf_index, 1);
                assert_eq!(doc.pages[1].pdf_backdrop.unwrap().pdf_index, 0);
            }
            other => panic!("expected attachment merge, got {other:?}"),
        }
    }

    #[test]
    fn pending_push_ack_recovery_and_subset_creation_retry_avoid_conflict() {
        let mut pushed = Document::new(1200, 1600);
        pushed.id = "nb-create".to_string();
        let s1 = pushed.alloc_stroke_id();
        pushed
            .strokes
            .push(make_stroke(s1.0, 10.0, 10.0, 30.0, 30.0));
        pushed.sync_active_to_pages();

        // User drew another stroke locally after push was sent, before ACK returned.
        let mut local = pushed.clone();
        let s2 = local.alloc_stroke_id();
        local
            .strokes
            .push(make_stroke(s2.0, 60.0, 60.0, 90.0, 90.0));
        local.sync_active_to_pages();

        // Case 1: explicit pending_push_hash recorded before push.
        let base_with_pending = DocumentMergeBase {
            pending_push_hash: Some(canonical_document_hash(&pushed)),
            pending_push_page_hashes: canonical_document_page_hashes(&pushed),
            pending_push_page_elements: compute_document_page_elements(&pushed),
            ..Default::default()
        };
        match merge_documents_three_way(&local, &pushed, &base_with_pending) {
            MergeDecision::Merged { doc, kind } => {
                assert_eq!(kind, MergeKind::RebasedPendingPush);
                assert_eq!(doc.pages[0].strokes.len(), 2);
            }
            other => panic!("expected RebasedPendingPush, got {other:?}"),
        }

        // Case 2: empty base (e.g. crash before manifest saved) still recognizes subset.
        match merge_documents_three_way(&local, &pushed, &DocumentMergeBase::default()) {
            MergeDecision::Merged { doc, kind } => {
                assert_eq!(kind, MergeKind::RebasedPendingPush);
                assert_eq!(doc.pages[0].strokes.len(), 2);
            }
            other => panic!("expected RebasedPendingPush on subset, got {other:?}"),
        }
    }

    #[test]
    fn strip_conflict_copy_suffix_prevents_cascading_names() {
        assert_eq!(
            strip_conflict_copy_suffix("Linear Algebra"),
            "Linear Algebra"
        );
        assert_eq!(
            strip_conflict_copy_suffix("Linear Algebra (Conflict Copy)"),
            "Linear Algebra"
        );
        assert_eq!(
            strip_conflict_copy_suffix("Linear Algebra (Conflict Copy 2)"),
            "Linear Algebra"
        );
        assert_eq!(
            strip_conflict_copy_suffix("Linear Algebra (Conflict Copy) (Conflict Copy 3)"),
            "Linear Algebra"
        );
    }

    #[test]
    fn same_page_shape_and_stroke_merge_keeps_both() {
        let mut base_doc = Document::new(1200, 1600);
        base_doc.id = "nb-shape-stroke".to_string();
        base_doc.load_active_from_pages();
        let base = base_from_doc(&base_doc);

        let mut local = base_doc.clone();
        let s1 = local.alloc_stroke_id();
        local
            .strokes
            .push(make_stroke(s1.0, 20.0, 20.0, 80.0, 80.0));
        local.sync_active_to_pages();

        let mut remote = base_doc;
        remote.shapes.push(Arc::new(GeometricShape {
            kind: ShapeKind::Rectangle,
            x1: 200.0,
            y1: 200.0,
            x2: 320.0,
            y2: 280.0,
            ..GeometricShape::default()
        }));
        remote.sync_active_to_pages();

        match merge_documents_three_way(&local, &remote, &base) {
            MergeDecision::Merged { doc, kind } => {
                assert_eq!(kind, MergeKind::ElementLevel);
                assert_eq!(doc.pages[0].strokes.len(), 1);
                assert_eq!(doc.pages[0].shapes.len(), 1);
            }
            other => panic!("expected ElementLevel merge, got {other:?}"),
        }
    }
}
