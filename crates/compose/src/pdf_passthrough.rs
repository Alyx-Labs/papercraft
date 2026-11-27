//! Vector PDF pass-through composer built on `hayro-syntax` object copying.
//!
//! Copies only the indirect objects reachable from selected pages without decoding or
//! rasterizing existing content, image, or font streams. Unannotated pages pass through
//! directly; annotated pages wrap original `/Contents` in `q`/`Q` and place a PaperCraft
//! Form XObject overlay mapped cleanly via the inverse of `Page::initial_transform(true)`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::Write as _;

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::dict::keys::{
    AF, ANNOTS, ART_BOX, BLEED_BOX, COLORSPACE, CONTENTS, D, DECODE_PARMS, DEST, DP, DUR,
    EXT_G_STATE, F, FILTER, FONT, GROUP, LAST_MODIFIED, LENGTH, METADATA, PATTERN, PIECE_INFO,
    PROPERTIES, RESOURCES, S, SHADING, STRUCT_PARENT, STRUCT_PARENTS, TRANS, TRIM_BOX, TYPE,
    USER_UNIT, VP, XOBJECT,
};
use hayro::hayro_syntax::object::{
    Array, Dict, MaybeRef, Name, Number, Object, ObjectIdentifier, Rect, Stream,
};
use hayro::hayro_syntax::page::{Page, Resources, Rotation};
use papercraft_doc::{NotebookPage, PdfPageBackdrop};

use crate::export::{
    PdfImageEntry, PdfWriterState, append_pdf_overlay_body, format_pdf_overlay_resources,
    prepare_pdf_page_images,
};

const MAX_COPIED_OBJECTS_PER_PDF: usize = 250_000;
pub(crate) const MAX_NESTING_DEPTH: usize = 64;

pub(crate) const KEY_ENCRYPT: &[u8] = b"Encrypt";
const KEY_CRYPT: &[u8] = b"Crypt";
const KEY_BOX_COLOR_INFO: &[u8] = b"BoxColorInfo";
const KEY_SEPARATION_INFO: &[u8] = b"SeparationInfo";
const KEY_PZ: &[u8] = b"PZ";
const KEY_TABS: &[u8] = b"Tabs";

/// Parsed `/PageLabels` range rule from a source PDF's catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct PageLabelRule {
    pub(crate) style: Option<Vec<u8>>,
    pub(crate) prefix: Option<Vec<u8>>,
    pub(crate) start: usize,
}

/// Per-source-PDF mapping and validation context shared across page pass-through and metadata export.
pub(crate) struct SourcePdfContext {
    pub(crate) pdf_idx: usize,
    pub(crate) dest_prefix: Option<Vec<u8>>,
    pub(crate) page_id_map: HashMap<ObjectIdentifier, Option<usize>>,
    pub(crate) page_index_map: HashMap<usize, usize>,
    pub(crate) first_exported_dest_page_id: Option<usize>,
    pub(crate) valid_dest_names: HashSet<Vec<u8>>,
    pub(crate) valid_dest_strings: HashSet<Vec<u8>>,
    pub(crate) page_labels: Vec<(usize, PageLabelRule)>,
    pub(crate) exports_all_pages_uniquely: bool,
    pub(crate) all_exported_pages_passthrough: bool,
    pub(crate) any_annot_dropped: bool,
    pub(crate) dropped_annot_ids: HashSet<ObjectIdentifier>,
}

/// Builds the page-remapping and metadata validation context for `pdf_idx`.
pub(crate) fn build_source_pdf_context(
    pdf_idx: usize,
    pdf: &Pdf,
    selected_backdrops: &[Option<PdfPageBackdrop>],
    primary_pdf_index: Option<usize>,
    multi_pdf: bool,
) -> SourcePdfContext {
    let dest_prefix = if multi_pdf && primary_pdf_index != Some(pdf_idx) {
        Some(format!("pc{pdf_idx}_").into_bytes())
    } else {
        None
    };

    let mut page_index_map: HashMap<usize, usize> = HashMap::new();
    let mut page_counts: HashMap<usize, usize> = HashMap::new();
    let mut first_exported_dest_page_id: Option<usize> = None;

    for (out_idx, backdrop_opt) in selected_backdrops.iter().enumerate() {
        let Some(backdrop) = backdrop_opt else {
            continue;
        };
        if backdrop.pdf_index != pdf_idx {
            continue;
        }
        let dest_page_id = 3 + out_idx;
        if first_exported_dest_page_id.is_none() {
            first_exported_dest_page_id = Some(dest_page_id);
        }
        page_index_map
            .entry(backdrop.page_index)
            .or_insert(dest_page_id);
        *page_counts.entry(backdrop.page_index).or_insert(0) += 1;
    }

    let raw_pages = pdf.pages();
    let src_page_count = raw_pages.len();
    let mut page_id_map: HashMap<ObjectIdentifier, Option<usize>> =
        HashMap::with_capacity(src_page_count);
    for (src_idx, page) in raw_pages.iter().enumerate() {
        if let Some(src_id) = page.raw().obj_id() {
            page_id_map.insert(src_id, page_index_map.get(&src_idx).copied());
        }
    }

    let exports_all_pages_uniquely = src_page_count > 0
        && !multi_pdf
        && selected_backdrops.len() == src_page_count
        && page_counts.len() == src_page_count
        && page_counts.values().all(|&c| c == 1);

    let (valid_dest_names, valid_dest_strings) =
        crate::pdf_metadata::collect_valid_named_dests(pdf, &page_id_map, &page_index_map);
    let page_labels = crate::pdf_metadata::parse_page_labels(pdf);

    SourcePdfContext {
        pdf_idx,
        dest_prefix,
        page_id_map,
        page_index_map,
        first_exported_dest_page_id,
        valid_dest_names,
        valid_dest_strings,
        page_labels,
        exports_all_pages_uniquely,
        all_exported_pages_passthrough: true,
        any_annot_dropped: false,
        dropped_annot_ids: HashSet::new(),
    }
}

