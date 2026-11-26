//! Off-thread Web Worker PDF rasterizer reusing the already-loaded Trunk WASM bundle.
//!
//! Each PDF binary is transferred into the Dedicated Worker once and kept in a small
//! worker-local LRU cache, while rendered RGBA frames are transferred back zero-copy via
//! `ArrayBuffer` transfer lists.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use js_sys::{Array, Object, Reflect, Uint8Array};
use papercraft_engine::RenderedPdfPage;
use papercraft_ui_egui::{WebPdfRenderRequest, WebPdfRenderResult, WebPdfWorkerHooks};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::{JsValue, wasm_bindgen};

use crate::web_files::SharedEguiCtx;

const MAX_WORKER_CACHED_DOCS: usize = 4;

#[derive(Default)]
struct WorkerDocCache {
    tick: u64,
    docs: Vec<(String, Arc<Vec<u8>>, u64)>,
}

impl WorkerDocCache {
    fn insert(&mut self, doc_key: String, bytes: Vec<u8>) {
        self.tick = self.tick.saturating_add(1);
        let now = self.tick;
        let shared = Arc::new(bytes);
        if let Some(existing) = self.docs.iter_mut().find(|(k, _, _)| *k == doc_key) {
            existing.1 = shared;
            existing.2 = now;
            return;
        }
        while self.docs.len() >= MAX_WORKER_CACHED_DOCS {
            if let Some((oldest_idx, _)) =
                self.docs.iter().enumerate().min_by_key(|(_, (_, _, t))| *t)
            {
                self.docs.swap_remove(oldest_idx);
            } else {
                break;
            }
        }
        self.docs.push((doc_key, shared, now));
    }

    fn get(&mut self, doc_key: &str) -> Option<Arc<Vec<u8>>> {
        self.tick = self.tick.saturating_add(1);
        let now = self.tick;
        let entry = self.docs.iter_mut().find(|(k, _, _)| k == doc_key)?;
        entry.2 = now;
        Some(Arc::clone(&entry.1))
    }
}

thread_local! {
    static WORKER_PDF_DOCS: RefCell<WorkerDocCache> = RefCell::new(WorkerDocCache::default());
}

fn failed_render_object() -> JsValue {
    let obj = Object::new();
    let _ = Reflect::set(&obj, &JsValue::from_str("ok"), &JsValue::FALSE);
    obj.into()
}

/// Registers PDF bytes in the worker thread's bounded document cache under `doc_key`.
#[wasm_bindgen]
pub fn papercraft_pdf_worker_register_doc(doc_key: String, bytes: Vec<u8>) -> bool {
    if doc_key.is_empty() || bytes.is_empty() {
        return false;
    }
    WORKER_PDF_DOCS.with(|cache| {
        cache.borrow_mut().insert(doc_key, bytes);
    });
    true
}

/// Rasterizes one PDF page on the Web Worker thread into a standalone transferable `Uint8Array`.
#[wasm_bindgen]
pub fn papercraft_pdf_worker_render_page(
    doc_key: &str,
    page_index: usize,
    max_dim: u32,
) -> JsValue {
    let Some(bytes) = WORKER_PDF_DOCS.with(|cache| cache.borrow_mut().get(doc_key)) else {
        return failed_render_object();
    };
    let Ok(rendered) =
        papercraft_engine::Session::render_pdf_page_rgba(bytes, page_index, max_dim, max_dim)
    else {
        return failed_render_object();
    };
    let expected_len = (rendered.width as usize)
        .saturating_mul(rendered.height as usize)
        .saturating_mul(4);
    if rendered.width == 0 || rendered.height == 0 || rendered.rgba.len() != expected_len {
        return failed_render_object();
    }

    // `Uint8Array::from` allocates an isolated JS ArrayBuffer so the worker can transfer
    // ownership back to the UI thread without detaching WASM linear memory.
    let rgba_u8 = Uint8Array::from(rendered.rgba.as_slice());
    let obj = Object::new();
    if Reflect::set(&obj, &JsValue::from_str("ok"), &JsValue::TRUE).is_err()
        || Reflect::set(
            &obj,
            &JsValue::from_str("width"),
            &JsValue::from_f64(f64::from(rendered.width)),
        )
        .is_err()
        || Reflect::set(
            &obj,
            &JsValue::from_str("height"),
            &JsValue::from_f64(f64::from(rendered.height)),
        )
        .is_err()
        || Reflect::set(&obj, &JsValue::from_str("rgba"), rgba_u8.as_ref()).is_err()
    {
        return failed_render_object();
    }
    obj.into()
}

