//! Pure-Rust PDF inspection and CPU rasterization backed by `hayro`.

use std::cell::RefCell;
use std::fmt;
use std::sync::{Arc, Weak};

use hayro::hayro_interpret::InterpreterSettings;
use hayro::hayro_interpret::util::TransformExt;
use hayro::hayro_syntax::page::Page;
use hayro::hayro_syntax::{LoadPdfError, Pdf};
use hayro::kurbo::Affine;
use hayro::vello_cpu::color::palette::css::WHITE;
use hayro::vello_cpu::{Pixmap, RasterizerSettings, RenderContext, Resources, TargetInit};
use hayro::{RenderCache, RenderSettings, render_into};
use papercraft_doc::looks_like_pdf_bytes;

const MAX_RENDER_PIXEL_DIM: u32 = 4096;
const MAX_CACHED_OPEN_PDFS: usize = 2;

struct CachedOpenPdf {
    bytes_weak: Weak<Vec<u8>>,
    bytes_len: usize,
    pdf: Arc<Pdf>,
}

impl CachedOpenPdf {
    fn matches(&self, bytes: &Arc<Vec<u8>>) -> bool {
        self.bytes_len == bytes.len()
            && self
                .bytes_weak
                .upgrade()
                .is_some_and(|cached| Arc::ptr_eq(&cached, bytes))
    }
}

struct VelloRenderBuffers {
    ctx: RenderContext,
    pixmap: Pixmap,
    resources: Resources,
}

impl VelloRenderBuffers {
    fn new() -> Self {
        Self {
            ctx: RenderContext::new(0, 0),
            pixmap: Pixmap::new(0, 0),
            resources: Resources::default(),
        }
    }
}

struct PooledVelloBuffers(Option<VelloRenderBuffers>);

impl PooledVelloBuffers {
    fn acquire() -> Self {
        let existing = THREAD_VELLO_BUFFERS
            .with(|cell| cell.try_borrow_mut().ok().and_then(|mut slot| slot.take()));
        Self(Some(existing.unwrap_or_else(VelloRenderBuffers::new)))
    }

    fn get_or_init_mut(&mut self) -> &mut VelloRenderBuffers {
        self.0.get_or_insert_with(VelloRenderBuffers::new)
    }
}

impl Drop for PooledVelloBuffers {
    fn drop(&mut self) {
        if let Some(bufs) = self.0.take() {
            THREAD_VELLO_BUFFERS.with(|cell| {
                if let Ok(mut slot) = cell.try_borrow_mut() {
                    *slot = Some(bufs);
                }
            });
        }
    }
}

thread_local! {
    static THREAD_PDF_CACHE: RefCell<Vec<CachedOpenPdf>> = const { RefCell::new(Vec::new()) };
    static THREAD_VELLO_BUFFERS: RefCell<Option<VelloRenderBuffers>> = const { RefCell::new(None) };
}

/// Clears this thread's cached parsed PDFs and reusable rasterizer scratch buffers.
pub fn clear_pdf_cache() {
    THREAD_PDF_CACHE.with(|cell| {
        if let Ok(mut cache) = cell.try_borrow_mut() {
            cache.clear();
        }
    });
    THREAD_VELLO_BUFFERS.with(|cell| {
        if let Ok(mut slot) = cell.try_borrow_mut() {
            *slot = None;
        }
    });
}

/// Errors raised when inspecting or rendering an imported PDF file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PdfError {
    Empty,
    TooLarge {
        limit_bytes: usize,
    },
    Encrypted,
    Invalid(String),
    NoPages,
    PageLimitExceeded {
        page_count: usize,
        max_pages: usize,
    },
    PageOutOfBounds {
        page_index: usize,
        page_count: usize,
    },
    EncodeFailed(String),
}

