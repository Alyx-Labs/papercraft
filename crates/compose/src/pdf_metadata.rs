//! Document-level PDF metadata, outline index, named destinations, page labels,
//! optional content layers, interactive forms, and structure tree preservation.

use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::dict::keys::{
    A, ACRO_FORM, AF, AP, AS, AUTHOR, BASE_STATE, C, CO, COLLECTION, COUNT, CREATION_DATE, CREATOR,
    D, DEST, DESTS, EMBEDDED_FILES, EXTENSIONS, F, FIELDS, FIRST, FT, INFO, JAVA_SCRIPT, KEYWORDS,
    KIDS, LANG, MARK_INFO, METADATA, MOD_DATE, NAME, NAMES, NEXT, NUMS, OCGS, OCPROPERTIES, OFF,
    ON, OPEN_ACTION, ORDER, OUTLINES, OUTPUT_INTENTS, P, PAGE_LABELS, PAGE_LAYOUT, PAGE_MODE,
    PARENT, PIECE_INFO, PREV, PRODUCER, RECT, S, SIG, SIG_FLAGS, ST, STRUCT_PARENT,
    STRUCT_TREE_ROOT, SUBJECT, SUBTYPE, TITLE, URI, VERSION, VIEWER_PREFERENCES, WIDGET, XFA,
};
use hayro::hayro_syntax::object::{
    self, Array, DateTime, Dict, FromBytes, MaybeRef, Name, Object, ObjectIdentifier, Stream,
};
use papercraft_doc::{Document, PdfPageBackdrop};

use crate::export::PdfWriterState;
use crate::pdf_passthrough::{
    KEY_ENCRYPT, ObjectCopier, PageLabelRule, SourcePdfContext, format_pdf_f64, page_bytes_writer,
    write_pdf_literal_string, write_pdf_name,
};

const MAX_TREE_DEPTH: usize = 64;
const MAX_NAMED_DESTS: usize = 200_000;
const MAX_PAGE_LABEL_ENTRIES: usize = 50_000;
const MAX_OUTLINE_ITEMS: usize = 50_000;
const MAX_OUTLINE_DEPTH: usize = 64;
const NAME_TREE_LEAF_SIZE: usize = 64;

const KEY_GO_TO: &[u8] = b"GoTo";
const KEY_USE_OUTLINES: &[u8] = b"UseOutlines";
const KEY_URLS: &[u8] = b"URLS";
const KEY_IDS: &[u8] = b"IDS";
const KEY_RENDITIONS: &[u8] = b"Renditions";
const KEY_ALTERNATE_PRESENTATIONS: &[u8] = b"AlternatePresentations";
const KEY_RB_GROUPS: &[u8] = b"RBGroups";
const KEY_LOCKED: &[u8] = b"Locked";
const KEY_CONFIGS: &[u8] = b"Configs";

/// Snapshot of export inputs needed to finalize `/Info` and `/Catalog` metadata.
pub(crate) struct MetadataExportContext<'a> {
    pub(crate) doc: &'a Document,
    pub(crate) selected_backdrops: &'a [Option<PdfPageBackdrop>],
    pub(crate) ordered_used_pdfs: &'a [usize],
    pub(crate) primary_pdf_index: Option<usize>,
    pub(crate) parsed_pdfs: &'a [Option<Option<Pdf>>],
    pub(crate) source_contexts: &'a [Option<SourcePdfContext>],
}

impl<'a> MetadataExportContext<'a> {
    fn pdf_and_ctx(&self, pdf_idx: usize) -> Option<(&'a Pdf, &'a SourcePdfContext)> {
        let pdf = self.parsed_pdfs.get(pdf_idx)?.as_ref()?.as_ref()?;
        let ctx = self.source_contexts.get(pdf_idx)?.as_ref()?;
        Some((pdf, ctx))
    }

    fn catalog_of(&self, pdf_idx: usize) -> Option<(&'a Pdf, &'a SourcePdfContext, Dict<'a>)> {
        let (pdf, ctx) = self.pdf_and_ctx(pdf_idx)?;
        let catalog = pdf.xref().get::<Dict<'a>>(pdf.xref().root_id())?;
        Some((pdf, ctx, catalog))
    }
}

/// Emits trailer `/Info` and all preserved `/Catalog` metadata, outlines, destinations,
/// page labels, annotations/forms, optional content layers, and structure trees.
pub(crate) fn emit_document_metadata_and_catalog(
    writer: &mut PdfWriterState,
    ctx: &MetadataExportContext<'_>,
) {
    emit_info_dict(writer, ctx);

    let emitted_outlines = emit_outlines(writer, ctx);
    emit_catalog_dests(writer, ctx);
    emit_catalog_names(writer, ctx);
    emit_page_labels(writer, ctx);
    emit_catalog_properties(writer, ctx, emitted_outlines);
    emit_oc_properties(writer, ctx);
    emit_acro_form(writer, ctx);
    emit_struct_tree(writer, ctx);
}

// ---------------------------------------------------------------------------
// Pre-scanning helpers called when building `SourcePdfContext`
// ---------------------------------------------------------------------------

/// Collects valid PDF 1.1 `/Dests` names and PDF 1.2+ `/Names /Dests` strings
/// whose target pages exist in `page_id_map` or `page_index_map`.
pub(crate) fn collect_valid_named_dests(
    pdf: &Pdf,
    page_id_map: &HashMap<ObjectIdentifier, Option<usize>>,
    page_index_map: &HashMap<usize, usize>,
) -> (HashSet<Vec<u8>>, HashSet<Vec<u8>>) {
    let mut valid_names = HashSet::new();
    let mut valid_strings = HashSet::new();
    let Some(catalog) = pdf.xref().get::<Dict<'_>>(pdf.xref().root_id()) else {
        return (valid_names, valid_strings);
    };

    if let Some(dests_dict) = catalog.get::<Dict<'_>>(DESTS) {
        for (key, val) in dests_dict.entries() {
            if valid_names.len() >= MAX_NAMED_DESTS {
                break;
            }
            if let Some(obj) = resolve_maybe_obj(pdf, val)
                && is_dest_target_obj_valid(&obj, page_id_map, page_index_map)
            {
                valid_names.insert(key.as_ref().to_vec());
            }
        }
    }

    if let Some(names_dict) = catalog.get::<Dict<'_>>(NAMES)
        && let Some(dests_tree) = names_dict.get::<Dict<'_>>(DESTS)
    {
        let mut entries = Vec::new();
        collect_name_tree_raw(pdf, &dests_tree, &mut entries, MAX_NAMED_DESTS);
        for (key_bytes, val_maybe) in entries {
            if let Some(obj) = resolve_maybe_obj(pdf, val_maybe)
                && is_dest_target_obj_valid(&obj, page_id_map, page_index_map)
            {
                valid_strings.insert(key_bytes);
            }
        }
    }

    (valid_names, valid_strings)
}

/// Parses `/Catalog /PageLabels` Number Tree into sorted `(source_page_index, rule)` entries.
pub(crate) fn parse_page_labels(pdf: &Pdf) -> Vec<(usize, PageLabelRule)> {
    let Some(catalog) = pdf.xref().get::<Dict<'_>>(pdf.xref().root_id()) else {
        return Vec::new();
    };
    let Some(labels_tree) = catalog.get::<Dict<'_>>(PAGE_LABELS) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    let mut visited = HashSet::new();
    if let Some(id) = labels_tree.obj_id() {
        visited.insert(id);
    }
    walk_page_label_node(pdf, &labels_tree, 0, &mut visited, &mut out);
    out.sort_by_key(|(idx, _)| *idx);
    let mut deduped: Vec<(usize, PageLabelRule)> = Vec::with_capacity(out.len());
    for entry in out {
        if let Some(last) = deduped.last_mut()
            && last.0 == entry.0
        {
            *last = entry;
            continue;
        }
        deduped.push(entry);
    }
    deduped
}

fn walk_page_label_node(
    pdf: &Pdf,
    node: &Dict<'_>,
    depth: usize,
    visited: &mut HashSet<ObjectIdentifier>,
    out: &mut Vec<(usize, PageLabelRule)>,
) {
    if depth >= MAX_TREE_DEPTH || out.len() >= MAX_PAGE_LABEL_ENTRIES {
        return;
    }
    if let Some(nums) = node.get::<Array<'_>>(NUMS) {
        let mut iter = nums.raw_iter();
        while let (Some(k_item), Some(v_item)) = (iter.next(), iter.next()) {
            if out.len() >= MAX_PAGE_LABEL_ENTRIES {
                break;
            }
            let Some(page_idx) = extract_nonneg_int_from_maybe(pdf, k_item) else {
                continue;
            };
            let Some(Object::Dict(rule_dict)) = resolve_maybe_obj(pdf, v_item) else {
                continue;
            };
            let style = rule_dict
                .get::<Name<'_>>(S)
                .map(|n| n.as_ref().to_vec())
                .filter(|s| matches!(s.as_slice(), b"D" | b"R" | b"r" | b"A" | b"a"));
            let prefix = rule_dict
                .get::<object::String<'_>>(P)
                .map(|s| s.as_bytes().to_vec());
            let start = rule_dict.get::<i32>(ST).map_or(1, |n| n.max(1) as usize);
            out.push((
                page_idx,
                PageLabelRule {
                    style,
                    prefix,
                    start,
                },
            ));
        }
    }
    if let Some(kids) = node.get::<Array<'_>>(KIDS) {
        for kid in kids.raw_iter() {
            if out.len() >= MAX_PAGE_LABEL_ENTRIES {
                break;
            }
            let kid_dict = match kid {
                MaybeRef::Ref(r) => {
                    let id: ObjectIdentifier = r.into();
                    if !visited.insert(id) {
                        continue;
                    }
                    pdf.xref().get::<Dict<'_>>(id)
                }
                MaybeRef::NotRef(Object::Dict(d)) => Some(d),
                MaybeRef::NotRef(_) => None,
            };
            if let Some(d) = kid_dict {
                walk_page_label_node(pdf, &d, depth + 1, visited, out);
            }
        }
    }
}

