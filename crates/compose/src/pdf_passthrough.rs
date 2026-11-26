//! Vector PDF pass-through composer built on `hayro-syntax` object copying.
//!
//! Copies only the indirect objects reachable from selected pages without decoding or
//! rasterizing existing content, image, or font streams. Unannotated pages pass through
//! directly; annotated pages wrap original `/Contents` in `q`/`Q` and place a PaperCraft
//! Form XObject overlay mapped cleanly via the inverse of `Page::initial_transform(true)`.

use std::collections::{HashSet, VecDeque};
use std::fmt::Write as _;

use hayro::hayro_syntax::Pdf;
use hayro::hayro_syntax::object::dict::keys::{
    COLORSPACE, CONTENTS, DECODE_PARMS, DP, EXT_G_STATE, F, FILTER, FONT, LENGTH, PATTERN,
    PROPERTIES, RESOURCES, SHADING, TYPE, XOBJECT,
};
use hayro::hayro_syntax::object::{
    Array, Dict, MaybeRef, Name, Number, Object, ObjectIdentifier, Rect, Stream,
};
use hayro::hayro_syntax::page::{Page, Resources, Rotation};
use papercraft_doc::{Document, NotebookPage, PdfPageBackdrop};

use crate::export::{
    PdfImageEntry, PdfWriterState, append_pdf_overlay_body, format_pdf_overlay_resources,
    prepare_pdf_page_images,
};

const MAX_COPIED_OBJECTS_PER_PDF: usize = 250_000;
const MAX_NESTING_DEPTH: usize = 64;

const KEY_BLEED_BOX: &[u8] = b"BleedBox";
const KEY_TRIM_BOX: &[u8] = b"TrimBox";
const KEY_ART_BOX: &[u8] = b"ArtBox";
const KEY_GROUP: &[u8] = b"Group";
const KEY_USER_UNIT: &[u8] = b"UserUnit";
const KEY_METADATA: &[u8] = b"Metadata";
const KEY_ENCRYPT: &[u8] = b"Encrypt";
const KEY_CRYPT: &[u8] = b"Crypt";

/// Attempts to emit a PDF-backed `page` using lossless vector pass-through.
///
/// Returns `Ok(())` when the page is completely emitted into `writer`. Returns `Err(())`
/// (with `writer` atomically rolled back) on any unexpected PDF structure so the caller can
/// transparently fall back to single-page streaming rasterization.
pub(crate) fn try_emit_pdf_backdrop_page(
    writer: &mut PdfWriterState,
    pdf: &Pdf,
    page: &NotebookPage,
    backdrop: PdfPageBackdrop,
    _doc: &Document,
) -> Result<(), ()> {
    let Some(src_page) = pdf.pages().get(backdrop.page_index) else {
        return Err(());
    };

    let overlay_matrix = if page.has_annotations() {
        Some(compute_overlay_pc_to_pdf_matrix(src_page, page).ok_or(())?)
    } else {
        None
    };

    let checkpoint = writer.checkpoint();
    let res = emit_pdf_backdrop_page_inner(writer, pdf, src_page, page, backdrop, overlay_matrix);
    if res.is_err() {
        writer.rollback(checkpoint);
    }
    res
}

fn emit_pdf_backdrop_page_inner(
    writer: &mut PdfWriterState,
    pdf: &Pdf,
    src_page: &Page<'_>,
    page: &NotebookPage,
    backdrop: PdfPageBackdrop,
    overlay_matrix: Option<[f64; 6]>,
) -> Result<(), ()> {
    let mut copier = ObjectCopier::new(writer, pdf, backdrop.pdf_index)?;
    let orig_contents = resolve_original_contents(pdf, src_page)?;

    let (overlay_xobj_id, overlay_invoke_id, overlay_name) = if let Some(matrix) = overlay_matrix {
        let overlay_name = pick_free_xobject_name(src_page);

        // Allocate image object IDs from `copier.writer` without interleaving writes.
        let mut images = prepare_pdf_page_images(page, &mut 0);
        assign_writer_image_ids(copier.writer, &mut images);

        let (overlay_stream, states) = append_pdf_overlay_body(page, &images);
        let overlay_res = format_pdf_overlay_resources(&states, None, &images);

        let xobj_id = copier.writer.alloc_obj_id();
        let w = page.width.clamp(64, 8192);
        let h = page.height.clamp(64, 8192);
        let form_extra = format!(
            "/Type /XObject /Subtype /Form /FormType 1 /BBox [0 0 {w} {h}] /Resources {overlay_res} "
        );
        copier
            .writer
            .emit_stream_object(xobj_id, &form_extra, &overlay_stream);
        copier.writer.emit_image_entries(&images);
        drop(images);

        let invoke_id = copier.writer.alloc_obj_id();
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
        copier
            .writer
            .emit_stream_object(invoke_id, "", invoke_body.as_bytes());
        (Some(xobj_id), Some(invoke_id), Some(overlay_name))
    } else {
        (None, None, None)
    };

    let page_obj_id = copier.writer.alloc_obj_id();
    let media_box = src_page.media_box();
    let crop_box = src_page.intersected_crop_box();
    let orig_rot_deg = match src_page.rotation() {
        Rotation::None => 0,
        Rotation::Horizontal => 90,
        Rotation::Flipped => 180,
        Rotation::FlippedHorizontal => 270,
    };
    let final_rot_deg = (orig_rot_deg + i32::from(page.backdrop_rotation % 4) * 90) % 360;

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

    append_safe_page_extras(&mut copier, src_page, &mut page_bytes)?;
    append_page_resources(
        &mut copier,
        src_page,
        overlay_name.as_deref(),
        overlay_xobj_id,
        &mut page_bytes,
    )?;
    append_page_contents(
        &mut copier,
        src_page,
        &orig_contents,
        overlay_invoke_id,
        &mut page_bytes,
    )?;
    page_bytes.extend_from_slice(b" >>");

    copier.writer.emit_raw_object(page_obj_id, &page_bytes);
    copier.writer.page_obj_ids.push(page_obj_id);
    copier.drain_queue()?;
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
struct ByteFmtWriter<'a>(&'a mut Vec<u8>);