impl fmt::Display for PdfError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => write!(f, "PDF file is empty."),
            Self::TooLarge { limit_bytes } => {
                write!(
                    f,
                    "PDF exceeds the {} MiB maximum file size.",
                    limit_bytes / (1024 * 1024)
                )
            }
            Self::Encrypted => {
                write!(f, "Password-protected PDF files are not supported yet.")
            }
            Self::Invalid(why) => write!(f, "Could not read PDF file: {why}"),
            Self::NoPages => write!(f, "PDF file does not contain any pages."),
            Self::PageLimitExceeded {
                page_count,
                max_pages,
            } => write!(
                f,
                "PDF has {page_count} pages, exceeding the limit of {max_pages} pages."
            ),
            Self::PageOutOfBounds {
                page_index,
                page_count,
            } => write!(
                f,
                "PDF page {} does not exist (document has {page_count} pages).",
                page_index + 1
            ),
            Self::EncodeFailed(why) => write!(f, "Failed to encode PDF page backdrop: {why}"),
        }
    }
}

impl std::error::Error for PdfError {}

/// Page dimensions extracted from an imported PDF page, scaled to PaperCraft page units.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PdfPageMetrics {
    pub page_width: u32,
    pub page_height: u32,
}

/// Summary returned by [`inspect_pdf`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PdfDocumentSummary {
    pub title: Option<String>,
    pub pages: Vec<PdfPageMetrics>,
}

/// Rasterized opaque RGBA8 image for a single PDF page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderedPdfPage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Converts PDF user-space points (72 pt/in) into PaperCraft page dimensions.
///
/// Uses a `2x` scale so standard Letter/A4 sheets map near `1200x1600` page points,
/// matching default pen sizes and `0.5x` PDF re-export scaling.
#[must_use]
pub fn pdf_pt_to_page_size(w_pt: f32, h_pt: f32) -> (u32, u32) {
    let w = if w_pt.is_finite() && w_pt > 1.0 {
        f64::from(w_pt)
    } else {
        595.0
    };
    let h = if h_pt.is_finite() && h_pt > 1.0 {
        f64::from(h_pt)
    } else {
        842.0
    };

    let mut sx = w * 2.0;
    let mut sy = h * 2.0;
    let max_side = sx.max(sy);
    if max_side > 4096.0 {
        let k = 4096.0 / max_side;
        sx *= k;
        sy *= k;
    }
    let min_side = sx.min(sy);
    if min_side < 64.0 && min_side > 0.0 {
        let k = 64.0 / min_side;
        sx = (sx * k).min(4096.0);
        sy = (sy * k).min(4096.0);
    }
    (
        (sx.round() as u32).clamp(64, 8192),
        (sy.round() as u32).clamp(64, 8192),
    )
}

fn open_pdf(bytes: Arc<Vec<u8>>) -> Result<Pdf, PdfError> {
    if bytes.is_empty() {
        return Err(PdfError::Empty);
    }
    if !looks_like_pdf_bytes(&bytes) {
        return Err(PdfError::Invalid("missing %PDF- header".to_string()));
    }
    Pdf::new(bytes).map_err(|err| match err {
        LoadPdfError::Decryption(_) => PdfError::Encrypted,
        LoadPdfError::Invalid => PdfError::Invalid("corrupted or unsupported PDF structure".into()),
    })
}

fn get_or_open_pdf(bytes: Arc<Vec<u8>>) -> Result<Arc<Pdf>, PdfError> {
    let bytes_len = bytes.len();
    let cached_hit = THREAD_PDF_CACHE.with(|cell| {
        let Ok(mut cache) = cell.try_borrow_mut() else {
            return None;
        };
        let idx = cache.iter().position(|entry| entry.matches(&bytes))?;
        let entry = cache.remove(idx);
        let pdf = Arc::clone(&entry.pdf);
        cache.insert(0, entry);
        Some(pdf)
    });
    if let Some(pdf) = cached_hit {
        return Ok(pdf);
    }

    let parsed = Arc::new(open_pdf(Arc::clone(&bytes))?);
    let bytes_weak = Arc::downgrade(&bytes);
    THREAD_PDF_CACHE.with(|cell| {
        if let Ok(mut cache) = cell.try_borrow_mut() {
            if let Some(idx) = cache.iter().position(|entry| entry.matches(&bytes)) {
                let entry = cache.remove(idx);
                cache.insert(0, entry);
                return;
            }
            while cache.len() >= MAX_CACHED_OPEN_PDFS {
                cache.pop();
            }
            cache.insert(
                0,
                CachedOpenPdf {
                    bytes_weak,
                    bytes_len,
                    pdf: Arc::clone(&parsed),
                },
            );
        }
    });
    Ok(parsed)
}