fn collect_name_tree_raw<'a>(
    pdf: &'a Pdf,
    root: &Dict<'a>,
    out: &mut Vec<(Vec<u8>, MaybeRef<Object<'a>>)>,
    limit: usize,
) {
    let mut visited = HashSet::new();
    if let Some(id) = root.obj_id() {
        visited.insert(id);
    }
    walk_name_tree_node(pdf, root, 0, &mut visited, out, limit);
}

fn walk_name_tree_node<'a>(
    pdf: &'a Pdf,
    node: &Dict<'a>,
    depth: usize,
    visited: &mut HashSet<ObjectIdentifier>,
    out: &mut Vec<(Vec<u8>, MaybeRef<Object<'a>>)>,
    limit: usize,
) {
    if depth >= MAX_TREE_DEPTH || out.len() >= limit {
        return;
    }
    if let Some(names) = node.get::<Array<'a>>(NAMES) {
        let mut iter = names.raw_iter();
        while let (Some(k_item), Some(v_item)) = (iter.next(), iter.next()) {
            if out.len() >= limit {
                break;
            }
            let Some(Object::String(k_str)) = resolve_maybe_obj(pdf, k_item) else {
                continue;
            };
            out.push((k_str.as_bytes().to_vec(), v_item));
        }
    }
    if let Some(kids) = node.get::<Array<'a>>(KIDS) {
        for kid in kids.raw_iter() {
            if out.len() >= limit {
                break;
            }
            let kid_dict = match kid {
                MaybeRef::Ref(r) => {
                    let id: ObjectIdentifier = r.into();
                    if !visited.insert(id) {
                        continue;
                    }
                    pdf.xref().get::<Dict<'a>>(id)
                }
                MaybeRef::NotRef(Object::Dict(d)) => Some(d),
                MaybeRef::NotRef(_) => None,
            };
            if let Some(d) = kid_dict {
                walk_name_tree_node(pdf, &d, depth + 1, visited, out, limit);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Shared validation & annotation filtering
// ---------------------------------------------------------------------------

pub(crate) fn resolve_explicit_dest_target(
    arr: &Array<'_>,
    page_id_map: &HashMap<ObjectIdentifier, Option<usize>>,
    page_index_map: &HashMap<usize, usize>,
) -> Option<usize> {
    let first = arr.raw_iter().next()?;
    match first {
        MaybeRef::Ref(r) => page_id_map.get(&r.into()).copied().flatten(),
        MaybeRef::NotRef(Object::Number(n)) => {
            let v = n.as_f64();
            if v.is_finite() && v >= 0.0 && v.fract() == 0.0 && v <= (usize::MAX as f64) {
                page_index_map.get(&(v as usize)).copied()
            } else {
                None
            }
        }
        MaybeRef::NotRef(_) => None,
    }
}

fn is_dest_target_obj_valid(
    obj: &Object<'_>,
    page_id_map: &HashMap<ObjectIdentifier, Option<usize>>,
    page_index_map: &HashMap<usize, usize>,
) -> bool {
    match obj {
        Object::Array(arr) => {
            resolve_explicit_dest_target(arr, page_id_map, page_index_map).is_some()
        }
        Object::Dict(dict) => dict.get::<Array<'_>>(D).is_some_and(|arr| {
            resolve_explicit_dest_target(&arr, page_id_map, page_index_map).is_some()
        }),
        _ => false,
    }
}

pub(crate) fn is_valid_destination(src_ctx: &SourcePdfContext, dest: &Object<'_>) -> bool {
    match dest {
        Object::Array(arr) => {
            resolve_explicit_dest_target(arr, &src_ctx.page_id_map, &src_ctx.page_index_map)
                .is_some()
        }
        Object::Name(n) => {
            let bytes = n.as_ref();
            src_ctx.valid_dest_names.contains(bytes) || src_ctx.valid_dest_strings.contains(bytes)
        }
        Object::String(s) => {
            let bytes = s.as_bytes();
            src_ctx.valid_dest_strings.contains(bytes) || src_ctx.valid_dest_names.contains(bytes)
        }
        _ => false,
    }
}

pub(crate) fn is_valid_action(src_ctx: &SourcePdfContext, action: &Dict<'_>, depth: usize) -> bool {
    if depth >= 16 {
        return false;
    }
    let Some(subtype) = action.get::<Name<'_>>(S) else {
        return false;
    };
    if subtype.as_ref() == JAVA_SCRIPT {
        return false;
    }
    if subtype.as_ref() == KEY_GO_TO {
        let Some(dest_obj) = action.get::<Object<'_>>(D) else {
            return false;
        };
        if !is_valid_destination(src_ctx, &dest_obj) {
            return false;
        }
    }
    if let Some(next_obj) = action.get::<Object<'_>>(NEXT) {
        match next_obj {
            Object::Dict(next_dict) => {
                if !is_valid_action(src_ctx, &next_dict, depth + 1) {
                    return false;
                }
            }
            Object::Array(next_arr) => {
                for item in next_arr.iter::<Dict<'_>>() {
                    if !is_valid_action(src_ctx, &item, depth + 1) {
                        return false;
                    }
                }
            }
            _ => {}
        }
    }
    true
}

pub(crate) fn is_signature_widget(annot: &Dict<'_>) -> bool {
    if !annot
        .get::<Name<'_>>(SUBTYPE)
        .is_some_and(|st| st.as_ref() == WIDGET)
    {
        return false;
    }
    if annot
        .get::<Name<'_>>(FT)
        .is_some_and(|ft| ft.as_ref() == SIG)
    {
        return true;
    }
    let mut visited = HashSet::new();
    if let Some(id) = annot.obj_id() {
        visited.insert(id);
    }
    let mut cur = annot.clone();
    for _ in 0..16 {
        let Some(parent_ref) = cur.get_ref(PARENT) else {
            break;
        };
        let parent_id: ObjectIdentifier = parent_ref.into();
        if !visited.insert(parent_id) {
            break;
        }
        let Some(parent_dict) = cur.get::<Dict<'_>>(PARENT) else {
            break;
        };
        if parent_dict
            .get::<Name<'_>>(FT)
            .is_some_and(|ft| ft.as_ref() == SIG)
        {
            return true;
        }
        cur = parent_dict;
    }
    false
}

pub(crate) fn should_keep_annotation(src_ctx: &SourcePdfContext, annot: &Dict<'_>) -> bool {
    if is_signature_widget(annot) {
        return false;
    }
    let is_link = annot
        .get::<Name<'_>>(SUBTYPE)
        .is_some_and(|st| st.as_ref() == b"Link");
    if is_link {
        if let Some(dest) = annot.get::<Object<'_>>(DEST)
            && !is_valid_destination(src_ctx, &dest)
        {
            return false;
        }
        if let Some(action) = annot.get::<Dict<'_>>(A)
            && !is_valid_action(src_ctx, &action, 0)
        {
            return false;
        }
    }
    true
}

fn resolve_maybe_obj<'a>(pdf: &'a Pdf, val: MaybeRef<Object<'a>>) -> Option<Object<'a>> {
    match val {
        MaybeRef::Ref(r) => pdf.xref().get::<Object<'a>>(r.into()),
        MaybeRef::NotRef(obj) => Some(obj),
    }
}