struct PageEmitTarget<'a> {
    page_obj_id: usize,
    src_page: &'a Page<'a>,
    page: &'a NotebookPage,
    backdrop: PdfPageBackdrop,
    overlay_matrix: Option<[f64; 6]>,
}

/// Attempts to emit a PDF-backed `page` using lossless vector pass-through at `page_obj_id`.
///
/// Returns `Ok(())` when the page is completely emitted into `writer`. Returns `Err(())`
/// (with `writer` atomically rolled back and `page_obj_id`'s offset cleared) on any unexpected
/// PDF structure so the caller can transparently fall back to single-page streaming rasterization.
pub(crate) fn try_emit_pdf_backdrop_page(
    writer: &mut PdfWriterState,
    pdf: &Pdf,
    src_ctx: &mut SourcePdfContext,
    page_obj_id: usize,
    page: &NotebookPage,
    backdrop: PdfPageBackdrop,
) -> Result<(), ()> {
    let Some(src_page) = pdf.pages().get(backdrop.page_index) else {
        writer.clear_offset(page_obj_id);
        return Err(());
    };

    let overlay_matrix = if page.has_annotations() {
        let Some(m) = compute_overlay_pc_to_pdf_matrix(src_page, page) else {
            writer.clear_offset(page_obj_id);
            return Err(());
        };
        Some(m)
    } else {
        None
    };

    let target = PageEmitTarget {
        page_obj_id,
        src_page,
        page,
        backdrop,
        overlay_matrix,
    };
    let checkpoint = writer.checkpoint();
    let res = emit_pdf_backdrop_page_inner(writer, pdf, src_ctx, &target);
    if res.is_err() {
        writer.rollback(checkpoint);
        writer.clear_offset(page_obj_id);
    }
    res
}

fn emit_pdf_backdrop_page_inner(
    writer: &mut PdfWriterState,
    pdf: &Pdf,
    src_ctx: &mut SourcePdfContext,
    target: &PageEmitTarget<'_>,
) -> Result<(), ()> {
    let orig_contents = resolve_original_contents(pdf, target.src_page)?;
    let kept_annots = classify_page_annots(src_ctx, target.src_page);

    let (overlay_xobj_id, overlay_invoke_id, overlay_name) = if let Some(matrix) =
        target.overlay_matrix
    {
        let overlay_name = pick_free_xobject_name(target.src_page);

        // Allocate image object IDs from `writer` without interleaving writes.
        let mut images = prepare_pdf_page_images(target.page, &mut 0);
        assign_writer_image_ids(writer, &mut images);

        let (overlay_stream, states) = append_pdf_overlay_body(target.page, &images);
        let overlay_res = format_pdf_overlay_resources(&states, None, &images);

        let xobj_id = writer.alloc_obj_id();
        let w = target.page.width.clamp(64, 8192);
        let h = target.page.height.clamp(64, 8192);
        let form_extra = format!(
            "/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 {w} {h}] /Resources {overlay_res} "
        );
        writer.emit_stream_object(xobj_id, &form_extra, &overlay_stream);
        writer.emit_image_entries(&images);
        drop(images);

        let invoke_id = writer.alloc_obj_id();
        let invoke_prefix = if orig_contents.is_empty() { "" } else { "Q\n" };
        let [a, b, c, d, e, f] = matrix;
        let invoke_body = format!(
            "{invoke_prefix}q {} {} {} {} {} {} cm /{overlay_name} Do Q\n",
            fmt_affine_coeff(a),
            fmt_affine_coeff(b),
            fmt_affine_coeff(c),
            fmt_affine_coeff(d),
            fmt_affine_coeff(e),
            fmt_affine_coeff(f),
        );
        writer.emit_stream_object(invoke_id, "", invoke_body.as_bytes());
        (Some(xobj_id), Some(invoke_id), Some(overlay_name))
    } else {
        (None, None, None)
    };

    let current_page_remap = target
        .src_page
        .raw()
        .obj_id()
        .map(|id| (id, target.page_obj_id));
    let mut copier = ObjectCopier::new(writer, pdf, src_ctx, target.backdrop.pdf_index)?;
    copier.current_page_remap = current_page_remap;
    copier.strip_struct_parent = !src_ctx.exports_all_pages_uniquely;
    copier.rewrite_named_dests = src_ctx.dest_prefix.is_some();

    let media_box = target.src_page.media_box();
    let crop_box = target.src_page.intersected_crop_box();
    let orig_rot_deg = match target.src_page.rotation() {
        Rotation::None => 0,
        Rotation::Horizontal => 90,
        Rotation::Flipped => 180,
        Rotation::FlippedHorizontal => 270,
    };
    let final_rot_deg = (orig_rot_deg + i32::from(target.page.backdrop_rotation % 4) * 90) % 360;

    let mut page_bytes = Vec::with_capacity(512);
    page_bytes.extend_from_slice(b"<< /Type /Page /Parent 2 0 R /MediaBox ");
    write_rect(&mut page_bytes, media_box);
    page_bytes.extend_from_slice(b" /CropBox ");
    write_rect(&mut page_bytes, crop_box);
    if final_rot_deg != 0 {
        let _ = write!(
            page_bytes_writer(&mut page_bytes),
            " /Rotate {final_rot_deg}"
        );
    }

    append_safe_page_extras(&mut copier, target.src_page, &mut page_bytes)?;
    append_page_resources(
        &mut copier,
        target.src_page,
        overlay_name.as_deref(),
        overlay_xobj_id,
        &mut page_bytes,
    )?;
    append_page_contents(
        &mut copier,
        target.src_page,
        &orig_contents,
        overlay_invoke_id,
        &mut page_bytes,
    )?;
    copier.drain_queue()?;
    let failed_annots = append_page_annots(&mut copier, &kept_annots, &mut page_bytes);
    page_bytes.extend_from_slice(b" >>");

    copier
        .writer
        .emit_raw_object(target.page_obj_id, &page_bytes);
    drop(copier);

    if !failed_annots.is_empty() {
        src_ctx.any_annot_dropped = true;
        for id in failed_annots.into_iter().flatten() {
            src_ctx.dropped_annot_ids.insert(id);
        }
    }
    Ok(())
}