impl std::fmt::Write for ByteFmtWriter<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}

fn page_bytes_writer(buf: &mut Vec<u8>) -> ByteFmtWriter<'_> {
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

fn append_safe_page_extras(
    copier: &mut ObjectCopier<'_, '_>,
    src_page: &Page<'_>,
    out: &mut Vec<u8>,
) -> Result<(), ()> {
    const SAFE_EXTRA_KEYS: &[&[u8]] = &[
        KEY_BLEED_BOX,
        KEY_TRIM_BOX,
        KEY_ART_BOX,
        KEY_GROUP,
        KEY_USER_UNIT,
        KEY_METADATA,
    ];
    let raw_dict = src_page.raw();
    for (key, val) in raw_dict.entries() {
        if SAFE_EXTRA_KEYS
            .iter()
            .any(|allowed| *allowed == key.as_ref())
        {
            out.push(b' ');
            write_pdf_name(key.as_ref(), out);
            out.push(b' ');
            copier.serialize_maybe_ref(val, out, 0)?;
        }
    }
    Ok(())
}

fn append_page_resources(
    copier: &mut ObjectCopier<'_, '_>,
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
    copier: &mut ObjectCopier<'_, '_>,
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
    copier: &mut ObjectCopier<'_, '_>,
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
    copier: &mut ObjectCopier<'_, '_>,
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

struct ObjectCopier<'w, 'p> {
    writer: &'w mut PdfWriterState,
    pdf: &'p Pdf,
    pdf_idx: usize,
    queue: VecDeque<(ObjectIdentifier, usize)>,
    queued_or_emitted: HashSet<ObjectIdentifier>,
}

impl<'w, 'p> ObjectCopier<'w, 'p> {
    fn new(writer: &'w mut PdfWriterState, pdf: &'p Pdf, pdf_idx: usize) -> Result<Self, ()> {
        writer.ensure_pdf_counter_capacity(pdf_idx);
        Ok(Self {
            writer,
            pdf,
            pdf_idx,
            queue: VecDeque::new(),
            queued_or_emitted: HashSet::new(),
        })
    }

    fn enqueue_ref(&mut self, src_id: ObjectIdentifier) -> Result<usize, ()> {
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
        self.writer.insert_remapped(self.pdf_idx, src_id, dest_id);
        if self.queued_or_emitted.insert(src_id) {
            self.queue.push_back((src_id, dest_id));
        }
        Ok(dest_id)
    }

    fn drain_queue(&mut self) -> Result<(), ()> {
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

    fn serialize_maybe_ref(
        &mut self,
        val: MaybeRef<Object<'_>>,
        out: &mut Vec<u8>,
        depth: usize,
    ) -> Result<(), ()> {
        match val {
            MaybeRef::Ref(r) => {
                let dest_id = self.enqueue_ref(r.into())?;
                let _ = write!(page_bytes_writer(out), "{dest_id} 0 R");
                Ok(())
            }
            MaybeRef::NotRef(obj) => self.serialize_object(obj, out, depth),
        }
    }

    fn serialize_object(
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

    fn serialize_array(
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

    fn serialize_dict(
        &mut self,
        dict: &Dict<'_>,
        out: &mut Vec<u8>,
        depth: usize,
        strip_encrypt: bool,
    ) -> Result<(), ()> {
        if depth >= MAX_NESTING_DEPTH {
            return Err(());
        }
        out.extend_from_slice(b"<<");
        for (key, val) in dict.entries() {
            if strip_encrypt && key.as_ref() == KEY_ENCRYPT {
                continue;
            }
            out.push(b' ');
            write_pdf_name(key.as_ref(), out);
            out.push(b' ');
            self.serialize_maybe_ref(val, out, depth)?;
        }
        out.extend_from_slice(b" >>");
        Ok(())
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
    matches!(ty.as_ref(), b"Pages" | b"Catalog")
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

fn format_pdf_number(n: Number, out: &mut Vec<u8>) {
    let display = n.to_string();
    if !display.contains(['.', 'e', 'E']) && display.parse::<i64>().is_ok() {
        out.extend_from_slice(display.as_bytes());
        return;
    }
    format_pdf_f64(n.as_f64(), out);
}

fn format_pdf_f64(v: f64, out: &mut Vec<u8>) {
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

fn write_pdf_name(bytes: &[u8], out: &mut Vec<u8>) {
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

fn write_pdf_literal_string(bytes: &[u8], out: &mut Vec<u8>) {
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