fn extract_nonneg_int_from_maybe(pdf: &Pdf, val: MaybeRef<Object<'_>>) -> Option<usize> {
    let Object::Number(n) = resolve_maybe_obj(pdf, val)? else {
        return None;
    };
    let v = n.as_f64();
    if v.is_finite() && v >= 0.0 && v.fract() == 0.0 && v <= (usize::MAX as f64) {
        Some(v as usize)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Trailer `/Info`
// ---------------------------------------------------------------------------

fn find_last_startxref(data: &[u8]) -> Option<usize> {
    let needle = b"startxref";
    let pos = data.windows(needle.len()).rposition(|w| w == needle)?;
    let rest = skip_pdf_ws_and_comments(&data[pos + needle.len()..]);
    parse_ascii_usize(rest).map(|(val, _)| val)
}

fn is_pdf_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\r' | b'\n' | 0x00 | 0x0C)
}

fn skip_pdf_ws_and_comments(mut data: &[u8]) -> &[u8] {
    loop {
        while let Some(&b) = data.first() {
            if is_pdf_ws(b) {
                data = &data[1..];
            } else {
                break;
            }
        }
        if data.first() == Some(&b'%') {
            while let Some(&b) = data.first() {
                data = &data[1..];
                if b == b'\n' || b == b'\r' {
                    break;
                }
            }
            continue;
        }
        break;
    }
    data
}

fn parse_ascii_usize(data: &[u8]) -> Option<(usize, &[u8])> {
    let mut len = 0usize;
    let mut val = 0usize;
    while let Some(&b) = data.get(len) {
        if !b.is_ascii_digit() {
            break;
        }
        val = val.checked_mul(10)?.checked_add(usize::from(b - b'0'))?;
        len += 1;
    }
    if len == 0 {
        None
    } else {
        Some((val, &data[len..]))
    }
}

fn read_trailer_dict_at<'a>(pdf: &'a Pdf, data: &'a [u8], pos: usize) -> Option<Dict<'a>> {
    let slice = data.get(pos..)?;
    let trimmed = skip_pdf_ws_and_comments(slice);
    if trimmed.starts_with(b"xref") {
        let after_xref = skip_pdf_ws_and_comments(&trimmed[4..]);
        if let Some(dict) = parse_classic_trailer_dict(after_xref) {
            return Some(dict);
        }
        let scan_limit = trimmed.len().min(2_000_000);
        return scan_for_trailer_dict(&trimmed[..scan_limit]);
    }
    let (obj_num_u, after_num) = parse_ascii_usize(trimmed)?;
    let obj_num = u32::try_from(obj_num_u).ok()?;
    let after_num = skip_pdf_ws_and_comments(after_num);
    let (gen_num_u, after_gen) = parse_ascii_usize(after_num)?;
    let gen_num = i32::try_from(gen_num_u).ok()?;
    let after_gen = skip_pdf_ws_and_comments(after_gen);
    if !after_gen.starts_with(b"obj") {
        return None;
    }
    let stream = pdf
        .xref()
        .get::<Stream<'_>>(ObjectIdentifier::new(obj_num, gen_num))?;
    Some(stream.dict().clone())
}

fn parse_classic_trailer_dict(mut cur: &[u8]) -> Option<Dict<'_>> {
    while cur.first().is_some_and(u8::is_ascii_digit) {
        let (_, after_start) = parse_ascii_usize(cur)?;
        let after_start = skip_pdf_ws_and_comments(after_start);
        let (count, after_count) = parse_ascii_usize(after_start)?;
        let after_count = skip_pdf_ws_and_comments(after_count);
        let skip_bytes = count.checked_mul(20)?;
        cur = after_count.get(skip_bytes..)?;
        cur = skip_pdf_ws_and_comments(cur);
    }
    if !cur.starts_with(b"trailer") {
        return None;
    }
    let dict_slice = skip_pdf_ws_and_comments(&cur[7..]);
    Dict::from_bytes(dict_slice)
}

fn scan_for_trailer_dict(slice: &[u8]) -> Option<Dict<'_>> {
    let needle = b"trailer";
    let mut offset = 0usize;
    while let Some(rel) = slice[offset..]
        .windows(needle.len())
        .position(|w| w == needle)
    {
        let idx = offset + rel;
        let prev_ok = idx == 0 || is_pdf_ws(slice[idx - 1]);
        if prev_ok {
            let after = skip_pdf_ws_and_comments(&slice[idx + needle.len()..]);
            if after.starts_with(b"<<")
                && let Some(dict) = Dict::from_bytes(after)
            {
                return Some(dict);
            }
        }
        offset = idx + needle.len();
    }
    None
}

fn find_raw_info_dict(pdf: &Pdf) -> Option<Dict<'_>> {
    let data = pdf.data().as_ref();
    let mut pos = find_last_startxref(data)?;
    let mut visited = HashSet::new();
    for _ in 0..16 {
        if !visited.insert(pos) {
            break;
        }
        let trailer = read_trailer_dict_at(pdf, data, pos)?;
        if let Some(info_ref) = trailer.get_ref(INFO)
            && let Some(info_dict) = pdf.xref().get::<Dict<'_>>(info_ref.into())
        {
            return Some(info_dict);
        }
        if let Some(info_dict) = trailer.get::<Dict<'_>>(INFO) {
            return Some(info_dict);
        }
        let Some(prev) = trailer.get::<i32>(PREV) else {
            break;
        };
        if prev < 0 {
            break;
        }
        pos = prev as usize;
    }
    None
}

fn format_pdf_datetime(dt: DateTime) -> Vec<u8> {
    if dt.utc_offset_hour == 0 && dt.utc_offset_minute == 0 {
        format!(
            "D:{:04}{:02}{:02}{:02}{:02}{:02}Z",
            dt.year, dt.month, dt.day, dt.hour, dt.minute, dt.second
        )
        .into_bytes()
    } else {
        let sign = if dt.utc_offset_hour < 0 { '-' } else { '+' };
        let oh = dt.utc_offset_hour.unsigned_abs();
        format!(
            "D:{:04}{:02}{:02}{:02}{:02}{:02}{}{:02}'{:02}'",
            dt.year,
            dt.month,
            dt.day,
            dt.hour,
            dt.minute,
            dt.second,
            sign,
            oh,
            dt.utc_offset_minute
        )
        .into_bytes()
    }
}

pub(crate) fn encode_pdf_text_string(text: &str) -> Vec<u8> {
    if text.bytes().all(|b| (0x20..=0x7E).contains(&b)) {
        return text.as_bytes().to_vec();
    }
    let mut out = Vec::with_capacity(2 + text.len() * 2);
    out.push(0xFE);
    out.push(0xFF);
    for unit in text.encode_utf16() {
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out
}

fn decode_pdf_text_bytes(raw: &[u8]) -> Option<String> {
    let decoded = if raw.starts_with(&[0xFE, 0xFF]) {
        let units: Vec<u16> = raw[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|&pair| u16::from_be_bytes(pair))
            .collect();
        String::from_utf16_lossy(&units)
    } else {
        String::from_utf8_lossy(raw).into_owned()
    };
    let clean: String = decoded
        .trim()
        .chars()
        .filter(|ch| !ch.is_control())
        .collect();
    let trimmed = clean.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn emit_info_dict(writer: &mut PdfWriterState, ctx: &MetadataExportContext<'_>) {
    let primary_pair = ctx.primary_pdf_index.and_then(|idx| ctx.pdf_and_ctx(idx));
    let mut body = Vec::with_capacity(256);
    body.extend_from_slice(b"<<");

    let mut has_title = false;
    let mut has_producer = false;

    if let Some((primary_pdf, primary_ctx)) = primary_pair {
        if let Some(raw_info) = find_raw_info_dict(primary_pdf) {
            let cp = writer.checkpoint();
            let mut copied_body = Vec::new();
            let ok = copy_raw_info_entries(
                writer,
                primary_pdf,
                primary_ctx,
                &raw_info,
                &mut copied_body,
                &mut has_title,
                &mut has_producer,
            );
            if ok {
                body.extend_from_slice(&copied_body);
            } else {
                writer.rollback(cp);
                has_title = false;
                has_producer = false;
                append_parsed_info_metadata(
                    primary_pdf,
                    &mut body,
                    &mut has_title,
                    &mut has_producer,
                );
            }
        } else {
            append_parsed_info_metadata(primary_pdf, &mut body, &mut has_title, &mut has_producer);
        }
    }

    let doc_title = ctx.doc.title.trim();
    if !has_title && !doc_title.is_empty() {
        body.extend_from_slice(b" /Title ");
        write_pdf_literal_string(&encode_pdf_text_string(doc_title), &mut body);
    }
    if !has_producer {
        body.extend_from_slice(b" /Producer (PaperCraft)");
    }
    body.extend_from_slice(b" >>");

    let info_id = writer.alloc_obj_id();
    writer.emit_raw_object(info_id, &body);
    writer.set_info_obj_id(info_id);
}

fn copy_raw_info_entries(
    writer: &mut PdfWriterState,
    pdf: &Pdf,
    src_ctx: &SourcePdfContext,
    raw_info: &Dict<'_>,
    out: &mut Vec<u8>,
    has_title: &mut bool,
    has_producer: &mut bool,
) -> bool {
    let Ok(mut copier) = ObjectCopier::new(writer, pdf, src_ctx, src_ctx.pdf_idx) else {
        return false;
    };
    for (key, val) in raw_info.entries() {
        let k = key.as_ref();
        if k == KEY_ENCRYPT {
            continue;
        }
        if k == TITLE {
            *has_title = true;
        } else if k == PRODUCER {
            *has_producer = true;
        }
        out.push(b' ');
        write_pdf_name(k, out);
        out.push(b' ');
        if copier.serialize_maybe_ref(val, out, 0).is_err() {
            return false;
        }
    }
    copier.drain_queue().is_ok()
}

fn append_parsed_info_metadata(
    pdf: &Pdf,
    out: &mut Vec<u8>,
    has_title: &mut bool,
    has_producer: &mut bool,
) {
    let meta = pdf.metadata();
    let text_fields: &[(&[u8], Option<&[u8]>)] = &[
        (TITLE, meta.title.as_deref()),
        (AUTHOR, meta.author.as_deref()),
        (SUBJECT, meta.subject.as_deref()),
        (KEYWORDS, meta.keywords.as_deref()),
        (CREATOR, meta.creator.as_deref()),
        (PRODUCER, meta.producer.as_deref()),
    ];
    for &(key, val_opt) in text_fields {
        let Some(bytes) = val_opt else {
            continue;
        };
        if key == TITLE {
            *has_title = true;
        } else if key == PRODUCER {
            *has_producer = true;
        }
        out.push(b' ');
        write_pdf_name(key, out);
        out.push(b' ');
        write_pdf_literal_string(bytes, out);
    }
    if let Some(dt) = meta.creation_date {
        out.push(b' ');
        write_pdf_name(CREATION_DATE, out);
        out.push(b' ');
        write_pdf_literal_string(&format_pdf_datetime(dt), out);
    }
    if let Some(dt) = meta.modification_date {
        out.push(b' ');
        write_pdf_name(MOD_DATE, out);
        out.push(b' ');
        write_pdf_literal_string(&format_pdf_datetime(dt), out);
    }
}

// ---------------------------------------------------------------------------
// Catalog `/Outlines`
// ---------------------------------------------------------------------------

struct PlannedOutlineNode {
    pdf_idx: usize,
    src_id: Option<ObjectIdentifier>,
    title_bytes: Vec<u8>,
    is_open: bool,
    has_valid_source_target: bool,
    fallback_page_id: Option<usize>,
    color: Option<[f64; 3]>,
    flags: Option<i32>,
    children: Vec<PlannedOutlineNode>,
    obj_id: usize,
    visible_descendants: usize,
}

fn plan_pdf_outlines(
    pdf_idx: usize,
    pdf: &Pdf,
    src_ctx: &SourcePdfContext,
) -> Vec<PlannedOutlineNode> {
    let Some(catalog) = pdf.xref().get::<Dict<'_>>(pdf.xref().root_id()) else {
        return Vec::new();
    };
    let Some(outlines_root) = catalog.get::<Dict<'_>>(OUTLINES) else {
        return Vec::new();
    };
    let mut visited = HashSet::new();
    if let Some(root_id) = outlines_root.obj_id() {
        visited.insert(root_id);
    }
    let mut total_items = 0usize;
    plan_outline_siblings(
        pdf_idx,
        pdf,
        src_ctx,
        &outlines_root,
        0,
        &mut visited,
        &mut total_items,
    )
}