fn extract_quoted_after(text: &str, anchor: &str, suffix: &str) -> Option<String> {
    let mut rest = text;
    while let Some(pos) = rest.find(anchor) {
        let after = &rest[pos + anchor.len()..];
        if let Some(q_idx) = after.find(['\'', '"']) {
            let quote = after.as_bytes()[q_idx] as char;
            let body = &after[q_idx + 1..];
            if let Some(end_idx) = body.find(quote) {
                let candidate = body[..end_idx].trim();
                if candidate.ends_with(suffix) && !candidate.is_empty() {
                    return Some(candidate.to_string());
                }
            }
        }
        rest = after;
    }
    None
}

fn extract_js_and_wasm_from_module_script(script: &str) -> (Option<String>, Option<String>) {
    let js_rel = extract_quoted_after(script, "from", ".js");
    let wasm_rel = extract_quoted_after(script, "module_or_path", ".wasm")
        .or_else(|| extract_quoted_after(script, "_bg.wasm", ""));
    (js_rel, wasm_rel)
}

fn resolve_abs_url(raw: &str, base_href: &str) -> Option<String> {
    web_sys::Url::new_with_base(raw, base_href)
        .ok()
        .map(|u| u.href())
}

fn find_link_href_ending(doc: &web_sys::Document, selector: &str, suffix: &str) -> Option<String> {
    let list = doc.query_selector_all(selector).ok()?;
    let mut fallback: Option<String> = None;
    for idx in 0..list.length() {
        let Some(node) = list.item(idx) else {
            continue;
        };
        let Ok(el) = node.dyn_into::<web_sys::Element>() else {
            continue;
        };
        let Some(href) = el.get_attribute("href") else {
            continue;
        };
        if !href.ends_with(suffix) {
            continue;
        }
        if href.contains("papercraft") {
            return Some(href);
        }
        if fallback.is_none() {
            fallback = Some(href);
        }
    }
    fallback
}

fn discover_wasm_asset_urls() -> Option<(String, String)> {
    let window = web_sys::window()?;
    let doc = window.document()?;
    let base_href = window.location().href().ok()?;

    let mut js_raw = find_link_href_ending(&doc, "link[rel='modulepreload']", ".js");
    let mut wasm_raw = find_link_href_ending(&doc, "link[rel='preload']", ".wasm");

    if (js_raw.is_none() || wasm_raw.is_none())
        && let Ok(scripts) = doc.query_selector_all("script[type='module']")
    {
        for idx in 0..scripts.length() {
            let Some(node) = scripts.item(idx) else {
                continue;
            };
            let Some(text) = node.text_content() else {
                continue;
            };
            let (found_js, found_wasm) = extract_js_and_wasm_from_module_script(&text);
            if js_raw.is_none() {
                js_raw = found_js;
            }
            if wasm_raw.is_none() {
                wasm_raw = found_wasm;
            }
            if js_raw.is_some() && wasm_raw.is_some() {
                break;
            }
        }
    }

    let js_abs = resolve_abs_url(&js_raw?, &base_href)?;
    let wasm_abs = resolve_abs_url(&wasm_raw?, &base_href)?;
    Some((js_abs, wasm_abs))
}