fn assign_writer_image_ids(writer: &mut PdfWriterState, images: &mut [PdfImageEntry]) {
    for entry in images {
        entry.obj_id = writer.alloc_obj_id();
        if entry.alpha_bytes.is_some() {
            entry.smask_obj_id = Some(writer.alloc_obj_id());
        } else {
            entry.smask_obj_id = None;
        }
    }
}

/// Helper adapter allowing `std::fmt::Write` directly onto a `Vec<u8>` buffer for ASCII metadata.
pub(crate) struct ByteFmtWriter<'a>(pub(crate) &'a mut Vec<u8>);

impl std::fmt::Write for ByteFmtWriter<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

pub(crate) fn page_bytes_writer(buf: &mut Vec<u8>) -> ByteFmtWriter<'_> {
    ByteFmtWriter(buf)
}

pub(crate) fn compute_overlay_pc_to_pdf_matrix(
    src_page: &Page<'_>,
    page: &NotebookPage,
) -> Option<[f64; 6]> {
    let (raw_w, raw_h) = src_page.render_dimensions();
    let rw = if raw_w.is_finite() && raw_w > 1.0 {
        f64::from(raw_w)
    } else {
        595.0
    };
    let rh = if raw_h.is_finite() && raw_h > 1.0 {
        f64::from(raw_h)
    } else {
        842.0
    };
    let w = f64::from(page.width.clamp(64, 8192));
    let h = f64::from(page.height.clamp(64, 8192));
    let t_norm_and_rot = match page.backdrop_rotation % 4 {
        1 => [0.0, h / rw, -w / rh, 0.0, w, 0.0],
        2 => [-w / rw, 0.0, 0.0, -h / rh, w, h],
        3 => [0.0, -h / rw, w / rh, 0.0, 0.0, h],
        _ => [w / rw, 0.0, 0.0, h / rh, 0.0, 0.0],
    };
    let t_init = src_page.initial_transform(true).as_coeffs();
    let t_pdf_to_pc = mul_affine(t_norm_and_rot, t_init);
    invert_affine(t_pdf_to_pc)
}

fn mul_affine(m1: [f64; 6], m2: [f64; 6]) -> [f64; 6] {
    [
        m1[0] * m2[0] + m1[2] * m2[1],
        m1[1] * m2[0] + m1[3] * m2[1],
        m1[0] * m2[2] + m1[2] * m2[3],
        m1[1] * m2[2] + m1[3] * m2[3],
        m1[0] * m2[4] + m1[2] * m2[5] + m1[4],
        m1[1] * m2[4] + m1[3] * m2[5] + m1[5],
    ]
}

fn invert_affine(m: [f64; 6]) -> Option<[f64; 6]> {
    let [a, b, c, d, e, f] = m;
    let det = a * d - b * c;
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let out = [
        d * inv,
        -b * inv,
        -c * inv,
        a * inv,
        (c * f - d * e) * inv,
        (b * e - a * f) * inv,
    ];
    out.iter().all(|v| v.is_finite()).then_some(out)
}

fn fmt_affine_coeff(v: f64) -> String {
    let mut buf = Vec::with_capacity(24);
    format_pdf_f64(v, &mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

fn resolve_original_contents(pdf: &Pdf, src_page: &Page<'_>) -> Result<Vec<ObjectIdentifier>, ()> {
    let Some(raw_contents) = src_page.raw().get_raw::<Object<'_>>(CONTENTS) else {
        return Ok(Vec::new());
    };
    match raw_contents {
        MaybeRef::Ref(r) => {
            let id: ObjectIdentifier = r.into();
            match pdf.xref().get::<Object<'_>>(id) {
                Some(Object::Stream(_)) => Ok(vec![id]),
                Some(Object::Array(arr)) => extract_stream_refs_from_array(&arr),
                _ => Err(()),
            }
        }
        MaybeRef::NotRef(Object::Array(arr)) => extract_stream_refs_from_array(&arr),
        _ => Err(()),
    }
}