fn plan_outline_siblings(
    pdf_idx: usize,
    pdf: &Pdf,
    src_ctx: &SourcePdfContext,
    parent_dict: &Dict<'_>,
    depth: usize,
    visited: &mut HashSet<ObjectIdentifier>,
    total_items: &mut usize,
) -> Vec<PlannedOutlineNode> {
    if depth >= MAX_OUTLINE_DEPTH || *total_items >= MAX_OUTLINE_ITEMS {
        return Vec::new();
    }
    let mut kept = Vec::new();
    let mut cur_ref_opt = parent_dict.get_ref(FIRST);
    while let Some(cur_ref) = cur_ref_opt {
        let cur_id: ObjectIdentifier = cur_ref.into();
        if *total_items >= MAX_OUTLINE_ITEMS || !visited.insert(cur_id) {
            break;
        }
        *total_items += 1;
        let Some(item_dict) = pdf.xref().get::<Dict<'_>>(cur_id) else {
            break;
        };
        let next_ref = item_dict.get_ref(NEXT);
        let children = plan_outline_siblings(
            pdf_idx,
            pdf,
            src_ctx,
            &item_dict,
            depth + 1,
            visited,
            total_items,
        );
        let has_valid_source_target = outline_item_has_valid_target(src_ctx, &item_dict);
        if has_valid_source_target || !children.is_empty() {
            let title_bytes = item_dict
                .get::<object::String<'_>>(TITLE)
                .map(|s| s.as_bytes().to_vec())
                .unwrap_or_default();
            let is_open = item_dict.get::<i32>(COUNT).is_none_or(|c| c >= 0);
            let color = parse_outline_color(&item_dict);
            let flags = item_dict.get::<i32>(F).filter(|&f| f != 0);
            kept.push(PlannedOutlineNode {
                pdf_idx,
                src_id: Some(cur_id),
                title_bytes,
                is_open,
                has_valid_source_target,
                fallback_page_id: None,
                color,
                flags,
                children,
                obj_id: 0,
                visible_descendants: 0,
            });
        }
        cur_ref_opt = next_ref;
    }
    kept
}

fn outline_item_has_valid_target(src_ctx: &SourcePdfContext, item_dict: &Dict<'_>) -> bool {
    if let Some(dest) = item_dict.get::<Object<'_>>(DEST) {
        return is_valid_destination(src_ctx, &dest);
    }
    if let Some(action) = item_dict.get::<Dict<'_>>(A) {
        return is_valid_action(src_ctx, &action, 0);
    }
    false
}

fn parse_outline_color(item_dict: &Dict<'_>) -> Option<[f64; 3]> {
    let arr = item_dict.get::<Array<'_>>(C)?;
    let mut vals = arr.iter::<f64>();
    let r = vals.next()?;
    let g = vals.next()?;
    let b = vals.next()?;
    if vals.next().is_none() && r.is_finite() && g.is_finite() && b.is_finite() {
        Some([r.clamp(0.0, 1.0), g.clamp(0.0, 1.0), b.clamp(0.0, 1.0)])
    } else {
        None
    }
}

fn assign_outline_ids_and_counts(
    nodes: &mut [PlannedOutlineNode],
    writer: &mut PdfWriterState,
) -> usize {
    let mut visible = 0usize;
    for node in nodes {
        node.obj_id = writer.alloc_obj_id();
        node.visible_descendants = assign_outline_ids_and_counts(&mut node.children, writer);
        visible += 1;
        if node.is_open {
            visible += node.visible_descendants;
        }
    }
    visible
}

fn strip_pdf_ext(name: &str) -> &str {
    let trimmed = name.trim();
    if trimmed.len() >= 4 && trimmed[trimmed.len() - 4..].eq_ignore_ascii_case(".pdf") {
        &trimmed[..trimmed.len() - 4]
    } else {
        trimmed
    }
}

fn pdf_display_title_bytes(doc: &Document, pdf_idx: usize, pdf: &Pdf) -> Vec<u8> {
    if let Some(raw_title) = pdf.metadata().title.as_deref()
        && let Some(decoded) = decode_pdf_text_bytes(raw_title)
    {
        return encode_pdf_text_string(&decoded);
    }
    let name = doc
        .pdf_documents
        .get(pdf_idx)
        .map_or("Document", |src| strip_pdf_ext(&src.name));
    let clean = if name.trim().is_empty() {
        "Document"
    } else {
        name.trim()
    };
    encode_pdf_text_string(clean)
}

fn emit_outlines(writer: &mut PdfWriterState, ctx: &MetadataExportContext<'_>) -> bool {
    let mut per_pdf_roots: Vec<(usize, Vec<PlannedOutlineNode>)> = Vec::new();
    for &pdf_idx in ctx.ordered_used_pdfs {
        let Some((pdf, src_ctx)) = ctx.pdf_and_ctx(pdf_idx) else {
            continue;
        };
        let roots = plan_pdf_outlines(pdf_idx, pdf, src_ctx);
        if !roots.is_empty() {
            per_pdf_roots.push((pdf_idx, roots));
        }
    }
    if per_pdf_roots.is_empty() {
        return false;
    }

    let mut top_nodes: Vec<PlannedOutlineNode> = if per_pdf_roots.len() == 1 {
        match per_pdf_roots.into_iter().next() {
            Some((_, roots)) => roots,
            None => return false,
        }
    } else {
        let mut wrappers = Vec::with_capacity(per_pdf_roots.len());
        for (pdf_idx, children) in per_pdf_roots {
            let Some((pdf, src_ctx)) = ctx.pdf_and_ctx(pdf_idx) else {
                continue;
            };
            wrappers.push(PlannedOutlineNode {
                pdf_idx,
                src_id: None,
                title_bytes: pdf_display_title_bytes(ctx.doc, pdf_idx, pdf),
                is_open: true,
                has_valid_source_target: false,
                fallback_page_id: src_ctx.first_exported_dest_page_id,
                color: None,
                flags: None,
                children,
                obj_id: 0,
                visible_descendants: 0,
            });
        }
        wrappers
    };

    if top_nodes.is_empty() {
        return false;
    }

    let cp = writer.checkpoint();
    let outline_root_id = writer.alloc_obj_id();
    let root_visible = assign_outline_ids_and_counts(&mut top_nodes, writer);
    let (Some(first_top), Some(last_top)) = (top_nodes.first(), top_nodes.last()) else {
        writer.rollback(cp);
        return false;
    };

    let root_body = format!(
        "<< /Type /Outlines /First {} 0 R /Last {} 0 R /Count {root_visible} >>",
        first_top.obj_id, last_top.obj_id
    );
    writer.emit_raw_object(outline_root_id, root_body.as_bytes());

    if emit_outline_siblings(writer, ctx, outline_root_id, &top_nodes).is_err() {
        writer.rollback(cp);
        return false;
    }

    let mut extra = Vec::new();
    let _ = write!(
        page_bytes_writer(&mut extra),
        " /Outlines {outline_root_id} 0 R"
    );
    writer.append_catalog_extra(&extra);
    true
}