fn decode_pdf_title(raw: &[u8]) -> Option<String> {
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
        .take(96)
        .collect();
    let trimmed = clean.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Validates PDF bytes and reads title metadata + per-page scaled canvas dimensions.
///
/// Pass `usize::MAX` for `max_pages` when no page count cap should be applied.
pub fn inspect_pdf(bytes: Arc<Vec<u8>>, max_pages: usize) -> Result<PdfDocumentSummary, PdfError> {
    let pdf = get_or_open_pdf(bytes)?;
    let raw_pages = pdf.pages();
    let page_count = raw_pages.len();
    if page_count == 0 {
        return Err(PdfError::NoPages);
    }
    if page_count > max_pages {
        return Err(PdfError::PageLimitExceeded {
            page_count,
            max_pages,
        });
    }

    let pages = raw_pages
        .iter()
        .map(|page| {
            let (w_pt, h_pt) = page.render_dimensions();
            let (page_width, page_height) = pdf_pt_to_page_size(w_pt, h_pt);
            PdfPageMetrics {
                page_width,
                page_height,
            }
        })
        .collect();
    let title = pdf.metadata().title.as_deref().and_then(decode_pdf_title);

    Ok(PdfDocumentSummary { title, pages })
}

fn fit_render_pixels(src_w: f32, src_h: f32, max_w: u32, max_h: u32) -> (u16, u16) {
    let w = if src_w.is_finite() && src_w > 1.0 {
        f64::from(src_w)
    } else {
        595.0
    };
    let h = if src_h.is_finite() && src_h > 1.0 {
        f64::from(src_h)
    } else {
        842.0
    };
    let cap_w = f64::from(max_w.clamp(1, MAX_RENDER_PIXEL_DIM));
    let cap_h = f64::from(max_h.clamp(1, MAX_RENDER_PIXEL_DIM));
    let scale = (cap_w / w).min(cap_h / h).max(0.01);
    let px_w = (w * scale).round().clamp(1.0, cap_w) as u16;
    let px_h = (h * scale).round().clamp(1.0, cap_h) as u16;
    (px_w, px_h)
}

struct PdfRenderState<'a> {
    cache: RenderCache<'a>,
    interpreter_settings: InterpreterSettings,
    render_settings: RenderSettings,
    vello: PooledVelloBuffers,
}

impl<'a> PdfRenderState<'a> {
    fn new() -> Self {
        Self {
            cache: RenderCache::new(),
            interpreter_settings: InterpreterSettings::default(),
            render_settings: RenderSettings::default(),
            vello: PooledVelloBuffers::acquire(),
        }
    }

    fn render_page(&mut self, page: &'a Page<'a>, pixel_w: u16, pixel_h: u16) -> RenderedPdfPage {
        let (src_w, src_h) = page.render_dimensions();
        let safe_w = if src_w.is_finite() && src_w > 1.0 {
            f64::from(src_w)
        } else {
            595.0
        };
        let safe_h = if src_h.is_finite() && src_h > 1.0 {
            f64::from(src_h)
        } else {
            842.0
        };
        let vello = self.vello.get_or_init_mut();
        vello.ctx.reset_and_resize(pixel_w, pixel_h);
        vello.pixmap.resize(pixel_w, pixel_h);

        let transform =
            Affine::scale_non_uniform(f64::from(pixel_w) / safe_w, f64::from(pixel_h) / safe_h)
                * page.initial_transform(true).to_kurbo();
        render_into(
            page,
            &self.cache,
            &self.interpreter_settings,
            &self.render_settings,
            &mut vello.ctx,
            transform,
        );
        vello.ctx.flush();
        vello.ctx.render_with(
            &mut vello.pixmap,
            &mut vello.resources,
            RasterizerSettings {
                target_init: TargetInit::Clear(WHITE),
                ..Default::default()
            },
        );

        let mut rgba = vello.pixmap.data_as_u8_slice().to_vec();
        // Surface is cleared with opaque white, so clamp alpha to 255 for straight RGBA consumers.
        for px in rgba.as_chunks_mut::<4>().0 {
            px[3] = 255;
        }
        RenderedPdfPage {
            width: u32::from(pixel_w),
            height: u32::from(pixel_h),
            rgba,
        }
    }
}