fn extract_stream_refs_from_array(arr: &Array<'_>) -> Result<Vec<ObjectIdentifier>, ()> {
    let mut out = Vec::new();
    for item in arr.raw_iter() {
        let MaybeRef::Ref(r) = item else {
            return Err(());
        };
        out.push(r.into());
    }
    Ok(out)
}

fn pick_free_xobject_name(src_page: &Page<'_>) -> String {
    let explicit_xobj = src_page
        .raw()
        .get::<Dict<'_>>(RESOURCES)
        .and_then(|res| res.get::<Dict<'_>>(XOBJECT));
    let in_xobjects = |candidate: &[u8]| -> bool {
        src_page.resources().x_objects.contains_key(candidate)
            || explicit_xobj
                .as_ref()
                .is_some_and(|d| d.contains_key(candidate))
    };
    if !in_xobjects(b"PCOv") {
        return "PCOv".to_string();
    }
    for idx in 0usize..10_000 {
        let candidate = format!("PCOv{idx}");
        if !in_xobjects(candidate.as_bytes()) {
            return candidate;
        }
    }
    "PCOvOverlay".to_string()
}

enum KeptAnnot<'a> {
    Ref(ObjectIdentifier),
    Direct(Object<'a>),
}

fn classify_page_annots<'a>(
    src_ctx: &mut SourcePdfContext,
    src_page: &Page<'a>,
) -> Vec<KeptAnnot<'a>> {
    let Some(arr) = src_page.raw().get::<Array<'a>>(ANNOTS) else {
        return Vec::new();
    };
    let mut kept = Vec::new();
    for item in arr.raw_iter() {
        match item {
            MaybeRef::Ref(r) => {
                let id: ObjectIdentifier = r.into();
                if let Some(dict) = src_page.xref().get::<Dict<'a>>(id)
                    && crate::pdf_metadata::should_keep_annotation(src_ctx, &dict)
                {
                    kept.push(KeptAnnot::Ref(id));
                } else {
                    src_ctx.any_annot_dropped = true;
                    src_ctx.dropped_annot_ids.insert(id);
                }
            }
            MaybeRef::NotRef(obj) => {
                if let Object::Dict(ref dict) = obj
                    && crate::pdf_metadata::should_keep_annotation(src_ctx, dict)
                {
                    kept.push(KeptAnnot::Direct(obj));
                } else {
                    src_ctx.any_annot_dropped = true;
                }
            }
        }
    }
    kept
}

fn append_page_annots(
    copier: &mut ObjectCopier<'_, '_, '_>,
    kept_annots: &[KeptAnnot<'_>],
    out: &mut Vec<u8>,
) -> Vec<Option<ObjectIdentifier>> {
    if kept_annots.is_empty() {
        return Vec::new();
    }
    let mut annots_buf = Vec::with_capacity(kept_annots.len() * 16);
    let mut emitted = 0usize;
    let mut failed = Vec::new();

    for item in kept_annots {
        let cp = copier.writer.checkpoint();
        let mut single = Vec::new();
        let ok = match item {
            KeptAnnot::Ref(src_id) => {
                if copier.src_ctx.dropped_annot_ids.contains(src_id) {
                    false
                } else {
                    copier
                        .enqueue_ref(*src_id)
                        .and_then(|dest_id| {
                            copier.drain_queue()?;
                            let _ = write!(page_bytes_writer(&mut single), "{dest_id} 0 R");
                            Ok(())
                        })
                        .is_ok()
                }
            }
            KeptAnnot::Direct(obj) => copier
                .serialize_object(obj.clone(), &mut single, 0)
                .and_then(|()| copier.drain_queue())
                .is_ok(),
        };
        if ok && !single.is_empty() {
            if emitted > 0 {
                annots_buf.push(b' ');
            }
            annots_buf.extend_from_slice(&single);
            emitted += 1;
        } else {
            copier.writer.rollback(cp);
            copier.queue.clear();
            let id_opt = match item {
                KeptAnnot::Ref(id) => Some(*id),
                KeptAnnot::Direct(_) => None,
            };
            failed.push(id_opt);
        }
    }

    if emitted > 0 {
        out.extend_from_slice(b" /Annots [");
        out.extend_from_slice(&annots_buf);
        out.push(b']');
    }
    failed
}

fn append_safe_page_extras(
    copier: &mut ObjectCopier<'_, '_, '_>,
    src_page: &Page<'_>,
    out: &mut Vec<u8>,
) -> Result<(), ()> {
    const SAFE_EXTRA_KEYS: &[&[u8]] = &[
        BLEED_BOX,
        TRIM_BOX,
        ART_BOX,
        GROUP,
        USER_UNIT,
        METADATA,
        TRANS,
        DUR,
        KEY_TABS,
        KEY_BOX_COLOR_INFO,
        KEY_SEPARATION_INFO,
        KEY_PZ,
        VP,
        PIECE_INFO,
        LAST_MODIFIED,
        AF,
    ];
    let raw_dict = src_page.raw();
    let keep_struct_parents = copier.src_ctx.exports_all_pages_uniquely;
    for (key, val) in raw_dict.entries() {
        let k = key.as_ref();
        let allowed = SAFE_EXTRA_KEYS.contains(&k) || (keep_struct_parents && k == STRUCT_PARENTS);
        if allowed {
            out.push(b' ');
            write_pdf_name(k, out);
            out.push(b' ');
            copier.serialize_maybe_ref(val, out, 0)?;
        }
    }
    Ok(())
}

fn append_page_resources(
    copier: &mut ObjectCopier<'_, '_, '_>,
    src_page: &Page<'_>,
    overlay_name: Option<&str>,
    overlay_xobj_id: Option<usize>,
    out: &mut Vec<u8>,
) -> Result<(), ()> {
    let raw_res_opt = src_page.raw().get_raw::<Object<'_>>(RESOURCES);
    if overlay_name.is_none()
        && let Some(raw_res) = raw_res_opt
    {
        // Fast path for unannotated pages: preserve indirect `/Resources` reference intact!
        out.extend_from_slice(b" /Resources ");
        copier.serialize_maybe_ref(raw_res, out, 0)?;
        return Ok(());
    }

    out.extend_from_slice(b" /Resources <<");
    if let Some(res_dict) = src_page.raw().get::<Dict<'_>>(RESOURCES) {
        let mut saw_xobject = false;
        for (key, val) in res_dict.entries() {
            if key.as_ref() == KEY_ENCRYPT {
                continue;
            }
            if key.as_ref() == XOBJECT {
                saw_xobject = true;
                if let (Some(ov_name), Some(ov_id)) = (overlay_name, overlay_xobj_id) {
                    let merged_xobj = res_dict.get::<Dict<'_>>(XOBJECT);
                    write_merged_xobject_dict(copier, merged_xobj.as_ref(), ov_name, ov_id, out)?;
                    continue;
                }
            }
            out.push(b' ');
            write_pdf_name(key.as_ref(), out);
            out.push(b' ');
            copier.serialize_maybe_ref(val, out, 0)?;
        }
        if !saw_xobject && let (Some(ov_name), Some(ov_id)) = (overlay_name, overlay_xobj_id) {
            write_merged_xobject_dict(copier, None, ov_name, ov_id, out)?;
        }
    } else {
        append_inherited_resources_dict(
            copier,
            src_page.resources(),
            overlay_name,
            overlay_xobj_id,
            out,
        )?;
    }
    out.extend_from_slice(b" >>");
    Ok(())
}