fn emit_outline_siblings(
    writer: &mut PdfWriterState,
    ctx: &MetadataExportContext<'_>,
    parent_id: usize,
    nodes: &[PlannedOutlineNode],
) -> Result<(), ()> {
    for (idx, node) in nodes.iter().enumerate() {
        let mut body = Vec::with_capacity(192);
        body.extend_from_slice(b"<< /Title ");
        write_pdf_literal_string(&node.title_bytes, &mut body);
        let _ = write!(page_bytes_writer(&mut body), " /Parent {parent_id} 0 R");
        if idx > 0 {
            let prev_id = nodes[idx - 1].obj_id;
            let _ = write!(page_bytes_writer(&mut body), " /Prev {prev_id} 0 R");
        }
        if idx + 1 < nodes.len() {
            let next_id = nodes[idx + 1].obj_id;
            let _ = write!(page_bytes_writer(&mut body), " /Next {next_id} 0 R");
        }
        if let (Some(first_c), Some(last_c)) = (node.children.first(), node.children.last()) {
            let signed_count: isize = if node.is_open {
                isize::try_from(node.visible_descendants).unwrap_or(isize::MAX)
            } else {
                -isize::try_from(node.visible_descendants).unwrap_or(isize::MAX)
            };
            let _ = write!(
                page_bytes_writer(&mut body),
                " /First {} 0 R /Last {} 0 R /Count {signed_count}",
                first_c.obj_id,
                last_c.obj_id
            );
        }
        if let Some([r, g, b]) = node.color {
            body.extend_from_slice(b" /C [");
            format_pdf_f64(r, &mut body);
            body.push(b' ');
            format_pdf_f64(g, &mut body);
            body.push(b' ');
            format_pdf_f64(b, &mut body);
            body.push(b']');
        }
        if let Some(flags) = node.flags {
            let _ = write!(page_bytes_writer(&mut body), " /F {flags}");
        }
        append_outline_target(writer, ctx, node, &mut body)?;
        body.extend_from_slice(b" >>");
        writer.emit_raw_object(node.obj_id, &body);

        if !node.children.is_empty() {
            emit_outline_siblings(writer, ctx, node.obj_id, &node.children)?;
        }
    }
    Ok(())
}

fn append_outline_target(
    writer: &mut PdfWriterState,
    ctx: &MetadataExportContext<'_>,
    node: &PlannedOutlineNode,
    body: &mut Vec<u8>,
) -> Result<(), ()> {
    if node.has_valid_source_target
        && let Some(src_id) = node.src_id
        && let Some((pdf, src_ctx)) = ctx.pdf_and_ctx(node.pdf_idx)
        && let Some(item_dict) = pdf.xref().get::<Dict<'_>>(src_id)
    {
        let mut copier = ObjectCopier::new(writer, pdf, src_ctx, node.pdf_idx)?;
        copier.strip_struct_parent = !src_ctx.exports_all_pages_uniquely;
        copier.rewrite_named_dests = src_ctx.dest_prefix.is_some();
        if let Some(raw_dest) = item_dict.get_raw::<Object<'_>>(DEST)
            && item_dict
                .get::<Object<'_>>(DEST)
                .is_some_and(|d| is_valid_destination(src_ctx, &d))
        {
            body.extend_from_slice(b" /Dest ");
            if let Some(prefix) = src_ctx.dest_prefix.as_deref()
                && try_write_prefixed_outline_dest(pdf, &raw_dest, prefix, body)
            {
                return copier.drain_queue();
            }
            copier.serialize_maybe_ref(raw_dest, body, 0)?;
            return copier.drain_queue();
        }
        if let Some(raw_action) = item_dict.get_raw::<Object<'_>>(A)
            && item_dict
                .get::<Dict<'_>>(A)
                .is_some_and(|a| is_valid_action(src_ctx, &a, 0))
        {
            body.extend_from_slice(b" /A ");
            copier.serialize_maybe_ref(raw_action, body, 0)?;
            return copier.drain_queue();
        }
    }
    if let Some(page_id) = node.fallback_page_id {
        let _ = write!(page_bytes_writer(body), " /Dest [{page_id} 0 R /Fit]");
    }
    Ok(())
}

fn try_write_prefixed_outline_dest(
    pdf: &Pdf,
    raw_dest: &MaybeRef<Object<'_>>,
    prefix: &[u8],
    out: &mut Vec<u8>,
) -> bool {
    let resolved = match raw_dest {
        MaybeRef::NotRef(Object::Name(n)) => Some((true, n.as_ref().to_vec())),
        MaybeRef::NotRef(Object::String(s)) => Some((false, s.as_bytes().to_vec())),
        MaybeRef::Ref(r) => match pdf.xref().get::<Object<'_>>((*r).into()) {
            Some(Object::Name(n)) => Some((true, n.as_ref().to_vec())),
            Some(Object::String(s)) => Some((false, s.as_bytes().to_vec())),
            _ => None,
        },
        _ => None,
    };
    let Some((is_name, raw)) = resolved else {
        return false;
    };
    let mut prefixed = Vec::with_capacity(prefix.len() + raw.len());
    prefixed.extend_from_slice(prefix);
    prefixed.extend_from_slice(&raw);
    if is_name {
        write_pdf_name(&prefixed, out);
    } else {
        write_pdf_literal_string(&prefixed, out);
    }
    true
}

// ---------------------------------------------------------------------------
// Catalog `/Dests` (PDF 1.1) and `/Names` (PDF 1.2+)
// ---------------------------------------------------------------------------

fn emit_catalog_dests(writer: &mut PdfWriterState, ctx: &MetadataExportContext<'_>) {
    let cp = writer.checkpoint();
    let mut body = Vec::new();
    body.extend_from_slice(b"<<");
    let mut count = 0usize;

    for &pdf_idx in ctx.ordered_used_pdfs {
        let Some((pdf, src_ctx, catalog)) = ctx.catalog_of(pdf_idx) else {
            continue;
        };
        let Some(dests_dict) = catalog.get::<Dict<'_>>(DESTS) else {
            continue;
        };
        let Ok(mut copier) = ObjectCopier::new(writer, pdf, src_ctx, pdf_idx) else {
            writer.rollback(cp);
            return;
        };
        copier.strip_struct_parent = !src_ctx.exports_all_pages_uniquely;
        for (key, val) in dests_dict.entries() {
            if !src_ctx.valid_dest_names.contains(key.as_ref()) {
                continue;
            }
            let entry_cp = copier.writer.checkpoint();
            let mut val_buf = Vec::new();
            let ok = copier
                .serialize_maybe_ref(val, &mut val_buf, 0)
                .and_then(|()| copier.drain_queue())
                .is_ok();
            if ok {
                body.push(b' ');
                if let Some(prefix) = src_ctx.dest_prefix.as_deref() {
                    let mut prefixed = Vec::with_capacity(prefix.len() + key.as_ref().len());
                    prefixed.extend_from_slice(prefix);
                    prefixed.extend_from_slice(key.as_ref());
                    write_pdf_name(&prefixed, &mut body);
                } else {
                    write_pdf_name(key.as_ref(), &mut body);
                }
                body.push(b' ');
                body.extend_from_slice(&val_buf);
                count += 1;
            } else {
                copier.writer.rollback(entry_cp);
                copier.queue.clear();
            }
        }
    }

    if count == 0 {
        writer.rollback(cp);
        return;
    }
    body.extend_from_slice(b" >>");
    let dests_id = writer.alloc_obj_id();
    writer.emit_raw_object(dests_id, &body);
    let mut extra = Vec::new();
    let _ = write!(page_bytes_writer(&mut extra), " /Dests {dests_id} 0 R");
    writer.append_catalog_extra(&extra);
}

fn emit_catalog_names(writer: &mut PdfWriterState, ctx: &MetadataExportContext<'_>) {
    let dests_tree_id = emit_merged_names_dests_tree(writer, ctx);
    let sibling_entries = collect_primary_safe_names_subtrees(writer, ctx);
    if dests_tree_id.is_none() && sibling_entries.is_empty() {
        return;
    }
    let mut body = Vec::new();
    body.extend_from_slice(b"<<");
    if let Some(id) = dests_tree_id {
        let _ = write!(page_bytes_writer(&mut body), " /Dests {id} 0 R");
    }
    for (key, val_bytes) in sibling_entries {
        body.push(b' ');
        write_pdf_name(key, &mut body);
        body.push(b' ');
        body.extend_from_slice(&val_bytes);
    }
    body.extend_from_slice(b" >>");
    let names_id = writer.alloc_obj_id();
    writer.emit_raw_object(names_id, &body);
    let mut extra = Vec::new();
    let _ = write!(page_bytes_writer(&mut extra), " /Names {names_id} 0 R");
    writer.append_catalog_extra(&extra);
}

fn emit_merged_names_dests_tree(
    writer: &mut PdfWriterState,
    ctx: &MetadataExportContext<'_>,
) -> Option<usize> {
    let cp = writer.checkpoint();
    let mut entries: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();

    for &pdf_idx in ctx.ordered_used_pdfs {
        let Some((pdf, src_ctx, catalog)) = ctx.catalog_of(pdf_idx) else {
            continue;
        };
        let Some(names_dict) = catalog.get::<Dict<'_>>(NAMES) else {
            continue;
        };
        let Some(dests_tree) = names_dict.get::<Dict<'_>>(DESTS) else {
            continue;
        };
        let mut raw_entries = Vec::new();
        collect_name_tree_raw(pdf, &dests_tree, &mut raw_entries, MAX_NAMED_DESTS);
        if raw_entries.is_empty() {
            continue;
        }
        let Ok(mut copier) = ObjectCopier::new(writer, pdf, src_ctx, pdf_idx) else {
            writer.rollback(cp);
            return None;
        };
        copier.strip_struct_parent = !src_ctx.exports_all_pages_uniquely;
        for (raw_key, val_maybe) in raw_entries {
            if !src_ctx.valid_dest_strings.contains(&raw_key) {
                continue;
            }
            let entry_cp = copier.writer.checkpoint();
            let mut val_buf = Vec::new();
            let ok = copier
                .serialize_maybe_ref(val_maybe, &mut val_buf, 0)
                .and_then(|()| copier.drain_queue())
                .is_ok();
            if ok {
                let final_key = if let Some(prefix) = src_ctx.dest_prefix.as_deref() {
                    let mut p = Vec::with_capacity(prefix.len() + raw_key.len());
                    p.extend_from_slice(prefix);
                    p.extend_from_slice(&raw_key);
                    p
                } else {
                    raw_key
                };
                entries.push((final_key, val_buf));
            } else {
                copier.writer.rollback(entry_cp);
                copier.queue.clear();
            }
        }
    }

    if entries.is_empty() {
        writer.rollback(cp);
        return None;
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries.dedup_by(|a, b| a.0 == b.0);
    Some(write_name_tree(writer, &entries))
}