fn build_worker_bootstrap_script(js_url: &str, wasm_url: &str) -> Option<String> {
    let js_lit = serde_json::to_string(js_url).ok()?;
    let wasm_lit = serde_json::to_string(wasm_url).ok()?;
    Some(format!(
        r#"
const JS_URL = {js_lit};
const WASM_URL = {wasm_lit};
let mod = null;
const pending = [];

async function processMsg(data) {{
    if (!data || !mod) return;
    if (data.type === 'register') {{
        if (typeof mod.papercraft_pdf_worker_register_doc === 'function' && data.buffer) {{
            mod.papercraft_pdf_worker_register_doc(String(data.docKey || ''), new Uint8Array(data.buffer));
        }}
        return;
    }}
    if (data.type === 'render') {{
        const jobId = Number(data.jobId || 0);
        try {{
            const fn = mod.papercraft_pdf_worker_render_page;
            if (typeof fn !== 'function') {{
                self.postMessage({{ type: 'rendered', jobId, ok: false }});
                return;
            }}
            const res = fn(String(data.docKey || ''), Number(data.pageIndex || 0), Number(data.maxDim || 0));
            if (res && res.ok && res.rgba && res.rgba.buffer) {{
                const buf = res.rgba.buffer;
                self.postMessage({{
                    type: 'rendered',
                    jobId,
                    ok: true,
                    width: res.width,
                    height: res.height,
                    buffer: buf,
                    byteOffset: res.rgba.byteOffset,
                    byteLength: res.rgba.byteLength
                }}, [buf]);
            }} else {{
                self.postMessage({{ type: 'rendered', jobId, ok: false }});
            }}
        }} catch (_err) {{
            self.postMessage({{ type: 'rendered', jobId, ok: false }});
        }}
    }}
}}

self.onmessage = (ev) => {{
    if (!mod) {{
        pending.push(ev.data);
        return;
    }}
    void processMsg(ev.data);
}};

(async () => {{
    try {{
        const loaded = await import(JS_URL);
        if (typeof loaded.default === 'function') {{
            await loaded.default({{ module_or_path: WASM_URL }});
        }}
        if (typeof loaded.papercraft_pdf_worker_register_doc !== 'function'
            || typeof loaded.papercraft_pdf_worker_render_page !== 'function') {{
            self.postMessage({{ type: 'fatal', error: 'missing pdf worker exports' }});
            return;
        }}
        mod = loaded;
        self.postMessage({{ type: 'ready' }});
        while (pending.length > 0) {{
            await processMsg(pending.shift());
        }}
    }} catch (err) {{
        self.postMessage({{ type: 'fatal', error: String(err) }});
    }}
}})();
"#
    ))
}

fn create_module_worker(script: &str) -> Option<(web_sys::Worker, String)> {
    let parts = Array::of1(&JsValue::from_str(script));
    let bag = web_sys::BlobPropertyBag::new();
    bag.set_type("application/javascript");
    let blob = web_sys::Blob::new_with_str_sequence_and_options(&parts, &bag).ok()?;
    let blob_url = web_sys::Url::create_object_url_with_blob(&blob).ok()?;
    let opts = web_sys::WorkerOptions::new();
    opts.set_type(web_sys::WorkerType::Module);
    match web_sys::Worker::new_with_options(&blob_url, &opts) {
        Ok(worker) => Some((worker, blob_url)),
        Err(_) => {
            let _ = web_sys::Url::revoke_object_url(&blob_url);
            None
        }
    }
}

fn decode_rendered_message(data: &JsValue) -> Option<WebPdfRenderResult> {
    let job_id_f = Reflect::get(data, &JsValue::from_str("jobId"))
        .ok()?
        .as_f64()?;
    if !job_id_f.is_finite() || job_id_f < 0.0 {
        return None;
    }
    let job_id = job_id_f as u64;
    let ok = Reflect::get(data, &JsValue::from_str("ok"))
        .ok()
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !ok {
        return Some(WebPdfRenderResult {
            job_id,
            rendered: None,
        });
    }

    let width = Reflect::get(data, &JsValue::from_str("width"))
        .ok()
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && *v >= 1.0 && *v <= f64::from(u32::MAX))?
        as u32;
    let height = Reflect::get(data, &JsValue::from_str("height"))
        .ok()
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && *v >= 1.0 && *v <= f64::from(u32::MAX))?
        as u32;
    let buffer = Reflect::get(data, &JsValue::from_str("buffer")).ok()?;
    let byte_offset = Reflect::get(data, &JsValue::from_str("byteOffset"))
        .ok()
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && *v >= 0.0)
        .unwrap_or(0.0) as u32;
    let byte_length = Reflect::get(data, &JsValue::from_str("byteLength"))
        .ok()
        .and_then(|v| v.as_f64())
        .filter(|v| v.is_finite() && *v >= 0.0)? as u32;

    let expected_len = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(4);
    if byte_length as usize != expected_len || expected_len == 0 {
        return Some(WebPdfRenderResult {
            job_id,
            rendered: None,
        });
    }
    let rgba =
        Uint8Array::new_with_byte_offset_and_length(&buffer, byte_offset, byte_length).to_vec();
    if rgba.len() != expected_len {
        return Some(WebPdfRenderResult {
            job_id,
            rendered: None,
        });
    }
    Some(WebPdfRenderResult {
        job_id,
        rendered: Some(RenderedPdfPage {
            width,
            height,
            rgba,
        }),
    })
}