fn append_inherited_resources_dict(
    copier: &mut ObjectCopier<'_, '_, '_>,
    res: &Resources<'_>,
    overlay_name: Option<&str>,
    overlay_xobj_id: Option<usize>,
    out: &mut Vec<u8>,
) -> Result<(), ()> {
    let subdicts: &[(&[u8], &Dict<'_>)] = &[
        (EXT_G_STATE, &res.ext_g_states),
        (FONT, &res.fonts),
        (PROPERTIES, &res.properties),
        (COLORSPACE, &res.color_spaces),
        (PATTERN, &res.patterns),
        (SHADING, &res.shadings),
    ];
    for (name, dict) in subdicts {
        if !dict.is_empty() {
            out.push(b' ');
            write_pdf_name(name, out);
            out.push(b' ');
            copier.serialize_dict(dict, out, 0, true)?;
        }
    }
    if !res.x_objects.is_empty() || (overlay_name.is_some() && overlay_xobj_id.is_some()) {
        out.extend_from_slice(b" /XObject <<");
        for (xk, xval) in res.x_objects.entries() {
            out.push(b' ');
            write_pdf_name(xk.as_ref(), out);
            out.push(b' ');
            copier.serialize_maybe_ref(xval, out, 1)?;
        }
        if let (Some(ov_name), Some(ov_id)) = (overlay_name, overlay_xobj_id) {
            out.push(b' ');
            write_pdf_name(ov_name.as_bytes(), out);
            let _ = write!(page_bytes_writer(out), " {ov_id} 0 R");
        }
        out.extend_from_slice(b" >>");
    }
    Ok(())
}

fn write_merged_xobject_dict(
    copier: &mut ObjectCopier<'_, '_, '_>,
    existing: Option<&Dict<'_>>,
    ov_name: &str,
    ov_id: usize,
    out: &mut Vec<u8>,
) -> Result<(), ()> {
    out.extend_from_slice(b" /XObject <<");
    if let Some(dict) = existing {
        for (key, val) in dict.entries() {
            if key.as_ref() == KEY_ENCRYPT {
                continue;
            }
            out.push(b' ');
            write_pdf_name(key.as_ref(), out);
            out.push(b' ');
            copier.serialize_maybe_ref(val, out, 1)?;
        }
    }
    out.push(b' ');
    write_pdf_name(ov_name.as_bytes(), out);
    let _ = write!(page_bytes_writer(out), " {ov_id} 0 R >>");
    Ok(())
}

fn append_page_contents(
    copier: &mut ObjectCopier<'_, '_, '_>,
    src_page: &Page<'_>,
    orig_contents: &[ObjectIdentifier],
    overlay_invoke_id: Option<usize>,
    out: &mut Vec<u8>,
) -> Result<(), ()> {
    if let Some(invoke_id) = overlay_invoke_id {
        out.extend_from_slice(b" /Contents [");
        let mut first = true;
        if !orig_contents.is_empty() {
            let q_id = copier.writer.ensure_shared_q_stream();
            let _ = write!(page_bytes_writer(out), "{q_id} 0 R");
            first = false;
            for &src_id in orig_contents {
                let dest_id = copier.enqueue_ref(src_id)?;
                let _ = write!(page_bytes_writer(out), " {dest_id} 0 R");
            }
        }
        if !first {
            out.push(b' ');
        }
        let _ = write!(page_bytes_writer(out), "{invoke_id} 0 R]");
        return Ok(());
    }

    if let Some(raw_contents) = src_page.raw().get_raw::<Object<'_>>(CONTENTS) {
        out.extend_from_slice(b" /Contents ");
        copier.serialize_maybe_ref(raw_contents, out, 0)?;
    }
    Ok(())
}