fn write_name_tree(writer: &mut PdfWriterState, entries: &[(Vec<u8>, Vec<u8>)]) -> usize {
    if entries.len() <= NAME_TREE_LEAF_SIZE {
        let mut body = Vec::new();
        body.extend_from_slice(b"<< /Names [");
        for (idx, (key, val)) in entries.iter().enumerate() {
            if idx > 0 {
                body.push(b' ');
            }
            write_pdf_literal_string(key, &mut body);
            body.push(b' ');
            body.extend_from_slice(val);
        }
        body.extend_from_slice(b"] >>");
        let id = writer.alloc_obj_id();
        writer.emit_raw_object(id, &body);
        return id;
    }

    let mut level: Vec<(usize, &[u8], &[u8])> = Vec::new();
    for chunk in entries.chunks(NAME_TREE_LEAF_SIZE) {
        let first = chunk.first().map_or(&[][..], |(k, _)| k.as_slice());
        let last = chunk.last().map_or(&[][..], |(k, _)| k.as_slice());
        let mut body = Vec::new();
        body.extend_from_slice(b"<< /Limits [");
        write_pdf_literal_string(first, &mut body);
        body.push(b' ');
        write_pdf_literal_string(last, &mut body);
        body.extend_from_slice(b"] /Names [");
        for (idx, (key, val)) in chunk.iter().enumerate() {
            if idx > 0 {
                body.push(b' ');
            }
            write_pdf_literal_string(key, &mut body);
            body.push(b' ');
            body.extend_from_slice(val);
        }
        body.extend_from_slice(b"] >>");
        let leaf_id = writer.alloc_obj_id();
        writer.emit_raw_object(leaf_id, &body);
        level.push((leaf_id, first, last));
    }

    while level.len() > 1 {
        let mut next_level = Vec::new();
        let is_top = level.len() <= NAME_TREE_LEAF_SIZE;
        for chunk in level.chunks(NAME_TREE_LEAF_SIZE) {
            let first = chunk.first().map_or(&[][..], |(_, f, _)| *f);
            let last = chunk.last().map_or(&[][..], |(_, _, l)| *l);
            let mut body = Vec::new();
            body.extend_from_slice(b"<< ");
            if !is_top {
                body.extend_from_slice(b"/Limits [");
                write_pdf_literal_string(first, &mut body);
                body.push(b' ');
                write_pdf_literal_string(last, &mut body);
                body.extend_from_slice(b"] ");
            }
            body.extend_from_slice(b"/Kids [");
            for (idx, (kid_id, _, _)) in chunk.iter().enumerate() {
                if idx > 0 {
                    body.push(b' ');
                }
                let _ = write!(page_bytes_writer(&mut body), "{kid_id} 0 R");
            }
            body.extend_from_slice(b"] >>");
            let node_id = writer.alloc_obj_id();
            writer.emit_raw_object(node_id, &body);
            next_level.push((node_id, first, last));
        }
        level = next_level;
    }

    level[0].0
}

fn collect_primary_safe_names_subtrees(
    writer: &mut PdfWriterState,
    ctx: &MetadataExportContext<'_>,
) -> Vec<(&'static [u8], Vec<u8>)> {
    const SAFE_NAMES_KEYS: &[&[u8]] = &[
        EMBEDDED_FILES,
        AP,
        KEY_URLS,
        KEY_IDS,
        KEY_RENDITIONS,
        KEY_ALTERNATE_PRESENTATIONS,
    ];
    let Some(primary_idx) = ctx.primary_pdf_index else {
        return Vec::new();
    };
    let Some((pdf, src_ctx, catalog)) = ctx.catalog_of(primary_idx) else {
        return Vec::new();
    };
    let Some(names_dict) = catalog.get::<Dict<'_>>(NAMES) else {
        return Vec::new();
    };
    let mut kept = Vec::new();
    for &key in SAFE_NAMES_KEYS {
        let Some(raw_val) = names_dict.get_raw::<Object<'_>>(key) else {
            continue;
        };
        let cp = writer.checkpoint();
        let mut val_buf = Vec::new();
        let ok = ObjectCopier::new(writer, pdf, src_ctx, primary_idx)
            .and_then(|mut copier| {
                copier.strip_struct_parent = !src_ctx.exports_all_pages_uniquely;
                copier.rewrite_named_dests = src_ctx.dest_prefix.is_some();
                copier.serialize_maybe_ref(raw_val, &mut val_buf, 0)?;
                copier.drain_queue()
            })
            .is_ok();
        if ok && !val_buf.is_empty() {
            kept.push((key, val_buf));
        } else {
            writer.rollback(cp);
        }
    }
    kept
}

// ---------------------------------------------------------------------------
// Catalog `/PageLabels`
// ---------------------------------------------------------------------------

#[derive(Clone, PartialEq, Eq)]
enum EffectivePageLabel {
    Labeled {
        style: Option<Vec<u8>>,
        prefix: Option<Vec<u8>>,
        num: usize,
    },
    Unlabeled,
}

fn resolve_effective_page_label(
    backdrop_opt: Option<PdfPageBackdrop>,
    source_contexts: &[Option<SourcePdfContext>],
) -> EffectivePageLabel {
    let Some(backdrop) = backdrop_opt else {
        return EffectivePageLabel::Unlabeled;
    };
    let Some(Some(src_ctx)) = source_contexts.get(backdrop.pdf_index) else {
        return EffectivePageLabel::Unlabeled;
    };
    if src_ctx.page_labels.is_empty() {
        return EffectivePageLabel::Unlabeled;
    }
    let idx = src_ctx
        .page_labels
        .partition_point(|(start_page, _)| *start_page <= backdrop.page_index);
    let Some((rule_start, rule)) = idx.checked_sub(1).and_then(|i| src_ctx.page_labels.get(i))
    else {
        return EffectivePageLabel::Unlabeled;
    };
    let num = rule.start.saturating_add(backdrop.page_index - rule_start);
    EffectivePageLabel::Labeled {
        style: rule.style.clone(),
        prefix: rule.prefix.clone(),
        num,
    }
}

fn write_page_label_dict(label: &EffectivePageLabel, out: &mut Vec<u8>) {
    match label {
        EffectivePageLabel::Unlabeled => out.extend_from_slice(b"<< >>"),
        EffectivePageLabel::Labeled { style, prefix, num } => {
            out.extend_from_slice(b"<<");
            if let Some(st) = style {
                out.extend_from_slice(b" /S ");
                write_pdf_name(st, out);
            }
            if let Some(px) = prefix {
                out.extend_from_slice(b" /P ");
                write_pdf_literal_string(px, out);
            }
            if style.is_some() && *num != 1 {
                let _ = write!(page_bytes_writer(out), " /St {num}");
            }
            out.extend_from_slice(b" >>");
        }
    }
}

fn emit_page_labels(writer: &mut PdfWriterState, ctx: &MetadataExportContext<'_>) {
    if ctx.selected_backdrops.is_empty() {
        return;
    }
    let effective: Vec<EffectivePageLabel> = ctx
        .selected_backdrops
        .iter()
        .map(|&b| resolve_effective_page_label(b, ctx.source_contexts))
        .collect();

    if !effective
        .iter()
        .any(|item| matches!(item, EffectivePageLabel::Labeled { .. }))
    {
        return;
    }

    type PageLabelRunState = (Option<Vec<u8>>, Option<Vec<u8>>, usize);
    let mut nums_body = Vec::new();
    nums_body.extend_from_slice(b"<< /Nums [");
    let mut prev_state: Option<PageLabelRunState> = None;
    let mut prev_unlabeled = false;

    for (out_idx, item) in effective.iter().enumerate() {
        let should_emit = if out_idx == 0 {
            true
        } else {
            match item {
                EffectivePageLabel::Unlabeled => !prev_unlabeled,
                EffectivePageLabel::Labeled { style, prefix, num } => {
                    if let Some((ref ps, ref pp, pn)) = prev_state {
                        !(ps == style
                            && pp == prefix
                            && ((style.is_some() && *num == pn.saturating_add(1))
                                || style.is_none()))
                    } else {
                        true
                    }
                }
            }
        };

        if should_emit {
            if out_idx > 0 {
                nums_body.push(b' ');
            }
            let _ = write!(page_bytes_writer(&mut nums_body), "{out_idx} ");
            write_page_label_dict(item, &mut nums_body);
        }

        match item {
            EffectivePageLabel::Unlabeled => {
                prev_unlabeled = true;
                prev_state = None;
            }
            EffectivePageLabel::Labeled { style, prefix, num } => {
                prev_unlabeled = false;
                prev_state = Some((style.clone(), prefix.clone(), *num));
            }
        }
    }

    nums_body.extend_from_slice(b"] >>");
    let labels_id = writer.alloc_obj_id();
    writer.emit_raw_object(labels_id, &nums_body);
    let mut extra = Vec::new();
    let _ = write!(
        page_bytes_writer(&mut extra),
        " /PageLabels {labels_id} 0 R"
    );
    writer.append_catalog_extra(&extra);
}