/// Reusable per-PDF renderer holding parsed pages and shared font/glyph caches across renders.
pub struct PdfPageRenderer<'a> {
    pages: &'a [Page<'a>],
    state: PdfRenderState<'a>,
}

impl<'a> PdfPageRenderer<'a> {
    /// Number of pages in the opened PDF.
    #[must_use]
    pub fn page_count(&self) -> usize {
        self.pages.len()
    }

    /// Renders `page_index` preserving its aspect ratio within `max_w x max_h`.
    pub fn render_page_rgba(
        &mut self,
        page_index: usize,
        max_w: u32,
        max_h: u32,
    ) -> Result<RenderedPdfPage, PdfError> {
        let Some(page) = self.pages.get(page_index) else {
            return Err(PdfError::PageOutOfBounds {
                page_index,
                page_count: self.pages.len(),
            });
        };
        let (src_w, src_h) = page.render_dimensions();
        let (px_w, px_h) = fit_render_pixels(src_w, src_h, max_w, max_h);
        Ok(self.state.render_page(page, px_w, px_h))
    }
}

/// Opens a PDF once and passes a [`PdfPageRenderer`] borrowing it to `f`, avoiding
/// repeated document parsing and sharing font/glyph caches across pages.
pub fn with_pdf_renderer<R, F>(bytes: Arc<Vec<u8>>, f: F) -> Result<R, PdfError>
where
    F: FnOnce(&mut PdfPageRenderer<'_>) -> R,
{
    let pdf = get_or_open_pdf(bytes)?;
    let pages = pdf.pages();
    let mut renderer = PdfPageRenderer {
        pages,
        state: PdfRenderState::new(),
    };
    Ok(f(&mut renderer))
}

/// Renders one PDF page preserving its aspect ratio within `max_w x max_h`.
pub fn render_pdf_page_rgba(
    bytes: Arc<Vec<u8>>,
    page_index: usize,
    max_w: u32,
    max_h: u32,
) -> Result<RenderedPdfPage, PdfError> {
    with_pdf_renderer(bytes, |renderer| {
        renderer.render_page_rgba(page_index, max_w, max_h)
    })?
}

/// Renders one PDF page to exact `(width, height)` pixel dimensions (clamped to `1..=4096`).
pub fn render_pdf_page_exact_rgba(
    bytes: Arc<Vec<u8>>,
    page_index: usize,
    width: u32,
    height: u32,
) -> Result<RenderedPdfPage, PdfError> {
    let results = render_pdf_batch_exact_rgba(bytes, &[(page_index, width, height)])?;
    results
        .into_iter()
        .next()
        .flatten()
        .ok_or(PdfError::Invalid("failed to render PDF page".to_string()))
}

/// Renders multiple `(pdf_page_index, exact_width, exact_height)` requests against one PDF,
/// sharing font/glyph caches across pages.
pub fn render_pdf_batch_exact_rgba(
    bytes: Arc<Vec<u8>>,
    requests: &[(usize, u32, u32)],
) -> Result<Vec<Option<RenderedPdfPage>>, PdfError> {
    let pdf = get_or_open_pdf(bytes)?;
    let pages = pdf.pages();
    let mut state = PdfRenderState::new();

    let mut out = Vec::with_capacity(requests.len());
    for &(page_index, req_w, req_h) in requests {
        let Some(page) = pages.get(page_index) else {
            out.push(None);
            continue;
        };
        let px_w = req_w.clamp(1, MAX_RENDER_PIXEL_DIM) as u16;
        let px_h = req_h.clamp(1, MAX_RENDER_PIXEL_DIM) as u16;
        out.push(Some(state.render_page(page, px_w, px_h)));
    }
    Ok(out)
}

/// Renders one PDF page and encodes it as PNG bytes within `max_w x max_h`.
pub fn render_pdf_page_png(
    bytes: Arc<Vec<u8>>,
    page_index: usize,
    max_w: u32,
    max_h: u32,
) -> Result<(u32, u32, Vec<u8>), PdfError> {
    let rendered = render_pdf_page_rgba(bytes, page_index, max_w, max_h)?;
    let png = papercraft_raster::png_encode(rendered.width, rendered.height, &rendered.rgba)
        .map_err(|err| PdfError::EncodeFailed(err.to_string()))?;
    Ok((rendered.width, rendered.height, png))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod cache_tests {
    use super::*;

    fn sample_pdf_bytes(r: f32, g: f32, b: f32) -> Arc<Vec<u8>> {
        let content = format!("{r:.3} {g:.3} {b:.3} rg\n0 0 100 100 re f\n");
        let objs = [
            "1 0 obj\n<< /Type /Catalog /Pages 2 0 R >>\nendobj\n".to_string(),
            "2 0 obj\n<< /Type /Pages /Kids [3 0 R] /Count 1 >>\nendobj\n".to_string(),
            "3 0 obj\n<< /Type /Page /Parent 2 0 R /MediaBox [0 0 100 100] /Contents 4 0 R >>\nendobj\n"
                .to_string(),
            format!(
                "4 0 obj\n<< /Length {} >>\nstream\n{content}endstream\nendobj\n",
                content.len()
            ),
        ];
        let mut pdf = Vec::from(&b"%PDF-1.4\n"[..]);
        let mut offsets = vec![0usize];
        for obj in &objs {
            offsets.push(pdf.len());
            pdf.extend_from_slice(obj.as_bytes());
        }
        let xref_offset = pdf.len();
        pdf.extend_from_slice(b"xref\n0 5\n0000000000 65535 f \n");
        for off in &offsets[1..] {
            pdf.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
        }
        pdf.extend_from_slice(
            format!("trailer\n<< /Size 5 /Root 1 0 R >>\nstartxref\n{xref_offset}\n%%EOF\n")
                .as_bytes(),
        );
        Arc::new(pdf)
    }

    #[test]
    fn thread_pdf_cache_hits_same_arc_and_bounds_lru_entries() {
        clear_pdf_cache();
        let a = sample_pdf_bytes(0.0, 0.0, 1.0);
        let b = sample_pdf_bytes(1.0, 0.0, 0.0);
        let c = sample_pdf_bytes(0.0, 1.0, 0.0);

        let pdf_a1 = get_or_open_pdf(Arc::clone(&a)).expect("parse A");
        let pdf_a2 = get_or_open_pdf(Arc::clone(&a)).expect("cache hit A");
        assert!(Arc::ptr_eq(&pdf_a1, &pdf_a2));

        let _ = get_or_open_pdf(Arc::clone(&b)).expect("parse B");
        let _ = get_or_open_pdf(Arc::clone(&c)).expect("parse C");

        let count = THREAD_PDF_CACHE.with(|cell| cell.borrow().len());
        assert_eq!(count, MAX_CACHED_OPEN_PDFS);

        // Oldest (`a`) was evicted when inserting `b` and `c`.
        let pdf_a3 = get_or_open_pdf(Arc::clone(&a)).expect("re-parse A");
        assert!(!Arc::ptr_eq(&pdf_a1, &pdf_a3));
        clear_pdf_cache();
    }

    #[test]
    fn pooled_vello_buffers_handle_shrink_grow_and_nested_renders() {
        clear_pdf_cache();
        let pdf_a = sample_pdf_bytes(0.0, 0.0, 1.0);
        let pdf_b = sample_pdf_bytes(1.0, 0.0, 0.0);

        let large = render_pdf_page_rgba(Arc::clone(&pdf_a), 0, 400, 400).expect("large");
        assert_eq!((large.width, large.height), (400, 400));
        assert_eq!(large.rgba.len(), 400 * 400 * 4);
        assert!(large.rgba[2] > 200 && large.rgba[0] < 40);

        let small = with_pdf_renderer(Arc::clone(&pdf_a), |renderer| {
            // Re-entrant nested render must not panic on RefCell borrow.
            let nested = render_pdf_page_rgba(Arc::clone(&pdf_b), 0, 80, 80).expect("nested");
            assert!(nested.rgba[0] > 200 && nested.rgba[2] < 40);
            renderer.render_page_rgba(0, 64, 64).expect("small")
        })
        .expect("with_pdf_renderer");
        assert_eq!((small.width, small.height), (64, 64));
        assert_eq!(small.rgba.len(), 64 * 64 * 4);
        assert!(small.rgba[2] > 200 && small.rgba[0] < 40);
        clear_pdf_cache();
    }
}