pub(crate) struct ObjectCopier<'w, 'p, 'c> {
    pub(crate) writer: &'w mut PdfWriterState,
    pub(crate) pdf: &'p Pdf,
    pub(crate) src_ctx: &'c SourcePdfContext,
    pub(crate) pdf_idx: usize,
    pub(crate) queue: VecDeque<(ObjectIdentifier, usize)>,
    pub(crate) current_page_remap: Option<(ObjectIdentifier, usize)>,
    pub(crate) fail_on_unexported_page_ref: bool,
    pub(crate) strip_struct_parent: bool,
    pub(crate) rewrite_named_dests: bool,
}

impl<'w, 'p, 'c> ObjectCopier<'w, 'p, 'c> {
    pub(crate) fn new(
        writer: &'w mut PdfWriterState,
        pdf: &'p Pdf,
        src_ctx: &'c SourcePdfContext,
        pdf_idx: usize,
    ) -> Result<Self, ()> {
        writer.ensure_pdf_counter_capacity(pdf_idx);
        Ok(Self {
            writer,
            pdf,
            src_ctx,
            pdf_idx,
            queue: VecDeque::new(),
            current_page_remap: None,
            fail_on_unexported_page_ref: false,
            strip_struct_parent: false,
            rewrite_named_dests: false,
        })
    }

    pub(crate) fn enqueue_ref(&mut self, src_id: ObjectIdentifier) -> Result<usize, ()> {
        if let Some(&dest_id) = self.writer.lookup_remapped(self.pdf_idx, src_id) {
            return Ok(dest_id);
        }
        if !self
            .writer
            .increment_copied_count(self.pdf_idx, MAX_COPIED_OBJECTS_PER_PDF)
        {
            return Err(());
        }
        let dest_id = self.writer.alloc_obj_id();
        if self.writer.insert_remapped(self.pdf_idx, src_id, dest_id) {
            self.queue.push_back((src_id, dest_id));
        }
        Ok(dest_id)
    }

