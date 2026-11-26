//! Bounded GPU texture cache and background rendering helpers for PDF page backdrops.

use std::collections::{HashMap, HashSet, VecDeque};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::Condvar;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use egui::{Color32, CornerRadius, Margin, Mesh, Rect, Sense, Shape, Stroke, Vec2, pos2};
use papercraft_doc::{
    CustomPaperTemplate, CustomTemplateKind, Document, EmbeddedImageTemplate, EmbeddedPdf,
    NotebookPage, PdfPageBackdrop,
};
use papercraft_engine::{RenderedPdfPage, Session};
use papercraft_sync_protocol::now_unix_ms;

use crate::icons::{Icon, paint_icon};
use crate::theme::Tokens;

/// Request dispatched to the browser Web Worker to rasterize one PDF page off the UI thread.
#[derive(Clone, Debug)]
pub struct WebPdfRenderRequest {
    pub job_id: u64,
    pub fingerprint: u64,
    pub byte_len: usize,
    pub bytes: Arc<Vec<u8>>,
    pub page_index: usize,
    pub max_dim: u32,
}

/// Completed RGBA rasterization frame returned from the browser Web Worker.
#[derive(Clone, Debug)]
pub struct WebPdfRenderResult {
    pub job_id: u64,
    pub rendered: Option<RenderedPdfPage>,
}

/// Platform bridge allowing `papercraft-ui-egui` to delegate Web PDF rasterization to a Worker.
pub struct WebPdfWorkerHooks {
    pub is_available: Box<dyn Fn() -> bool>,
    pub dispatch: Box<dyn FnMut(WebPdfRenderRequest) -> bool>,
    pub poll_results: Box<dyn FnMut() -> Vec<WebPdfRenderResult>>,
}

const MAX_CANVAS_TEXTURES: usize = 24;
const MAX_THUMB_TEXTURES: usize = 128;
const MAX_FAILED_KEYS: usize = 512;
const THUMB_MAX_DIM: u32 = 300;
#[cfg(not(target_arch = "wasm32"))]
const MAX_PENDING_CANVAS_JOBS: usize = 8;
#[cfg(not(target_arch = "wasm32"))]
const MAX_PENDING_THUMB_JOBS: usize = 32;
#[cfg(not(target_arch = "wasm32"))]
const PDF_SESSION_IDLE_KEEPALIVE: Duration = Duration::from_millis(1500);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct PdfTexKey {
    kind: u8,
    fingerprint: u64,
    byte_len: usize,
    page_index: usize,
    max_dim: u32,
}

#[derive(Clone)]
struct PdfTexEntry {
    handle: egui::TextureHandle,
    last_tick: u64,
}

#[derive(Clone, Default)]
struct PdfTexCache {
    entries: HashMap<PdfTexKey, PdfTexEntry>,
    failed: HashSet<PdfTexKey>,
    tick: u64,
}

fn fnv1a_extend(mut hash: u64, bytes: &[u8]) -> u64 {
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

fn pdf_bytes_fingerprint(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf29ce484222325_u64;
    hash = fnv1a_extend(hash, &bytes.len().to_le_bytes());
    let head = &bytes[..bytes.len().min(128)];
    hash = fnv1a_extend(hash, head);
    if bytes.len() > 256 {
        let mid_start = bytes.len() / 2 - 64;
        hash = fnv1a_extend(hash, &bytes[mid_start..mid_start + 128]);
    }
    if bytes.len() > 128 {
        let tail_start = bytes.len() - 128;
        hash = fnv1a_extend(hash, &bytes[tail_start..]);
    }
    hash
}

fn is_thumb_dim(max_dim: u32) -> bool {
    max_dim <= THUMB_MAX_DIM
}

fn cache_id_for_backdrop(is_thumb: bool) -> egui::Id {
    egui::Id::unique(("papercraft_pdf_tex_cache", is_thumb))
}

fn insert_rendered_texture(
    ctx: &egui::Context,
    key: PdfTexKey,
    rgba: Option<papercraft_engine::RenderedPdfPage>,
) {
    let is_thumb = is_thumb_dim(key.max_dim);
    let cache_id = cache_id_for_backdrop(is_thumb);
    let cap = if is_thumb {
        MAX_THUMB_TEXTURES
    } else {
        MAX_CANVAS_TEXTURES
    };

    let Some(rendered) = rgba else {
        ctx.data_mut(|d| {
            let cache = d.get_temp_mut_or_default::<PdfTexCache>(cache_id);
            if cache.failed.len() >= MAX_FAILED_KEYS {
                cache.failed.clear();
            }
            cache.failed.insert(key);
        });
        return;
    };

    let color_image = egui::ColorImage::from_rgba_unmultiplied(
        [rendered.width as usize, rendered.height as usize],
        &rendered.rgba,
    );
    let handle = ctx.load_texture(
        format!(
            "papercraft_pdf_{:016x}_{}_{}",
            key.fingerprint, key.page_index, key.max_dim
        ),
        color_image,
        egui::TextureOptions::LINEAR,
    );

    ctx.data_mut(|d| {
        let cache = d.get_temp_mut_or_default::<PdfTexCache>(cache_id);
        cache.tick = cache.tick.saturating_add(1);
        let now = cache.tick;
        while cache.entries.len() >= cap {
            let Some(oldest) = cache
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_tick)
                .map(|(k, _)| *k)
            else {
                break;
            };
            cache.entries.remove(&oldest);
        }
        cache.entries.insert(
            key,
            PdfTexEntry {
                handle,
                last_tick: now,
            },
        );
    });
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, PartialEq, Eq)]
struct PdfDocIdentity {
    fingerprint: u64,
    byte_len: usize,
}

#[cfg(not(target_arch = "wasm32"))]
struct PendingRenderJob {
    key: PdfTexKey,
    bytes: Arc<Vec<u8>>,
    page_index: usize,
    max_dim: u32,
    ctx: egui::Context,
    seq: u64,
}

#[cfg(not(target_arch = "wasm32"))]
struct CompletedRender {
    key: PdfTexKey,
    rgba: Option<papercraft_engine::RenderedPdfPage>,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct RenderQueueState {
    pending_canvas: Vec<PendingRenderJob>,
    pending_thumbs: Vec<PendingRenderJob>,
    in_flight: Option<PdfTexKey>,
    completed: Vec<CompletedRender>,
    next_seq: u64,
    shutdown: bool,
}

#[cfg(not(target_arch = "wasm32"))]
struct RenderShared {
    state: Mutex<RenderQueueState>,
    condvar: Condvar,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone)]