// ---------------------------------------------------------------------------
// Catalog scalar/sub-object entries, `/OpenAction`, `/OCProperties`, `/AcroForm`, `/StructTreeRoot`
// ---------------------------------------------------------------------------

fn emit_catalog_properties(
    writer: &mut PdfWriterState,
    ctx: &MetadataExportContext<'_>,
    emitted_outlines: bool,
) {
    const SAFE_CATALOG_KEYS: &[&[u8]] = &[
        METADATA,
        LANG,
        PAGE_LAYOUT,
        VIEWER_PREFERENCES,
        OUTPUT_INTENTS,
        URI,
        PIECE_INFO,
        EXTENSIONS,
        AF,
        COLLECTION,
    ];
    let Some(primary_idx) = ctx.primary_pdf_index else {
        return;
    };
    let Some((pdf, src_ctx, catalog)) = ctx.catalog_of(primary_idx) else {
        return;
    };

    if let Some(ver) = catalog.get::<Name<'_>>(VERSION) {
        let mut extra = Vec::new();
        extra.extend_from_slice(b" /Version ");
        write_pdf_name(ver.as_ref(), &mut extra);
        writer.append_catalog_extra(&extra);
    }

    if let Some(mode) = catalog.get::<Name<'_>>(PAGE_MODE)
        && !(mode.as_ref() == KEY_USE_OUTLINES && !emitted_outlines)
    {
        let mut extra = Vec::new();
        extra.extend_from_slice(b" /PageMode ");
        write_pdf_name(mode.as_ref(), &mut extra);
        writer.append_catalog_extra(&extra);
    }

    for &key in SAFE_CATALOG_KEYS {
        let Some(raw_val) = catalog.get_raw::<Object<'_>>(key) else {
            continue;
        };
        copy_catalog_entry(writer, pdf, src_ctx, key, raw_val);
    }

    if let Some(open_obj) = catalog.get::<Object<'_>>(OPEN_ACTION)
        && let Some(raw_open) = catalog.get_raw::<Object<'_>>(OPEN_ACTION)
    {
        let valid = match &open_obj {
            Object::Dict(act) => is_valid_action(src_ctx, act, 0),
            _ => is_valid_destination(src_ctx, &open_obj),
        };
        if valid {
            copy_catalog_entry(writer, pdf, src_ctx, OPEN_ACTION, raw_open);
        }
    }
}

fn copy_catalog_entry(
    writer: &mut PdfWriterState,
    pdf: &Pdf,
    src_ctx: &SourcePdfContext,
    key: &[u8],
    raw_val: MaybeRef<Object<'_>>,
) {
    let cp = writer.checkpoint();
    let mut val_buf = Vec::new();
    let ok = ObjectCopier::new(writer, pdf, src_ctx, src_ctx.pdf_idx)
        .and_then(|mut copier| {
            copier.strip_struct_parent = !src_ctx.exports_all_pages_uniquely;
            copier.rewrite_named_dests = src_ctx.dest_prefix.is_some();
            copier.serialize_maybe_ref(raw_val, &mut val_buf, 0)?;
            copier.drain_queue()
        })
        .is_ok();
    if ok && !val_buf.is_empty() {
        let mut extra = Vec::with_capacity(key.len() + val_buf.len() + 4);
        extra.push(b' ');
        write_pdf_name(key, &mut extra);
        extra.push(b' ');
        extra.extend_from_slice(&val_buf);
        writer.append_catalog_extra(&extra);
    } else {
        writer.rollback(cp);
    }
}

fn emit_oc_properties(writer: &mut PdfWriterState, ctx: &MetadataExportContext<'_>) {
    let mut sources: Vec<(usize, &Pdf, &SourcePdfContext, Dict<'_>)> = Vec::new();
    for &pdf_idx in ctx.ordered_used_pdfs {
        if let Some((pdf, src_ctx, catalog)) = ctx.catalog_of(pdf_idx)
            && let Some(ocp) = catalog.get::<Dict<'_>>(OCPROPERTIES)
        {
            sources.push((pdf_idx, pdf, src_ctx, ocp));
        }
    }
    if sources.is_empty() {
        return;
    }
    if sources.len() == 1 {
        let (pdf_idx, pdf, src_ctx, _) = sources[0];
        if let Some(catalog) = pdf.xref().get::<Dict<'_>>(pdf.xref().root_id())
            && let Some(raw_ocp) = catalog.get_raw::<Object<'_>>(OCPROPERTIES)
        {
            copy_catalog_entry(writer, pdf, src_ctx, OCPROPERTIES, raw_ocp);
        }
        let _ = pdf_idx;
        return;
    }

    let cp = writer.checkpoint();
    let mut all_ocgs = Vec::new();
    let mut d_on = Vec::new();
    let mut d_off = Vec::new();
    let mut d_order = Vec::new();
    let mut d_rbgroups = Vec::new();
    let mut d_locked = Vec::new();
    let mut d_as = Vec::new();
    let mut base_state: Option<Vec<u8>> = None;
    let mut d_name: Option<Vec<u8>> = None;
    let mut all_configs = Vec::new();

    for (pdf_idx, pdf, src_ctx, ocp) in sources {
        let Ok(mut copier) = ObjectCopier::new(writer, pdf, src_ctx, pdf_idx) else {
            writer.rollback(cp);
            return;
        };
        if let Some(ocgs) = ocp.get::<Array<'_>>(OCGS)
            && append_array_elements_via_copier(&mut copier, &ocgs, &mut all_ocgs).is_err()
        {
            writer.rollback(cp);
            return;
        }
        if let Some(d_dict) = ocp.get::<Dict<'_>>(D) {
            if base_state.is_none()
                && let Some(bs) = d_dict.get::<Name<'_>>(BASE_STATE)
            {
                base_state = Some(bs.as_ref().to_vec());
            }
            if d_name.is_none()
                && let Some(nm) = d_dict.get::<object::String<'_>>(NAME)
            {
                d_name = Some(nm.as_bytes().to_vec());
            }
            let merges: [(&[u8], &mut Vec<u8>); 6] = [
                (ON, &mut d_on),
                (OFF, &mut d_off),
                (ORDER, &mut d_order),
                (KEY_RB_GROUPS, &mut d_rbgroups),
                (KEY_LOCKED, &mut d_locked),
                (AS, &mut d_as),
            ];
            for (k, dst) in merges {
                if let Some(arr) = d_dict.get::<Array<'_>>(k)
                    && append_array_elements_via_copier(&mut copier, &arr, dst).is_err()
                {
                    writer.rollback(cp);
                    return;
                }
            }
        }
        if let Some(configs) = ocp.get::<Array<'_>>(KEY_CONFIGS)
            && append_array_elements_via_copier(&mut copier, &configs, &mut all_configs).is_err()
        {
            writer.rollback(cp);
            return;
        }
        if copier.drain_queue().is_err() {
            writer.rollback(cp);
            return;
        }
    }

    if all_ocgs.is_empty() {
        writer.rollback(cp);
        return;
    }
    let mut body = Vec::with_capacity(512);
    body.extend_from_slice(b"<< /OCGs [");
    body.extend_from_slice(&all_ocgs);
    body.extend_from_slice(b"] /D <<");
    if let Some(nm) = d_name {
        body.extend_from_slice(b" /Name ");
        write_pdf_literal_string(&nm, &mut body);
    }
    if let Some(bs) = base_state {
        body.extend_from_slice(b" /BaseState ");
        write_pdf_name(&bs, &mut body);
    }
    append_nonempty_named_array(ON, &d_on, &mut body);
    append_nonempty_named_array(OFF, &d_off, &mut body);
    append_nonempty_named_array(ORDER, &d_order, &mut body);
    append_nonempty_named_array(KEY_RB_GROUPS, &d_rbgroups, &mut body);
    append_nonempty_named_array(KEY_LOCKED, &d_locked, &mut body);
    append_nonempty_named_array(AS, &d_as, &mut body);
    body.extend_from_slice(b" >>");
    append_nonempty_named_array(KEY_CONFIGS, &all_configs, &mut body);
    body.extend_from_slice(b" >>");

    let ocp_id = writer.alloc_obj_id();
    writer.emit_raw_object(ocp_id, &body);
    let mut extra = Vec::new();
    let _ = write!(page_bytes_writer(&mut extra), " /OCProperties {ocp_id} 0 R");
    writer.append_catalog_extra(&extra);
}

fn append_array_elements_via_copier(
    copier: &mut ObjectCopier<'_, '_, '_>,
    arr: &Array<'_>,
    dst: &mut Vec<u8>,
) -> Result<(), ()> {
    for item in arr.raw_iter() {
        let mut elem = Vec::new();
        copier.serialize_maybe_ref(item, &mut elem, 0)?;
        if !dst.is_empty() {
            dst.push(b' ');
        }
        dst.extend_from_slice(&elem);
    }
    Ok(())
}

fn append_nonempty_named_array(name: &[u8], content: &[u8], out: &mut Vec<u8>) {
    if content.is_empty() {
        return;
    }
    out.push(b' ');
    write_pdf_name(name, out);
    out.extend_from_slice(b" [");
    out.extend_from_slice(content);
    out.push(b']');
}

// ---------------------------------------------------------------------------
// Catalog `/AcroForm` (filtered to surviving non-signature fields)
// ---------------------------------------------------------------------------