    pub(crate) fn drain_queue(&mut self) -> Result<(), ()> {
        while let Some((src_id, dest_id)) = self.queue.pop_front() {
            let Some(obj) = self.pdf.xref().get::<Object<'_>>(src_id) else {
                // PDF spec treats unresolved indirect references as `null`.
                self.writer.emit_raw_object(dest_id, b"null");
                continue;
            };
            if is_disallowed_indirect_object(&obj) {
                return Err(());
            }
            let mut body = Vec::with_capacity(128);
            self.serialize_object(obj, &mut body, 0)?;
            self.writer.emit_raw_object(dest_id, &body);
        }
        Ok(())
    }

    pub(crate) fn serialize_maybe_ref(
        &mut self,
        val: MaybeRef<Object<'_>>,
        out: &mut Vec<u8>,
        depth: usize,
    ) -> Result<(), ()> {
        match val {
            MaybeRef::Ref(r) => {
                let src_id: ObjectIdentifier = r.into();
                if let Some((cur_src, cur_dest)) = self.current_page_remap
                    && src_id == cur_src
                {
                    let _ = write!(page_bytes_writer(out), "{cur_dest} 0 R");
                    return Ok(());
                }
                if let Some(&mapped_opt) = self.src_ctx.page_id_map.get(&src_id) {
                    if let Some(dest_page_id) = mapped_opt {
                        let _ = write!(page_bytes_writer(out), "{dest_page_id} 0 R");
                        return Ok(());
                    }
                    if self.fail_on_unexported_page_ref {
                        return Err(());
                    }
                    out.extend_from_slice(b"null");
                    return Ok(());
                }
                if src_id == self.pdf.xref().root_id() {
                    out.extend_from_slice(b"1 0 R");
                    return Ok(());
                }
                if self.src_ctx.dropped_annot_ids.contains(&src_id) {
                    out.extend_from_slice(b"null");
                    return Ok(());
                }
                let dest_id = self.enqueue_ref(src_id)?;
                let _ = write!(page_bytes_writer(out), "{dest_id} 0 R");
                Ok(())
            }
            MaybeRef::NotRef(obj) => self.serialize_object(obj, out, depth),
        }
    }

    pub(crate) fn serialize_object(
        &mut self,
        obj: Object<'_>,
        out: &mut Vec<u8>,
        depth: usize,
    ) -> Result<(), ()> {
        if depth >= MAX_NESTING_DEPTH {
            return Err(());
        }
        match obj {
            Object::Null(_) => out.extend_from_slice(b"null"),
            Object::Boolean(b) => {
                out.extend_from_slice(if b { b"true" } else { b"false" });
            }
            Object::Number(n) => format_pdf_number(n, out),
            Object::Name(name) => write_pdf_name(name.as_ref(), out),
            Object::String(s) => write_pdf_literal_string(s.as_bytes(), out),
            Object::Array(arr) => self.serialize_array(&arr, out, depth + 1)?,
            Object::Dict(dict) => self.serialize_dict(&dict, out, depth + 1, true)?,
            Object::Stream(stream) => self.serialize_stream(&stream, out, depth + 1)?,
        }
        Ok(())
    }

    pub(crate) fn serialize_array(
        &mut self,
        arr: &Array<'_>,
        out: &mut Vec<u8>,
        depth: usize,
    ) -> Result<(), ()> {
        if depth >= MAX_NESTING_DEPTH {
            return Err(());
        }
        out.push(b'[');
        for (idx, item) in arr.raw_iter().enumerate() {
            if idx > 0 {
                out.push(b' ');
            }
            self.serialize_maybe_ref(item, out, depth)?;
        }
        out.push(b']');
        Ok(())
    }

    pub(crate) fn serialize_dict(
        &mut self,
        dict: &Dict<'_>,
        out: &mut Vec<u8>,
        depth: usize,
        strip_encrypt: bool,
    ) -> Result<(), ()> {
        if depth >= MAX_NESTING_DEPTH {
            return Err(());
        }
        let is_goto_action = self.rewrite_named_dests
            && self.src_ctx.dest_prefix.is_some()
            && dict
                .get::<Name<'_>>(S)
                .is_some_and(|s| s.as_ref() == b"GoTo");
        out.extend_from_slice(b"<<");
        for (key, val) in dict.entries() {
            let k = key.as_ref();
            if strip_encrypt && k == KEY_ENCRYPT {
                continue;
            }
            if self.strip_struct_parent && k == STRUCT_PARENT {
                continue;
            }
            out.push(b' ');
            write_pdf_name(k, out);
            out.push(b' ');
            if self.rewrite_named_dests
                && (k == DEST || (is_goto_action && k == D))
                && let Some(prefix) = self.src_ctx.dest_prefix.as_deref()
                && self.try_write_prefixed_dest_value(&val, prefix, out)
            {
                continue;
            }
            self.serialize_maybe_ref(val, out, depth)?;
        }
        out.extend_from_slice(b" >>");
        Ok(())
    }

    fn try_write_prefixed_dest_value(
        &self,
        val: &MaybeRef<Object<'_>>,
        prefix: &[u8],
        out: &mut Vec<u8>,
    ) -> bool {
        match val {
            MaybeRef::NotRef(Object::Name(n)) => {
                write_prefixed_name(prefix, n.as_ref(), out);
                true
            }
            MaybeRef::NotRef(Object::String(s)) => {
                write_prefixed_string(prefix, s.as_bytes(), out);
                true
            }
            MaybeRef::Ref(r) => match self.pdf.xref().get::<Object<'_>>((*r).into()) {
                Some(Object::Name(n)) => {
                    write_prefixed_name(prefix, n.as_ref(), out);
                    true
                }
                Some(Object::String(s)) => {
                    write_prefixed_string(prefix, s.as_bytes(), out);
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    fn serialize_stream(
        &mut self,
        stream: &Stream<'_>,
        out: &mut Vec<u8>,
        depth: usize,
    ) -> Result<(), ()> {
        if depth >= MAX_NESTING_DEPTH {
            return Err(());
        }
        // External file streams (`/F (file)` / `/F <<...>>`) cannot be copied inline reliably.
        if matches!(
            stream.dict().get::<Object<'_>>(F),
            Some(Object::String(_) | Object::Dict(_))
        ) {
            return Err(());
        }
        let raw = stream.raw_data();
        let crypt_cleanup = inspect_stream_crypt_filter(stream.dict());

        out.extend_from_slice(b"<<");
        for (key, val) in stream.dict().entries() {
            let k = key.as_ref();
            if k == LENGTH || k == KEY_ENCRYPT {
                continue;
            }
            if crypt_cleanup.strip_all_filters && (k == FILTER || k == DECODE_PARMS || k == DP) {
                continue;
            }
            if let Some(ref keep_mask) = crypt_cleanup.keep_mask
                && k == FILTER
                && let Some(arr) = stream.dict().get::<Array<'_>>(FILTER)
            {
                if keep_mask.iter().any(|&keep| keep) {
                    out.extend_from_slice(b" /Filter ");
                    self.serialize_filtered_array(&arr, keep_mask, out, depth)?;
                }
                continue;
            }
            if let Some(ref keep_mask) = crypt_cleanup.keep_mask
                && (k == DECODE_PARMS || k == DP)
                && let Some(arr) = stream
                    .dict()
                    .get::<Array<'_>>(DECODE_PARMS)
                    .or_else(|| stream.dict().get::<Array<'_>>(DP))
            {
                if keep_mask.iter().any(|&keep| keep) {
                    out.push(b' ');
                    write_pdf_name(k, out);
                    out.push(b' ');
                    self.serialize_filtered_array(&arr, keep_mask, out, depth)?;
                }
                continue;
            }
            out.push(b' ');
            write_pdf_name(k, out);
            out.push(b' ');
            self.serialize_maybe_ref(val, out, depth)?;
        }
        let _ = write!(
            page_bytes_writer(out),
            " /Length {} >>\nstream\n",
            raw.len()
        );
        out.extend_from_slice(raw.as_ref());
        out.extend_from_slice(b"\nendstream");
        Ok(())
    }

    fn serialize_filtered_array(
        &mut self,
        arr: &Array<'_>,
        keep_mask: &[bool],
        out: &mut Vec<u8>,
        depth: usize,
    ) -> Result<(), ()> {
        out.push(b'[');
        let mut emitted = 0usize;
        for (idx, item) in arr.raw_iter().enumerate() {
            if !keep_mask.get(idx).copied().unwrap_or(true) {
                continue;
            }
            if emitted > 0 {
                out.push(b' ');
            }
            self.serialize_maybe_ref(item, out, depth)?;
            emitted += 1;
        }
        out.push(b']');
        Ok(())
    }
}

fn write_prefixed_name(prefix: &[u8], raw: &[u8], out: &mut Vec<u8>) {
    let mut buf = Vec::with_capacity(prefix.len() + raw.len());
    buf.extend_from_slice(prefix);
    buf.extend_from_slice(raw);
    write_pdf_name(&buf, out);
}

fn write_prefixed_string(prefix: &[u8], raw: &[u8], out: &mut Vec<u8>) {
    let mut buf = Vec::with_capacity(prefix.len() + raw.len());
    buf.extend_from_slice(prefix);
    buf.extend_from_slice(raw);
    write_pdf_literal_string(&buf, out);
}

struct CryptFilterCleanup {
    strip_all_filters: bool,
    keep_mask: Option<Vec<bool>>,
}

fn inspect_stream_crypt_filter(dict: &Dict<'_>) -> CryptFilterCleanup {
    let Some(filter_obj) = dict.get::<Object<'_>>(FILTER) else {
        return CryptFilterCleanup {
            strip_all_filters: false,
            keep_mask: None,
        };
    };
    match filter_obj {
        Object::Name(n) if n.as_ref() == KEY_CRYPT => CryptFilterCleanup {
            strip_all_filters: true,
            keep_mask: None,
        },
        Object::Array(arr) => {
            let mut keep_mask = Vec::new();
            let mut has_crypt = false;
            for item in arr.iter::<Object<'_>>() {
                let is_crypt = matches!(item, Object::Name(n) if n.as_ref() == KEY_CRYPT);
                if is_crypt {
                    has_crypt = true;
                }
                keep_mask.push(!is_crypt);
            }
            if !has_crypt {
                return CryptFilterCleanup {
                    strip_all_filters: false,
                    keep_mask: None,
                };
            }
            if !keep_mask.iter().any(|&keep| keep) {
                return CryptFilterCleanup {
                    strip_all_filters: true,
                    keep_mask: None,
                };
            }
            CryptFilterCleanup {
                strip_all_filters: false,
                keep_mask: Some(keep_mask),
            }
        }
        _ => CryptFilterCleanup {
            strip_all_filters: false,
            keep_mask: None,
        },
    }
}

fn is_disallowed_indirect_object(obj: &Object<'_>) -> bool {
    let Object::Dict(dict) = obj else {
        return false;
    };
    let Some(ty) = dict.get::<Name<'_>>(TYPE) else {
        return false;
    };
    matches!(ty.as_ref(), b"Pages" | b"Catalog" | b"Page")
}