fn request_egui_repaint(ctx_slot: &SharedEguiCtx) {
    if let Ok(guard) = ctx_slot.lock()
        && let Some(egui_ctx) = guard.as_ref()
    {
        egui_ctx.request_repaint();
    }
}

fn revoke_blob_slot(blob_slot: &Arc<Mutex<Option<String>>>) {
    if let Ok(mut guard) = blob_slot.lock()
        && let Some(url) = guard.take()
    {
        let _ = web_sys::Url::revoke_object_url(&url);
    }
}

/// Main-thread handle to the dedicated PDF rasterizer Web Worker.
pub struct WebPdfWorkerBridge {
    worker: web_sys::Worker,
    failed: Arc<AtomicBool>,
    results: Arc<Mutex<Vec<WebPdfRenderResult>>>,
    registered_docs: Vec<String>,
}

impl WebPdfWorkerBridge {
    /// Attempts to spawn a dedicated module Web Worker reusing the app's cached WASM assets.
    pub fn try_spawn(ctx_slot: SharedEguiCtx) -> Option<Self> {
        let (js_url, wasm_url) = discover_wasm_asset_urls()?;
        let script = build_worker_bootstrap_script(&js_url, &wasm_url)?;
        let (worker, blob_url) = create_module_worker(&script)?;

        let failed = Arc::new(AtomicBool::new(false));
        let results = Arc::new(Mutex::new(Vec::new()));
        let blob_slot = Arc::new(Mutex::new(Some(blob_url)));

        let on_msg_results = Arc::clone(&results);
        let on_msg_failed = Arc::clone(&failed);
        let on_msg_ctx = Arc::clone(&ctx_slot);
        let on_msg_blob = Arc::clone(&blob_slot);
        let on_message = Closure::<dyn FnMut(web_sys::MessageEvent)>::wrap(Box::new(
            move |ev: web_sys::MessageEvent| {
                let data = ev.data();
                let kind = Reflect::get(&data, &JsValue::from_str("type"))
                    .ok()
                    .and_then(|v| v.as_string())
                    .unwrap_or_default();
                match kind.as_str() {
                    "ready" => {
                        revoke_blob_slot(&on_msg_blob);
                    }
                    "fatal" => {
                        revoke_blob_slot(&on_msg_blob);
                        on_msg_failed.store(true, Ordering::Relaxed);
                        request_egui_repaint(&on_msg_ctx);
                    }
                    "rendered" => {
                        if let Some(res) = decode_rendered_message(&data)
                            && let Ok(mut guard) = on_msg_results.lock()
                        {
                            guard.push(res);
                        }
                        request_egui_repaint(&on_msg_ctx);
                    }
                    _ => {}
                }
            },
        ));
        worker.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        on_message.forget();

        let on_err_failed = Arc::clone(&failed);
        let on_err_ctx = Arc::clone(&ctx_slot);
        let on_err_blob = Arc::clone(&blob_slot);
        let on_error = Closure::<dyn FnMut(web_sys::Event)>::wrap(Box::new(move |_| {
            revoke_blob_slot(&on_err_blob);
            on_err_failed.store(true, Ordering::Relaxed);
            request_egui_repaint(&on_err_ctx);
        }));
        worker.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        on_error.forget();

        let on_merr_failed = Arc::clone(&failed);
        let on_merr_ctx = ctx_slot;
        let on_merr_blob = blob_slot;
        let on_message_error =
            Closure::<dyn FnMut(web_sys::MessageEvent)>::wrap(Box::new(move |_| {
                revoke_blob_slot(&on_merr_blob);
                on_merr_failed.store(true, Ordering::Relaxed);
                request_egui_repaint(&on_merr_ctx);
            }));
        worker.set_onmessageerror(Some(on_message_error.as_ref().unchecked_ref()));
        on_message_error.forget();

        Some(Self {
            worker,
            failed,
            results,
            registered_docs: Vec::with_capacity(MAX_WORKER_CACHED_DOCS),
        })
    }