fn emit_acro_form(writer: &mut PdfWriterState, ctx: &MetadataExportContext<'_>) {
    let Some(primary_idx) = ctx.primary_pdf_index else {
        return;
    };
    let Some((pdf, src_ctx, catalog)) = ctx.catalog_of(primary_idx) else {
        return;
    };
    let Some(acro) = catalog.get::<Dict<'_>>(ACRO_FORM) else {
        return;
    };
    let Some(fields_arr) = acro.get::<Array<'_>>(FIELDS) else {
        return;
    };

    let cp = writer.checkpoint();
    let mut copier = match ObjectCopier::new(writer, pdf, src_ctx, primary_idx) {
        Ok(c) => c,
        Err(()) => {
            writer.rollback(cp);
            return;
        }
    };
    copier.strip_struct_parent = !src_ctx.exports_all_pages_uniquely;
    copier.rewrite_named_dests = src_ctx.dest_prefix.is_some();

    let mut kept_field_id_map: HashMap<ObjectIdentifier, usize> = HashMap::new();
    let mut visited_fields: HashSet<ObjectIdentifier> = HashSet::new();
    let mut emitted_top_fields = Vec::new();

    for item in fields_arr.raw_iter() {
        let Ok(Some(rendered)) = filter_and_emit_acro_field(
            &mut copier,
            item,
            0,
            &mut visited_fields,
            &mut kept_field_id_map,
        ) else {
            continue;
        };
        if !emitted_top_fields.is_empty() {
            emitted_top_fields.push(b' ');
        }
        emitted_top_fields.extend_from_slice(&rendered);
    }

    if emitted_top_fields.is_empty() {
        drop(copier);
        writer.rollback(cp);
        return;
    }

    let mut acro_body = Vec::with_capacity(256);
    acro_body.extend_from_slice(b"<< /Fields [");
    acro_body.extend_from_slice(&emitted_top_fields);
    acro_body.push(b']');

    if let Some(co_arr) = acro.get::<Array<'_>>(CO) {
        let mut co_buf = Vec::new();
        for co_item in co_arr.raw_iter() {
            if let MaybeRef::Ref(r) = co_item
                && let Some(&mapped_id) = kept_field_id_map.get(&r.into())
            {
                if !co_buf.is_empty() {
                    co_buf.push(b' ');
                }
                let _ = write!(page_bytes_writer(&mut co_buf), "{mapped_id} 0 R");
            }
        }
        append_nonempty_named_array(CO, &co_buf, &mut acro_body);
    }

    for (key, val) in acro.entries() {
        let k = key.as_ref();
        if k == FIELDS || k == CO || k == SIG_FLAGS || k == XFA || k == KEY_ENCRYPT {
            continue;
        }
        let ok = {
            let mut v_buf = Vec::new();
            let res = copier.serialize_maybe_ref(val, &mut v_buf, 0);
            if res.is_ok() {
                acro_body.push(b' ');
                write_pdf_name(k, &mut acro_body);
                acro_body.push(b' ');
                acro_body.extend_from_slice(&v_buf);
                true
            } else {
                false
            }
        };
        if !ok {
            drop(copier);
            writer.rollback(cp);
            return;
        }
    }

    if copier.drain_queue().is_err() {
        drop(copier);
        writer.rollback(cp);
        return;
    }
    drop(copier);

    acro_body.extend_from_slice(b" >>");
    let acro_id = writer.alloc_obj_id();
    writer.emit_raw_object(acro_id, &acro_body);
    let mut extra = Vec::new();
    let _ = write!(page_bytes_writer(&mut extra), " /AcroForm {acro_id} 0 R");
    writer.append_catalog_extra(&extra);
}

fn filter_and_emit_acro_field<'a>(
    copier: &mut ObjectCopier<'_, 'a, '_>,
    item: MaybeRef<Object<'a>>,
    depth: usize,
    visited: &mut HashSet<ObjectIdentifier>,
    kept_field_id_map: &mut HashMap<ObjectIdentifier, usize>,
) -> Result<Option<Vec<u8>>, ()> {
    if depth >= 32 {
        return Ok(None);
    }
    let (src_id_opt, dict) = match &item {
        MaybeRef::Ref(r) => {
            let id: ObjectIdentifier = (*r).into();
            if !visited.insert(id) {
                return Ok(None);
            }
            let Some(d) = copier.pdf.xref().get::<Dict<'a>>(id) else {
                return Ok(None);
            };
            (Some(id), d)
        }
        MaybeRef::NotRef(Object::Dict(d)) => (None, d.clone()),
        MaybeRef::NotRef(_) => return Ok(None),
    };

    if is_signature_widget(&dict)
        || dict
            .get::<Name<'_>>(FT)
            .is_some_and(|ft| ft.as_ref() == SIG)
    {
        return Ok(None);
    }

    if let Some(kids) = dict.get::<Array<'a>>(KIDS) {
        let mut kept_kids = Vec::new();
        for kid in kids.raw_iter() {
            if let Some(rendered) =
                filter_and_emit_acro_field(copier, kid, depth + 1, visited, kept_field_id_map)?
            {
                if !kept_kids.is_empty() {
                    kept_kids.push(b' ');
                }
                kept_kids.extend_from_slice(&rendered);
            }
        }
        if kept_kids.is_empty() {
            return Ok(None);
        }
        let mut body = Vec::new();
        body.extend_from_slice(b"<< /Kids [");
        body.extend_from_slice(&kept_kids);
        body.push(b']');
        for (key, val) in dict.entries() {
            let k = key.as_ref();
            if k == KIDS || k == KEY_ENCRYPT || (copier.strip_struct_parent && k == STRUCT_PARENT) {
                continue;
            }
            body.push(b' ');
            write_pdf_name(k, &mut body);
            body.push(b' ');
            copier.serialize_maybe_ref(val, &mut body, 0)?;
        }
        body.extend_from_slice(b" >>");
        return if let Some(src_id) = src_id_opt {
            let dest_id = copier.writer.alloc_obj_id();
            copier.writer.emit_raw_object(dest_id, &body);
            kept_field_id_map.insert(src_id, dest_id);
            Ok(Some(format!("{dest_id} 0 R").into_bytes()))
        } else {
            Ok(Some(body))
        };
    }

    if !is_acro_leaf_field_surviving(copier, src_id_opt, &dict) {
        return Ok(None);
    }

    if let Some(src_id) = src_id_opt {
        let dest_id = copier.enqueue_ref(src_id)?;
        kept_field_id_map.insert(src_id, dest_id);
        Ok(Some(format!("{dest_id} 0 R").into_bytes()))
    } else {
        let mut body = Vec::new();
        copier.serialize_dict(&dict, &mut body, 0, true)?;
        Ok(Some(body))
    }
}

fn is_acro_leaf_field_surviving(
    copier: &ObjectCopier<'_, '_, '_>,
    src_id_opt: Option<ObjectIdentifier>,
    dict: &Dict<'_>,
) -> bool {
    if let Some(p_ref) = dict.get_ref(P) {
        return copier
            .src_ctx
            .page_id_map
            .get(&p_ref.into())
            .copied()
            .flatten()
            .is_some();
    }
    let is_widget = dict
        .get::<Name<'_>>(SUBTYPE)
        .is_some_and(|st| st.as_ref() == WIDGET)
        || dict.contains_key(RECT);
    if !is_widget {
        return true;
    }
    src_id_opt.is_some_and(|id| copier.writer.lookup_remapped(copier.pdf_idx, id).is_some())
}

// ---------------------------------------------------------------------------
// Catalog `/StructTreeRoot` & `/MarkInfo`
// ---------------------------------------------------------------------------

fn emit_struct_tree(writer: &mut PdfWriterState, ctx: &MetadataExportContext<'_>) {
    let Some(primary_idx) = ctx.primary_pdf_index else {
        return;
    };
    let Some((pdf, src_ctx, catalog)) = ctx.catalog_of(primary_idx) else {
        return;
    };
    if !(src_ctx.exports_all_pages_uniquely
        && src_ctx.all_exported_pages_passthrough
        && !src_ctx.any_annot_dropped)
    {
        return;
    }
    let Some(raw_struct_root) = catalog.get_raw::<Object<'_>>(STRUCT_TREE_ROOT) else {
        return;
    };

    let cp = writer.checkpoint();
    let mut struct_buf = Vec::new();
    let mut mark_buf = Vec::new();
    let raw_mark = catalog.get_raw::<Object<'_>>(MARK_INFO);

    let ok = ObjectCopier::new(writer, pdf, src_ctx, primary_idx)
        .and_then(|mut copier| {
            copier.fail_on_unexported_page_ref = true;
            copier.strip_struct_parent = false;
            copier.rewrite_named_dests = false;
            copier.serialize_maybe_ref(raw_struct_root, &mut struct_buf, 0)?;
            if let Some(mark_val) = raw_mark {
                copier.serialize_maybe_ref(mark_val, &mut mark_buf, 0)?;
            }
            copier.drain_queue()
        })
        .is_ok();

    if !ok || struct_buf.is_empty() {
        writer.rollback(cp);
        return;
    }

    let mut extra = Vec::new();
    extra.extend_from_slice(b" /StructTreeRoot ");
    extra.extend_from_slice(&struct_buf);
    if !mark_buf.is_empty() {
        extra.extend_from_slice(b" /MarkInfo ");
        extra.extend_from_slice(&mark_buf);
    }
    writer.append_catalog_extra(&extra);
}