fn write_rect(out: &mut Vec<u8>, r: Rect) {
    out.push(b'[');
    format_pdf_f64(r.x0, out);
    out.push(b' ');
    format_pdf_f64(r.y0, out);
    out.push(b' ');
    format_pdf_f64(r.x1, out);
    out.push(b' ');
    format_pdf_f64(r.y1, out);
    out.push(b']');
}

pub(crate) fn format_pdf_number(n: Number, out: &mut Vec<u8>) {
    let display = n.to_string();
    if !display.contains(['.', 'e', 'E']) && display.parse::<i64>().is_ok() {
        out.extend_from_slice(display.as_bytes());
        return;
    }
    format_pdf_f64(n.as_f64(), out);
}

pub(crate) fn format_pdf_f64(v: f64, out: &mut Vec<u8>) {
    if !v.is_finite() || v == 0.0 {
        out.push(b'0');
        return;
    }
    let s = if v.abs() < 1.0 {
        format!("{v:.9}")
    } else {
        format!("{v:.6}")
    };
    let trimmed = s.trim_end_matches('0').trim_end_matches('.');
    if trimmed.is_empty() || trimmed == "-0" {
        out.push(b'0');
    } else {
        out.extend_from_slice(trimmed.as_bytes());
    }
}

pub(crate) fn write_pdf_name(bytes: &[u8], out: &mut Vec<u8>) {
    out.push(b'/');
    for &b in bytes {
        let regular = (0x21..=0x7E).contains(&b)
            && !matches!(
                b,
                b'#' | b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%'
            );
        if regular {
            out.push(b);
        } else {
            let _ = write!(page_bytes_writer(out), "#{b:02X}");
        }
    }
}

pub(crate) fn write_pdf_literal_string(bytes: &[u8], out: &mut Vec<u8>) {
    out.push(b'(');
    for &b in bytes {
        match b {
            b'(' => out.extend_from_slice(b"\\("),
            b')' => out.extend_from_slice(b"\\)"),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\n' => out.extend_from_slice(b"\\n"),
            0x20..=0x7E => out.push(b),
            other => {
                let _ = write!(page_bytes_writer(out), "\\{other:03o}");
            }
        }
    }
    out.push(b')');
}

/// Scans the PDF header within the first 2 KiB for `%PDF-X.Y` without parsing xref tables.
pub(crate) fn detect_pdf_header_version(bytes: &[u8]) -> (u8, u8) {
    let head = &bytes[..bytes.len().min(2048)];
    for idx in 0..head.len().saturating_sub(7) {
        if &head[idx..idx + 5] == b"%PDF-" {
            let maj = head[idx + 5];
            let dot = head[idx + 6];
            let min = head[idx + 7];
            if dot == b'.' && maj.is_ascii_digit() && min.is_ascii_digit() {
                return (maj - b'0', min - b'0');
            }
        }
    }
    (1, 4)
}