    fn mark_registered(&mut self, doc_key: &str) -> bool {
        if let Some(pos) = self.registered_docs.iter().position(|k| k == doc_key) {
            let existing = self.registered_docs.remove(pos);
            self.registered_docs.push(existing);
            return false;
        }
        while self.registered_docs.len() >= MAX_WORKER_CACHED_DOCS {
            self.registered_docs.remove(0);
        }
        self.registered_docs.push(doc_key.to_string());
        true
    }

    fn dispatch_request(&mut self, req: WebPdfRenderRequest) -> bool {
        if self.failed.load(Ordering::Relaxed) || req.bytes.is_empty() {
            return false;
        }
        let doc_key = format!("{:016x}_{}", req.fingerprint, req.byte_len);
        if self.mark_registered(&doc_key) {
            let u8_arr = Uint8Array::from(req.bytes.as_slice());
            let buf = u8_arr.buffer();
            let reg = Object::new();
            let reg_ok = Reflect::set(
                &reg,
                &JsValue::from_str("type"),
                &JsValue::from_str("register"),
            )
            .is_ok()
                && Reflect::set(
                    &reg,
                    &JsValue::from_str("docKey"),
                    &JsValue::from_str(&doc_key),
                )
                .is_ok()
                && Reflect::set(&reg, &JsValue::from_str("buffer"), buf.as_ref()).is_ok();
            let transfer = Array::of1(buf.as_ref());
            if !reg_ok
                || self
                    .worker
                    .post_message_with_transfer(reg.as_ref(), &transfer)
                    .is_err()
            {
                self.failed.store(true, Ordering::Relaxed);
                return false;
            }
        }

        let msg = Object::new();
        let msg_ok = Reflect::set(
            &msg,
            &JsValue::from_str("type"),
            &JsValue::from_str("render"),
        )
        .is_ok()
            && Reflect::set(
                &msg,
                &JsValue::from_str("jobId"),
                &JsValue::from_f64(req.job_id as f64),
            )
            .is_ok()
            && Reflect::set(
                &msg,
                &JsValue::from_str("docKey"),
                &JsValue::from_str(&doc_key),
            )
            .is_ok()
            && Reflect::set(
                &msg,
                &JsValue::from_str("pageIndex"),
                &JsValue::from_f64(req.page_index as f64),
            )
            .is_ok()
            && Reflect::set(
                &msg,
                &JsValue::from_str("maxDim"),
                &JsValue::from_f64(f64::from(req.max_dim)),
            )
            .is_ok();
        if !msg_ok || self.worker.post_message(msg.as_ref()).is_err() {
            self.failed.store(true, Ordering::Relaxed);
            return false;
        }
        true
    }

    /// Converts this bridge into UI-layer [`WebPdfWorkerHooks`].
    pub fn into_hooks(mut self) -> WebPdfWorkerHooks {
        let avail_failed = Arc::clone(&self.failed);
        let poll_results = Arc::clone(&self.results);
        WebPdfWorkerHooks {
            is_available: Box::new(move || !avail_failed.load(Ordering::Relaxed)),
            dispatch: Box::new(move |req| self.dispatch_request(req)),
            poll_results: Box::new(move || {
                if let Ok(mut guard) = poll_results.lock() {
                    std::mem::take(&mut *guard)
                } else {
                    Vec::new()
                }
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_js_and_wasm_urls_from_trunk_module_script() {
        let snippet = r#"
import init, * as bindings from '/papercraft_app-bd7057000b86c559.js';
const wasm = await init({ module_or_path: '/papercraft_app-bd7057000b86c559_bg.wasm' });
window.wasmBindings = bindings;
"#;
        let (js_url, wasm_url) = extract_js_and_wasm_from_module_script(snippet);
        assert_eq!(
            js_url.as_deref(),
            Some("/papercraft_app-bd7057000b86c559.js")
        );
        assert_eq!(
            wasm_url.as_deref(),
            Some("/papercraft_app-bd7057000b86c559_bg.wasm")
        );
    }
}