struct PdfRenderCoordinator {
    inner: Arc<PdfRenderCoordinatorInner>,
}

#[cfg(not(target_arch = "wasm32"))]
struct PdfRenderCoordinatorInner {
    shared: Arc<RenderShared>,
    handle: Mutex<Option<std::thread::JoinHandle<()>>>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for PdfRenderCoordinatorInner {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.shared.state.lock() {
            guard.shutdown = true;
            guard.pending_canvas.clear();
            guard.pending_thumbs.clear();
        }
        self.shared.condvar.notify_all();
        if let Ok(mut h) = self.handle.lock()
            && let Some(join) = h.take()
        {
            let _ = join.join();
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl PdfRenderCoordinator {
    fn new() -> Self {
        let shared = Arc::new(RenderShared {
            state: Mutex::new(RenderQueueState::default()),
            condvar: Condvar::new(),
        });
        let worker_shared = Arc::clone(&shared);
        let handle = std::thread::Builder::new()
            .name("papercraft-pdf-render".to_string())
            .spawn(move || run_pdf_render_worker(worker_shared))
            .ok();
        Self {
            inner: Arc::new(PdfRenderCoordinatorInner {
                shared,
                handle: Mutex::new(handle),
            }),
        }
    }

    fn enqueue(
        &self,
        key: PdfTexKey,
        bytes: Arc<Vec<u8>>,
        page_index: usize,
        max_dim: u32,
        ctx: &egui::Context,
    ) {
        let Ok(mut state) = self.inner.shared.state.lock() else {
            return;
        };
        if state.shutdown
            || state.in_flight == Some(key)
            || state.completed.iter().any(|c| c.key == key)
        {
            return;
        }
        state.next_seq = state.next_seq.saturating_add(1);
        let seq = state.next_seq;
        let is_thumb = max_dim <= 300;
        let (queue, cap) = if is_thumb {
            (&mut state.pending_thumbs, MAX_PENDING_THUMB_JOBS)
        } else {
            (&mut state.pending_canvas, MAX_PENDING_CANVAS_JOBS)
        };
        if let Some(existing) = queue.iter_mut().find(|job| job.key == key) {
            existing.seq = seq;
            return;
        }
        if queue.len() >= cap
            && let Some((oldest_idx, _)) = queue.iter().enumerate().min_by_key(|(_, j)| j.seq)
        {
            queue.swap_remove(oldest_idx);
        }
        queue.push(PendingRenderJob {
            key,
            bytes,
            page_index,
            max_dim,
            ctx: ctx.clone(),
            seq,
        });
        self.inner.shared.condvar.notify_one();
    }

    fn drain_completed(&self) -> Vec<CompletedRender> {
        let Ok(mut state) = self.inner.shared.state.lock() else {
            return Vec::new();
        };
        if state.completed.is_empty() {
            return Vec::new();
        }
        std::mem::take(&mut state.completed)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn pop_highest_seq(
    queue: &mut Vec<PendingRenderJob>,
    doc_filter: Option<PdfDocIdentity>,
) -> Option<PendingRenderJob> {
    let idx = queue
        .iter()
        .enumerate()
        .filter(|(_, job)| {
            doc_filter.is_none_or(|id| {
                job.key.fingerprint == id.fingerprint && job.key.byte_len == id.byte_len
            })
        })
        .max_by_key(|(_, job)| job.seq)
        .map(|(i, _)| i)?;
    Some(queue.swap_remove(idx))
}

#[cfg(not(target_arch = "wasm32"))]
fn pop_next_render_job(
    state: &mut RenderQueueState,
    preferred: Option<PdfDocIdentity>,
) -> Option<PendingRenderJob> {
    if let Some(id) = preferred
        && let Some(job) = pop_highest_seq(&mut state.pending_canvas, Some(id))
    {
        return Some(job);
    }
    if let Some(job) = pop_highest_seq(&mut state.pending_canvas, None) {
        return Some(job);
    }
    if let Some(id) = preferred
        && let Some(job) = pop_highest_seq(&mut state.pending_thumbs, Some(id))
    {
        return Some(job);
    }
    pop_highest_seq(&mut state.pending_thumbs, None)
}

#[cfg(not(target_arch = "wasm32"))]
fn take_session_followup_job(
    shared: &RenderShared,
    doc_id: PdfDocIdentity,
) -> Option<PendingRenderJob> {
    let Ok(mut state) = shared.state.lock() else {
        return None;
    };
    state.in_flight = None;
    let deadline = std::time::Instant::now() + PDF_SESSION_IDLE_KEEPALIVE;
    loop {
        if state.shutdown {
            return None;
        }
        if let Some(job) = pop_highest_seq(&mut state.pending_canvas, Some(doc_id)) {
            state.in_flight = Some(job.key);
            return Some(job);
        }
        if !state.pending_canvas.is_empty() {
            // Yield current PDF session immediately when another PDF's canvas page is waiting.
            return None;
        }
        if let Some(job) = pop_highest_seq(&mut state.pending_thumbs, Some(doc_id)) {
            state.in_flight = Some(job.key);
            return Some(job);
        }
        if !state.pending_thumbs.is_empty() {
            return None;
        }
        let now = std::time::Instant::now();
        if now >= deadline {
            return None;
        }
        let Ok((next_state, wait_res)) = shared.condvar.wait_timeout(state, deadline - now) else {
            return None;
        };
        state = next_state;
        if wait_res.timed_out()
            && state.pending_canvas.is_empty()
            && state.pending_thumbs.is_empty()
        {
            return None;
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn record_completed_job(
    shared: &RenderShared,
    key: PdfTexKey,
    ctx: &egui::Context,
    rgba: Option<papercraft_engine::RenderedPdfPage>,
) {
    if let Ok(mut state) = shared.state.lock() {
        state.in_flight = None;
        state.completed.push(CompletedRender { key, rgba });
    }
    ctx.request_repaint();
}

#[cfg(not(target_arch = "wasm32"))]
fn run_pdf_render_worker(shared: Arc<RenderShared>) {
    loop {
        let first_job = {
            let Ok(mut state) = shared.state.lock() else {
                return;
            };
            while !state.shutdown
                && state.pending_canvas.is_empty()
                && state.pending_thumbs.is_empty()
            {
                let Ok(next) = shared.condvar.wait(state) else {
                    return;
                };
                state = next;
            }
            if state.shutdown {
                return;
            }
            let Some(job) = pop_next_render_job(&mut state, None) else {
                continue;
            };
            state.in_flight = Some(job.key);
            job
        };

        let doc_id = PdfDocIdentity {
            fingerprint: first_job.key.fingerprint,
            byte_len: first_job.key.byte_len,
        };
        let first_key = first_job.key;
        let first_ctx = first_job.ctx.clone();
        let open_res = Session::with_pdf_renderer(Arc::clone(&first_job.bytes), |renderer| {
            let mut job = Some(first_job);
            while let Some(current) = job {
                let rgba = renderer
                    .render_page_rgba(current.page_index, current.max_dim, current.max_dim)
                    .ok();
                record_completed_job(&shared, current.key, &current.ctx, rgba);
                job = take_session_followup_job(&shared, doc_id);
            }
        });
        if open_res.is_err() {
            record_completed_job(&shared, first_key, &first_ctx, None);
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn pdf_coordinator(ctx: &egui::Context) -> PdfRenderCoordinator {
    let id = egui::Id::unique("papercraft_pdf_render_coordinator");
    ctx.data_mut(|d| {
        if let Some(existing) = d.get_temp::<PdfRenderCoordinator>(id) {
            existing
        } else {
            let created = PdfRenderCoordinator::new();
            d.insert_temp(id, created.clone());
            created
        }
    })
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const WASM_MAX_PENDING_CANVAS: usize = 8;
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const WASM_MAX_PENDING_THUMBS: usize = 24;
#[cfg(target_arch = "wasm32")]
const WASM_FRAME_BUDGET_MS: f64 = 10.0;
#[cfg(target_arch = "wasm32")]
const WASM_MAX_THUMBS_PER_FRAME: usize = 3;
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const WASM_CANVAS_SETTLE_DELAY_MS: f64 = 240.0;
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const WASM_SIDEBAR_SETTLE_DELAY_MS: f64 = 280.0;
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const WASM_WRITING_COOLDOWN_MS: f64 = 650.0;
const SIDEBAR_SCROLL_ACTIVE_MS: f64 = 180.0;
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const WORKER_SIDEBAR_SETTLE_MS: f64 = 70.0;
#[cfg(target_arch = "wasm32")]
const MAX_WORKER_UPLOADS_PER_FRAME: usize = 2;
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
const MAX_COMPLETED_READY_QUEUE: usize = 32;

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
#[derive(Clone)]
struct WasmPdfJob {
    key: PdfTexKey,
    bytes: Arc<Vec<u8>>,
    page_index: usize,
    max_dim: u32,
    seq: u64,
}

#[derive(Default)]
struct WasmPdfQueueState {
    seq: u64,
    writing_active: bool,
    defer_canvas_until_ms: f64,
    defer_sidebar_until_ms: f64,
    defer_writing_until_ms: f64,
    sidebar_last_motion_ms: f64,
    pending_canvas: Vec<WasmPdfJob>,
    pending_thumbs: Vec<WasmPdfJob>,
    #[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
    next_job_id: u64,
    in_flight: Option<(u64, PdfTexKey)>,
    completed_ready: VecDeque<(PdfTexKey, Option<RenderedPdfPage>)>,
    worker_mode_active: bool,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
impl WasmPdfQueueState {
    fn next_seq(&mut self) -> u64 {
        self.seq = self.seq.saturating_add(1);
        self.seq
    }

    fn enqueue(&mut self, key: PdfTexKey, bytes: Arc<Vec<u8>>, page_index: usize, max_dim: u32) {
        if self.in_flight.as_ref().is_some_and(|(_, k)| *k == key)
            || self.completed_ready.iter().any(|(k, _)| *k == key)
        {
            return;
        }
        let seq = self.next_seq();
        let is_thumb = is_thumb_dim(max_dim);
        let (list, cap) = if is_thumb {
            (&mut self.pending_thumbs, WASM_MAX_PENDING_THUMBS)
        } else {
            (&mut self.pending_canvas, WASM_MAX_PENDING_CANVAS)
        };
        if let Some(existing) = list.iter_mut().find(|job| job.key == key) {
            existing.seq = seq;
            return;
        }
        if list.len() >= cap
            && let Some((oldest_idx, _)) = list.iter().enumerate().min_by_key(|(_, job)| job.seq)
        {
            list.swap_remove(oldest_idx);
        }
        list.push(WasmPdfJob {
            key,
            bytes,
            page_index,
            max_dim,
            seq,
        });
    }

    fn has_pending(&self) -> bool {
        !self.pending_canvas.is_empty() || !self.pending_thumbs.is_empty()
    }

    fn pop_most_recent(list: &mut Vec<WasmPdfJob>) -> Option<WasmPdfJob> {
        let (idx, _) = list.iter().enumerate().max_by_key(|(_, job)| job.seq)?;
        Some(list.swap_remove(idx))
    }

    fn pop_next_job(&mut self, now_ms: f64) -> Option<(WasmPdfJob, bool)> {
        if self.writing_active
            || now_ms < self.defer_writing_until_ms
            || now_ms < self.defer_canvas_until_ms
        {
            return None;
        }
        if let Some(job) = Self::pop_most_recent(&mut self.pending_canvas) {
            return Some((job, false));
        }
        if now_ms < self.defer_sidebar_until_ms {
            return None;
        }
        Self::pop_most_recent(&mut self.pending_thumbs).map(|job| (job, true))
    }

    fn pop_next_worker_job(&mut self, now_ms: f64) -> Option<(u64, WasmPdfJob)> {
        if self.in_flight.is_some() {
            return None;
        }
        let job = if let Some(job) = Self::pop_most_recent(&mut self.pending_canvas) {
            job
        } else if now_ms >= self.sidebar_last_motion_ms + WORKER_SIDEBAR_SETTLE_MS {
            Self::pop_most_recent(&mut self.pending_thumbs)?
        } else {
            return None;
        };
        self.next_job_id = self.next_job_id.saturating_add(1);
        let job_id = self.next_job_id;
        self.in_flight = Some((job_id, job.key));
        Some((job_id, job))
    }

    fn abandon_in_flight(&mut self, job_id: u64) {
        if self.in_flight.as_ref().is_some_and(|(id, _)| *id == job_id) {
            self.in_flight = None;
        }
    }

    fn record_worker_result(&mut self, result: WebPdfRenderResult) {
        let Some((active_id, key)) = self.in_flight else {
            return;
        };
        if active_id != result.job_id {
            return;
        }
        self.in_flight = None;
        while self.completed_ready.len() >= MAX_COMPLETED_READY_QUEUE {
            let _ = self.completed_ready.pop_front();
        }
        self.completed_ready.push_back((key, result.rendered));
    }

    fn take_ready_upload(&mut self) -> Option<(PdfTexKey, Option<RenderedPdfPage>)> {
        if !self.writing_active {
            return self.completed_ready.pop_front();
        }
        let idx = self
            .completed_ready
            .iter()
            .position(|(k, _)| !is_thumb_dim(k.max_dim))?;
        self.completed_ready.remove(idx)
    }

    fn cancel_in_flight_for_fallback(&mut self) {
        self.in_flight = None;
        self.worker_mode_active = false;
    }

    fn next_wakeup_delay_ms(&self, now_ms: f64) -> Option<u64> {
        if self.worker_mode_active {
            if !self.completed_ready.is_empty() && !self.writing_active {
                return Some(0);
            }
            if self.in_flight.is_none() {
                if !self.pending_canvas.is_empty() {
                    return Some(0);
                }
                if !self.pending_thumbs.is_empty() {
                    let target = self.sidebar_last_motion_ms + WORKER_SIDEBAR_SETTLE_MS;
                    return if target > now_ms {
                        Some(((target - now_ms).ceil() as u64).clamp(16, 250))
                    } else {
                        Some(0)
                    };
                }
            }
            return None;
        }
        if !self.has_pending() {
            return None;
        }
        if self.writing_active {
            return Some(48);
        }
        let base = self.defer_canvas_until_ms.max(self.defer_writing_until_ms);
        let target_ms = if !self.pending_canvas.is_empty() {
            base
        } else {
            base.max(self.defer_sidebar_until_ms)
        };
        if target_ms > now_ms {
            Some(((target_ms - now_ms).ceil() as u64).clamp(16, 700))
        } else {
            Some(0)
        }
    }
}

#[derive(Clone, Default)]
struct WasmPdfQueue {
    inner: Arc<Mutex<WasmPdfQueueState>>,
}

fn wasm_queue(ctx: &egui::Context) -> WasmPdfQueue {
    let id = egui::Id::unique("papercraft_wasm_pdf_render_queue");
    ctx.data_mut(|d| {
        if let Some(existing) = d.get_temp::<WasmPdfQueue>(id) {
            existing
        } else {
            let created = WasmPdfQueue::default();
            d.insert_temp(id, created.clone());
            created
        }
    })
}

/// Defers background/WASM thumbnail PDF rasterization while canvas zoom or pan is active.
pub(crate) fn note_canvas_camera_motion(ctx: &egui::Context) {
    let until_ms = now_unix_ms() as f64 + WASM_CANVAS_SETTLE_DELAY_MS;
    let queue = wasm_queue(ctx);
    if let Ok(mut st) = queue.inner.lock() {
        st.defer_canvas_until_ms = st.defer_canvas_until_ms.max(until_ms);
    }
}

/// Defers queued sidebar thumbnail PDF rasterization briefly while scrolling the thumbnail list.
pub(crate) fn note_sidebar_scroll_motion(ctx: &egui::Context) {
    let now_ms = now_unix_ms() as f64;
    let queue = wasm_queue(ctx);
    if let Ok(mut st) = queue.inner.lock() {
        st.sidebar_last_motion_ms = now_ms;
        st.defer_sidebar_until_ms = st
            .defer_sidebar_until_ms
            .max(now_ms + WASM_SIDEBAR_SETTLE_DELAY_MS);
    }
}

/// Returns `true` when sidebar thumbnail scrolling settled within the last ~180ms.
pub(crate) fn is_sidebar_scroll_active(ctx: &egui::Context) -> bool {
    let now_ms = now_unix_ms() as f64;
    let queue = wasm_queue(ctx);
    queue
        .inner
        .lock()
        .is_ok_and(|st| now_ms < st.sidebar_last_motion_ms + SIDEBAR_SCROLL_ACTIVE_MS)
}

/// Marks whether live stylus/pointer drawing or erasing is active so queued PDF work waits cleanly.
pub(crate) fn set_writing_active(ctx: &egui::Context, active: bool) {
    let now_ms = now_unix_ms() as f64;
    let queue = wasm_queue(ctx);
    let wake_delay = if let Ok(mut st) = queue.inner.lock() {
        let stopped = st.writing_active && !active;
        st.writing_active = active;
        if stopped {
            st.defer_writing_until_ms = st
                .defer_writing_until_ms
                .max(now_ms + WASM_WRITING_COOLDOWN_MS);
        }
        if stopped
            && (!st.completed_ready.is_empty() || (st.worker_mode_active && st.has_pending()))
        {
            Some(0)
        } else if stopped && st.has_pending() {
            Some(WASM_WRITING_COOLDOWN_MS as u64)
        } else {
            None
        }
    } else {
        None
    };
    if let Some(delay) = wake_delay {
        if delay == 0 {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(delay));
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn enqueue_wasm_render(
    ctx: &egui::Context,
    key: PdfTexKey,
    bytes: Arc<Vec<u8>>,
    page_index: usize,
    max_dim: u32,
) {
    let queue = wasm_queue(ctx);
    let wake_delay = if let Ok(mut st) = queue.inner.lock() {
        st.enqueue(key, bytes, page_index, max_dim);
        st.next_wakeup_delay_ms(now_unix_ms() as f64)
    } else {
        None
    };
    if let Some(delay) = wake_delay {
        if delay == 0 {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(delay));
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn pump_wasm_renders(ctx: &egui::Context) {
    let queue = wasm_queue(ctx);
    let frame_start_ms = now_unix_ms() as f64;
    let mut rendered_count = 0usize;

    loop {
        let next = queue
            .inner
            .lock()
            .ok()
            .and_then(|mut st| st.pop_next_job(now_unix_ms() as f64));
        let Some((job, is_thumb)) = next else {
            break;
        };
        if lookup_cached_texture(ctx, job.key, is_thumb).is_some()
            || is_failed_key(ctx, job.key, is_thumb)
        {
            continue;
        }
        let rendered = Session::render_pdf_page_rgba(
            Arc::clone(&job.bytes),
            job.page_index,
            job.max_dim,
            job.max_dim,
        )
        .ok();
        insert_rendered_texture(ctx, job.key, rendered);
        rendered_count += 1;

        let elapsed_ms = (now_unix_ms() as f64 - frame_start_ms).max(0.0);
        if !is_thumb
            || rendered_count >= WASM_MAX_THUMBS_PER_FRAME
            || elapsed_ms >= WASM_FRAME_BUDGET_MS
        {
            break;
        }
    }

    let remaining_delay = queue
        .inner
        .lock()
        .ok()
        .and_then(|st| st.next_wakeup_delay_ms(now_unix_ms() as f64));
    if let Some(delay) = remaining_delay {
        if delay == 0 {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(delay));
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn take_ready_uploads_batch(
    ctx: &egui::Context,
) -> (Vec<(PdfTexKey, Option<RenderedPdfPage>)>, bool) {
    let queue = wasm_queue(ctx);
    let mut batch = Vec::with_capacity(MAX_WORKER_UPLOADS_PER_FRAME);
    let mut more_ready = false;
    if let Ok(mut st) = queue.inner.lock() {
        st.worker_mode_active = true;
        for _ in 0..MAX_WORKER_UPLOADS_PER_FRAME {
            let Some(item) = st.take_ready_upload() else {
                break;
            };
            batch.push(item);
        }
        more_ready = !st.completed_ready.is_empty() && !st.writing_active;
    }
    (batch, more_ready)
}

#[cfg(target_arch = "wasm32")]
fn dispatch_worker_queue_if_idle(ctx: &egui::Context, worker: &mut WebPdfWorkerHooks) {
    let queue = wasm_queue(ctx);
    loop {
        let popped = queue
            .inner
            .lock()
            .ok()
            .and_then(|mut st| st.pop_next_worker_job(now_unix_ms() as f64));
        let Some((job_id, job)) = popped else {
            break;
        };
        let is_thumb = is_thumb_dim(job.max_dim);
        if lookup_cached_texture(ctx, job.key, is_thumb).is_some()
            || is_failed_key(ctx, job.key, is_thumb)
        {
            if let Ok(mut st) = queue.inner.lock() {
                st.abandon_in_flight(job_id);
            }
            continue;
        }
        let req = WebPdfRenderRequest {
            job_id,
            fingerprint: job.key.fingerprint,
            byte_len: job.key.byte_len,
            bytes: Arc::clone(&job.bytes),
            page_index: job.page_index,
            max_dim: job.max_dim,
        };
        if !(worker.dispatch)(req)
            && let Ok(mut st) = queue.inner.lock()
        {
            st.cancel_in_flight_for_fallback();
            st.enqueue(job.key, job.bytes, job.page_index, job.max_dim);
        }
        break;
    }

    let delay = queue
        .inner
        .lock()
        .ok()
        .and_then(|st| st.next_wakeup_delay_ms(now_unix_ms() as f64));
    if let Some(delay) = delay {
        if delay == 0 {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(Duration::from_millis(delay));
        }
    }
}

#[cfg(target_arch = "wasm32")]
fn pump_wasm_with_worker(ctx: &egui::Context, worker: &mut WebPdfWorkerHooks) {
    let results = (worker.poll_results)();
    if !results.is_empty()
        && let Ok(mut st) = wasm_queue(ctx).inner.lock()
    {
        for res in results {
            st.record_worker_result(res);
        }
    }
    let (batch, more_ready) = take_ready_uploads_batch(ctx);
    for (key, rendered) in batch {
        insert_rendered_texture(ctx, key, rendered);
    }
    if more_ready {
        ctx.request_repaint();
    }
    dispatch_worker_queue_if_idle(ctx, worker);
}

#[cfg(target_arch = "wasm32")]
fn fallback_from_worker(ctx: &egui::Context) {
    let queue = wasm_queue(ctx);
    let mut ready = Vec::with_capacity(MAX_WORKER_UPLOADS_PER_FRAME);
    if let Ok(mut st) = queue.inner.lock() {
        st.cancel_in_flight_for_fallback();
        for _ in 0..MAX_WORKER_UPLOADS_PER_FRAME {
            let Some(item) = st.take_ready_upload() else {
                break;
            };
            ready.push(item);
        }
    }
    for (key, rendered) in ready {
        insert_rendered_texture(ctx, key, rendered);
    }
    pump_wasm_renders(ctx);
}

/// Uploads completed background PDF renders into the texture cache for `ctx`, delegating
/// Web rasterization to the browser Web Worker when available.
pub(crate) fn pump_completed_renders_with_worker(
    ctx: &egui::Context,
    worker: Option<&mut WebPdfWorkerHooks>,
) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = worker;
        let coord = pdf_coordinator(ctx);
        for done in coord.drain_completed() {
            insert_rendered_texture(ctx, done.key, done.rgba);
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(w) = worker
            && (w.is_available)()
        {
            pump_wasm_with_worker(ctx, w);
        } else {
            fallback_from_worker(ctx);
        }
    }
}

/// Dispatches the highest-priority pending PDF job to the browser Web Worker if idle.
pub(crate) fn dispatch_pending_web_worker_job(
    ctx: &egui::Context,
    worker: Option<&mut WebPdfWorkerHooks>,
) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = (ctx, worker);
    }
    #[cfg(target_arch = "wasm32")]
    {
        if let Some(w) = worker
            && (w.is_available)()
        {
            if let Ok(mut st) = wasm_queue(ctx).inner.lock() {
                st.worker_mode_active = true;
            }
            dispatch_worker_queue_if_idle(ctx, w);
        }
    }
}

/// Uploads completed background PDF renders into the texture cache for `ctx`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn pump_completed_renders(ctx: &egui::Context) {
    pump_completed_renders_with_worker(ctx, None);
}

fn lookup_cached_texture(
    ctx: &egui::Context,
    key: PdfTexKey,
    is_thumb: bool,
) -> Option<egui::TextureId> {
    let cache_id = cache_id_for_backdrop(is_thumb);
    ctx.data_mut(|d| {
        let cache = d.get_temp_mut_or_default::<PdfTexCache>(cache_id);
        cache.tick = cache.tick.saturating_add(1);
        let now = cache.tick;
        cache.entries.get_mut(&key).map(|entry| {
            entry.last_tick = now;
            entry.handle.id()
        })
    })
}

fn lookup_fallback_canvas_texture(
    ctx: &egui::Context,
    fingerprint: u64,
    byte_len: usize,
    page_index: usize,
) -> Option<egui::TextureId> {
    for is_thumb in [false, true] {
        let cache_id = cache_id_for_backdrop(is_thumb);
        let found = ctx.data_mut(|d| {
            let cache = d.get_temp_mut_or_default::<PdfTexCache>(cache_id);
            cache
                .entries
                .iter()
                .filter(|(k, _)| {
                    k.kind == 0
                        && k.fingerprint == fingerprint
                        && k.byte_len == byte_len
                        && k.page_index == page_index
                })
                .max_by_key(|(k, _)| k.max_dim)
                .map(|(_, entry)| entry.handle.id())
        });
        if found.is_some() {
            return found;
        }
    }
    None
}

fn is_failed_key(ctx: &egui::Context, key: PdfTexKey, is_thumb: bool) -> bool {
    let cache_id = cache_id_for_backdrop(is_thumb);
    ctx.data(|d| {
        d.get_temp::<PdfTexCache>(cache_id)
            .is_some_and(|cache| cache.failed.contains(&key))
    })
}

fn resolve_backdrop_texture(
    ctx: &egui::Context,
    pdf: &EmbeddedPdf,
    page_index: usize,
    max_dim: u32,
) -> Option<egui::TextureId> {
    #[cfg(not(target_arch = "wasm32"))]
    pump_completed_renders(ctx);
    let fp = pdf_bytes_fingerprint(&pdf.bytes);
    let key = PdfTexKey {
        kind: 0,
        fingerprint: fp,
        byte_len: pdf.bytes.len(),
        page_index,
        max_dim,
    };
    let is_thumb = is_thumb_dim(max_dim);
    if let Some(hit) = lookup_cached_texture(ctx, key, is_thumb) {
        return Some(hit);
    }
    if is_failed_key(ctx, key, is_thumb) {
        return None;
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        let coord = pdf_coordinator(ctx);
        coord.enqueue(key, Arc::clone(&pdf.bytes), page_index, max_dim, ctx);
        if !is_thumb {
            return lookup_fallback_canvas_texture(ctx, fp, pdf.bytes.len(), page_index);
        }
        None
    }

    #[cfg(target_arch = "wasm32")]
    {
        enqueue_wasm_render(ctx, key, Arc::clone(&pdf.bytes), page_index, max_dim);
        lookup_fallback_canvas_texture(ctx, fp, pdf.bytes.len(), page_index)
    }
}

fn resolve_custom_image_texture(
    ctx: &egui::Context,
    tpl: &EmbeddedImageTemplate,
) -> Option<egui::TextureId> {
    if tpl.png_bytes.is_empty() {
        return None;
    }
    let fp = pdf_bytes_fingerprint(&tpl.png_bytes);
    let key = PdfTexKey {
        kind: 1,
        fingerprint: fp,
        byte_len: tpl.png_bytes.len(),
        page_index: 0,
        max_dim: 0,
    };
    let cache_id = egui::Id::unique("papercraft_custom_tpl_tex_cache");
    if let Some(hit) = ctx.data_mut(|d| {
        let cache = d.get_temp_mut_or_default::<PdfTexCache>(cache_id);
        cache.tick = cache.tick.saturating_add(1);
        let now = cache.tick;
        cache.entries.get_mut(&key).map(|entry| {
            entry.last_tick = now;
            entry.handle.id()
        })
    }) {
        return Some(hit);
    }

    let decoded = image::load_from_memory(&tpl.png_bytes).ok()?;
    let rgba = decoded.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    if w == 0 || h == 0 {
        return None;
    }
    let color_image = egui::ColorImage::from_rgba_unmultiplied([w, h], rgba.as_raw());
    let handle = ctx.load_texture(
        format!("papercraft_custom_tpl_{fp:016x}"),
        color_image,
        egui::TextureOptions::LINEAR,
    );
    let tex_id = handle.id();
    ctx.data_mut(|d| {
        let cache = d.get_temp_mut_or_default::<PdfTexCache>(cache_id);
        cache.tick = cache.tick.saturating_add(1);
        let now = cache.tick;
        while cache.entries.len() >= 32 {
            let Some(oldest) = cache
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.last_tick)
                .map(|(k, _)| *k)
            else {
                break;
            };
            cache.entries.remove(&oldest);
        }
        cache.entries.insert(
            key,
            PdfTexEntry {
                handle,
                last_tick: now,
            },
        );
    });
    Some(tex_id)
}

fn paint_rotated_texture(p: &egui::Painter, rect: Rect, tex_id: egui::TextureId, rotation: u8) {
    let uvs = match rotation % 4 {
        1 => [
            pos2(0.0, 1.0),
            pos2(0.0, 0.0),
            pos2(1.0, 0.0),
            pos2(1.0, 1.0),
        ],
        2 => [
            pos2(1.0, 1.0),
            pos2(0.0, 1.0),
            pos2(0.0, 0.0),
            pos2(1.0, 0.0),
        ],
        3 => [
            pos2(1.0, 0.0),
            pos2(1.0, 1.0),
            pos2(0.0, 1.0),
            pos2(0.0, 0.0),
        ],
        _ => [
            pos2(0.0, 0.0),
            pos2(1.0, 0.0),
            pos2(1.0, 1.0),
            pos2(0.0, 1.0),
        ],
    };
    let corners = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
    ];
    let mut mesh = Mesh::with_texture(tex_id);
    for (pos, uv) in corners.into_iter().zip(uvs) {
        mesh.vertices.push(egui::epaint::Vertex {
            pos,
            uv,
            color: Color32::WHITE,
        });
    }
    mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
    p.with_clip_rect(rect).add(Shape::mesh(mesh));
}

/// Chooses one of three quantized resolution tiers (`900`, `1500`, `2400`) from zoomed canvas dimensions.
#[must_use]
pub(crate) fn canvas_pdf_tier_dim(ctx: &egui::Context, page_rect: Rect) -> u32 {
    let display_max = page_rect.width().max(page_rect.height()).max(64.0) * ctx.pixels_per_point();
    #[cfg(target_arch = "wasm32")]
    {
        if display_max <= 900.0 {
            900
        } else if display_max <= 1400.0 {
            1400
        } else {
            2048
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        if display_max <= 900.0 {
            900
        } else if display_max <= 1500.0 {
            1500
        } else {
            2400
        }
    }
}

fn paint_backdrop(
    ctx: &egui::Context,
    p: &egui::Painter,
    rect: Rect,
    pdfs: &[EmbeddedPdf],
    backdrop: Option<PdfPageBackdrop>,
    rotation: u8,
    max_dim: u32,
) -> bool {
    let Some(backdrop) = backdrop else {
        return false;
    };
    let Some(pdf) = pdfs.get(backdrop.pdf_index) else {
        return false;
    };
    let Some(tex_id) = resolve_backdrop_texture(ctx, pdf, backdrop.page_index, max_dim) else {
        return false;
    };
    paint_rotated_texture(p, rect, tex_id, rotation);
    true
}

/// Paints the active document's PDF or custom image backdrop texture into `rect` with
/// an extra clockwise `quarter_turns` offset (used by modal previews).
pub(crate) fn paint_doc_pdf_backdrop_with_rotation(
    ctx: &egui::Context,
    p: &egui::Painter,
    rect: Rect,
    doc: &Document,
    max_dim: u32,
    extra_quarter_turns: u8,
) -> bool {
    let rotation = doc.backdrop_rotation.wrapping_add(extra_quarter_turns) % 4;
    if paint_backdrop(
        ctx,
        p,
        rect,
        &doc.pdf_documents,
        doc.pdf_backdrop,
        rotation,
        max_dim,
    ) {
        return true;
    }
    if let Some(img_idx) = doc.custom_image_backdrop
        && let Some(tpl) = doc.image_templates.get(img_idx)
        && let Some(tex_id) = resolve_custom_image_texture(ctx, tpl)
    {
        paint_rotated_texture(p, rect, tex_id, rotation);
        return true;
    }
    false
}

/// Paints a saved [`CustomPaperTemplate`] preview into `rect` rotated by `rotation` quarter turns.
pub(crate) fn paint_custom_template_preview_rotated(
    ctx: &egui::Context,
    p: &egui::Painter,
    rect: Rect,
    tpl: &CustomPaperTemplate,
    max_dim: u32,
    rotation: u8,
) -> bool {
    let rot = rotation % 4;
    match tpl.kind {
        CustomTemplateKind::Image => {
            let tpl_img = EmbeddedImageTemplate {
                name: tpl.name.clone(),
                png_bytes: Arc::clone(&tpl.bytes),
            };
            if let Some(tex_id) = resolve_custom_image_texture(ctx, &tpl_img) {
                paint_rotated_texture(p, rect, tex_id, rot);
                return true;
            }
            false
        }
        CustomTemplateKind::Pdf => {
            let pdf = EmbeddedPdf {
                name: tpl.name.clone(),
                bytes: Arc::clone(&tpl.bytes),
            };
            if let Some(tex_id) = resolve_backdrop_texture(ctx, &pdf, tpl.pdf_page_index, max_dim) {
                paint_rotated_texture(p, rect, tex_id, rot);
                return true;
            }
            false
        }
    }
}

/// Paints the active document's PDF or custom image backdrop texture into `rect` when present.
pub(crate) fn paint_doc_pdf_backdrop_if_present(
    ctx: &egui::Context,
    p: &egui::Painter,
    rect: Rect,
    doc: &Document,
    max_dim: u32,
) -> bool {
    paint_doc_pdf_backdrop_with_rotation(ctx, p, rect, doc, max_dim, 0)
}

/// Paints a specific `NotebookPage`'s PDF or custom image backdrop texture into `rect` when present.
pub(crate) fn paint_page_pdf_backdrop_if_present(
    ctx: &egui::Context,
    p: &egui::Painter,
    rect: Rect,
    doc: &Document,
    page: &NotebookPage,
    max_dim: u32,
) -> bool {
    if paint_backdrop(
        ctx,
        p,
        rect,
        &doc.pdf_documents,
        page.pdf_backdrop,
        page.backdrop_rotation,
        max_dim,
    ) {
        return true;
    }
    if let Some(img_idx) = page.custom_image_backdrop
        && let Some(tpl) = doc.image_templates.get(img_idx)
        && let Some(tex_id) = resolve_custom_image_texture(ctx, tpl)
    {
        paint_rotated_texture(p, rect, tex_id, page.backdrop_rotation);
        return true;
    }
    false
}

/// Renders a dismissible floating canvas error banner; returns `true` when dismissed.
pub(crate) fn render_floating_error_banner(
    ctx: &egui::Context,
    avail: Rect,
    id_salt: &str,
    message: &str,
    t: &Tokens,
) -> bool {
    let mut dismiss = false;
    let max_w = (avail.width() - 32.0).clamp(180.0, 520.0);
    egui::Area::new(egui::Id::unique(id_salt))
        .order(egui::Order::Foreground)
        .pivot(egui::Align2::CENTER_TOP)
        .fixed_pos(pos2(avail.center().x, avail.top() + 12.0))
        .show(ctx, |ui| {
            egui::Frame::NONE
                .fill(t.danger_bg)
                .stroke(Stroke::new(1.0, t.danger_border))
                .corner_radius(CornerRadius::same(8))
                .inner_margin(Margin::symmetric(12, 8))
                .show(ui, |ui| {
                    ui.set_max_width(max_w);
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let (icon_rect, _) =
                            ui.allocate_exact_size(Vec2::splat(16.0), Sense::hover());
                        paint_icon(ui.painter(), icon_rect, Icon::AlertCircle, t.danger_fg);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(message).size(12.0).color(t.danger_fg),
                            )
                            .wrap(),
                        );
                        let (close_rect, close_resp) =
                            ui.allocate_exact_size(Vec2::splat(20.0), Sense::click());
                        if close_resp.hovered() {
                            ui.painter().rect_filled(
                                close_rect,
                                CornerRadius::same(4),
                                t.danger_border.gamma_multiply(0.35),
                            );
                        }
                        paint_icon(
                            ui.painter(),
                            close_rect.shrink(3.0),
                            Icon::Close,
                            t.danger_fg,
                        );
                        if close_resp.on_hover_text("Dismiss error").clicked() {
                            dismiss = true;
                        }
                    });
                });
        });
    dismiss
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod wasm_queue_tests {
    use super::*;

    fn test_key(page_index: usize, max_dim: u32) -> PdfTexKey {
        PdfTexKey {
            kind: 0,
            fingerprint: 0x1234_5678,
            byte_len: 1024,
            page_index,
            max_dim,
        }
    }

    #[test]
    fn wasm_queue_prioritizes_canvas_over_thumbs_and_dedups() {
        let bytes = Arc::new(Vec::new());
        let mut q = WasmPdfQueueState::default();
        q.enqueue(test_key(0, 240), Arc::clone(&bytes), 0, 240);
        q.enqueue(test_key(1, 240), Arc::clone(&bytes), 1, 240);
        q.enqueue(test_key(1, 240), Arc::clone(&bytes), 1, 240);
        q.enqueue(test_key(0, 900), Arc::clone(&bytes), 0, 900);

        assert_eq!(q.pending_thumbs.len(), 2);
        assert_eq!(q.pending_canvas.len(), 1);

        let (first, first_is_thumb) = q.pop_next_job(1_000.0).expect("canvas job");
        assert!(!first_is_thumb);
        assert_eq!(first.max_dim, 900);

        let (second, second_is_thumb) = q.pop_next_job(1_000.0).expect("most recent thumb");
        assert!(second_is_thumb);
        assert_eq!(second.page_index, 1);

        let (third, third_is_thumb) = q.pop_next_job(1_000.0).expect("remaining thumb");
        assert!(third_is_thumb);
        assert_eq!(third.page_index, 0);
    }

    #[test]
    fn wasm_queue_respects_deferrals_and_caps_pending_thumbs() {
        let bytes = Arc::new(Vec::new());
        let mut q = WasmPdfQueueState::default();
        for p in 0..(WASM_MAX_PENDING_THUMBS + 5) {
            q.enqueue(test_key(p, 240), Arc::clone(&bytes), p, 240);
        }
        assert_eq!(q.pending_thumbs.len(), WASM_MAX_PENDING_THUMBS);

        q.enqueue(test_key(0, 900), Arc::clone(&bytes), 0, 900);
        q.writing_active = true;
        assert!(q.pop_next_job(1_000.0).is_none());
        q.writing_active = false;

        q.defer_canvas_until_ms = 1_140.0;
        assert!(q.pop_next_job(1_050.0).is_none());
        assert_eq!(q.next_wakeup_delay_ms(1_050.0), Some(90));

        // After canvas settles, canvas runs even if sidebar scrolling is active.
        q.defer_sidebar_until_ms = 1_300.0;
        let (canvas_job, is_thumb) = q.pop_next_job(1_200.0).expect("canvas ready");
        assert!(!is_thumb);
        assert_eq!(canvas_job.max_dim, 900);

        // Thumbs wait until sidebar settle time passes.
        assert!(q.pop_next_job(1_200.0).is_none());
        let (thumb_job, is_thumb) = q.pop_next_job(1_310.0).expect("thumb ready");
        assert!(is_thumb);
        assert_eq!(thumb_job.page_index, WASM_MAX_PENDING_THUMBS + 4);
    }

    #[test]
    fn wasm_fallback_honors_writing_cooldown_after_pen_lift() {
        let bytes = Arc::new(Vec::new());
        let mut q = WasmPdfQueueState::default();
        q.enqueue(test_key(0, 900), Arc::clone(&bytes), 0, 900);

        q.writing_active = true;
        assert!(q.pop_next_job(1_000.0).is_none());

        // Simulate pen lift setting WASM_WRITING_COOLDOWN_MS.
        q.writing_active = false;
        q.defer_writing_until_ms = 1_000.0 + WASM_WRITING_COOLDOWN_MS;
        assert!(q.pop_next_job(1_200.0).is_none());
        let wait = q.next_wakeup_delay_ms(1_200.0).expect("scheduled wakeup");
        assert!(wait > 0);

        let (job, is_thumb) = q
            .pop_next_job(1_000.0 + WASM_WRITING_COOLDOWN_MS + 1.0)
            .expect("canvas job after cooldown");
        assert!(!is_thumb);
        assert_eq!(job.max_dim, 900);
    }

    #[test]
    fn worker_queue_enforces_single_in_flight_priority_and_writing_upload_guard() {
        let bytes = Arc::new(Vec::new());
        let mut q = WasmPdfQueueState {
            worker_mode_active: true,
            sidebar_last_motion_ms: 1_000.0,
            ..Default::default()
        };
        q.enqueue(test_key(0, 160), Arc::clone(&bytes), 0, 160);
        q.enqueue(test_key(1, 900), Arc::clone(&bytes), 1, 900);

        // Even while writing_active and sidebar scrolling, canvas dispatches to worker immediately.
        q.writing_active = true;
        let (job_id_1, first) = q
            .pop_next_worker_job(1_010.0)
            .expect("canvas dispatches while writing");
        assert_eq!(first.max_dim, 900);
        assert_eq!(q.in_flight, Some((job_id_1, test_key(1, 900))));

        // Only 1 job may be in flight at a time, and duplicate key is ignored.
        q.enqueue(test_key(1, 900), Arc::clone(&bytes), 1, 900);
        assert!(q.pop_next_worker_job(2_000.0).is_none());

        q.record_worker_result(WebPdfRenderResult {
            job_id: job_id_1,
            rendered: Some(RenderedPdfPage {
                width: 2,
                height: 2,
                rgba: vec![255; 16],
            }),
        });
        assert!(q.in_flight.is_none());

        // Thumbs wait briefly for sidebar scroll settle, then dispatch while writing_active.
        assert!(q.pop_next_worker_job(1_020.0).is_none());
        let (job_id_2, second) = q
            .pop_next_worker_job(1_000.0 + WORKER_SIDEBAR_SETTLE_MS + 1.0)
            .expect("thumb dispatches after settle");
        assert_eq!(second.max_dim, 160);
        q.record_worker_result(WebPdfRenderResult {
            job_id: job_id_2,
            rendered: Some(RenderedPdfPage {
                width: 1,
                height: 1,
                rgba: vec![255; 4],
            }),
        });

        // While writing_active, canvas upload proceeds immediately while thumb upload waits.
        let (k1, _) = q.take_ready_upload().expect("canvas upload allowed");
        assert_eq!(k1.max_dim, 900);
        assert!(q.take_ready_upload().is_none());

        q.writing_active = false;
        let (k2, _) = q.take_ready_upload().expect("thumb upload after pen lift");
        assert_eq!(k2.max_dim, 160);
    }
}
