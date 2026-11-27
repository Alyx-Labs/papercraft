//! Cross-tab Web Locks API coordination with `localStorage` lease fallback.

use std::cell::{Cell, RefCell};
use std::future::Future;
use std::rc::Rc;

use js_sys::{Array, Date, Function, Object, Promise, Reflect};
use wasm_bindgen::JsCast;
use wasm_bindgen::JsValue;
use wasm_bindgen::closure::Closure;
use wasm_bindgen_futures::{JsFuture, future_to_promise};

const FALLBACK_LOCK_PREFIX: &str = "papercraft_lock_";

fn get_lock_manager_and_request() -> Option<(JsValue, Function)> {
    let window = web_sys::window()?;
    let navigator = window.navigator();
    let locks = Reflect::get(&navigator, &JsValue::from_str("locks")).ok()?;
    if locks.is_null() || locks.is_undefined() {
        return None;
    }
    let req_val = Reflect::get(&locks, &JsValue::from_str("request")).ok()?;
    let req_fn = req_val.dyn_into::<Function>().ok()?;
    Some((locks, req_fn))
}

fn sanitize_lock_storage_key(name: &str) -> String {
    let mut out = String::from(FALLBACK_LOCK_PREFIX);
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' || ch == ':' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    out
}

fn try_acquire_storage_lease(key: &str, owner: &str, lease_ms: f64) -> bool {
    let Some(window) = web_sys::window() else {
        return true;
    };
    let Ok(Some(storage)) = window.local_storage() else {
        return true;
    };
    let now = Date::now();
    if let Ok(Some(existing)) = storage.get_item(key)
        && let Some((holder, exp_raw)) = existing.split_once(':')
        && holder != owner
        && let Ok(expires_at) = exp_raw.parse::<f64>()
        && expires_at.is_finite()
        && expires_at > now
    {
        return false;
    }
    let expires_at = now + lease_ms.clamp(5_000.0, 120_000.0);
    let token = format!("{owner}:{expires_at:.0}");
    let _ = storage.set_item(key, &token);
    storage
        .get_item(key)
        .ok()
        .flatten()
        .is_some_and(|val| val.starts_with(&format!("{owner}:")))
}

fn release_storage_lease(key: &str, owner: &str) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let Ok(Some(storage)) = window.local_storage() else {
        return;
    };
    if let Ok(Some(existing)) = storage.get_item(key)
        && existing.starts_with(&format!("{owner}:"))
    {
        let _ = storage.remove_item(key);
    }
}

/// Acquires an exclusive Web Lock named `name` (waiting until any active holder completes)
/// and executes `f`. Falls back to executing `f` directly when `navigator.locks` is unavailable.
pub async fn with_exclusive_lock<F, Fut, T>(name: &str, f: F) -> Option<T>
where
    F: FnOnce() -> Fut + 'static,
    Fut: Future<Output = T> + 'static,
    T: 'static,
{
    let f_slot: Rc<RefCell<Option<F>>> = Rc::new(RefCell::new(Some(f)));
    let res_slot: Rc<RefCell<Option<T>>> = Rc::new(RefCell::new(None));

    if let Some((locks_obj, request_fn)) = get_lock_manager_and_request() {
        let f_cb = Rc::clone(&f_slot);
        let res_cb = Rc::clone(&res_slot);
        let cb = Closure::once(move |_lock: JsValue| -> JsValue {
            if let Some(work) = f_cb.borrow_mut().take() {
                let promise = future_to_promise(async move {
                    let value = work().await;
                    *res_cb.borrow_mut() = Some(value);
                    Ok(JsValue::TRUE)
                });
                JsValue::from(promise)
            } else {
                JsValue::from(Promise::resolve(&JsValue::FALSE))
            }
        });
        let args = Array::of2(&JsValue::from_str(name), cb.as_ref());
        if let Ok(outer_val) = Reflect::apply(&request_fn, &locks_obj, &args)
            && let Ok(outer_promise) = outer_val.dyn_into::<Promise>()
        {
            let _ = JsFuture::from(outer_promise).await;
        }
        drop(cb);
    }

    if let Some(value) = res_slot.borrow_mut().take() {
        return Some(value);
    }
    let fallback_fn = f_slot.borrow_mut().take()?;
    Some(fallback_fn().await)
}

/// Attempts to acquire an exclusive Web Lock non-blockingly (`ifAvailable: true`). Returns `None`
/// if another tab currently holds `name`, or falls back to a `localStorage` lease when Web Locks
/// are unavailable.
pub async fn try_with_exclusive_lock_if_available<F, Fut, T>(
    name: &str,
    tab_id: &str,
    lease_ms: f64,
    f: F,
) -> Option<T>
where
    F: FnOnce() -> Fut + 'static,
    Fut: Future<Output = T> + 'static,
    T: 'static,
{
    let f_slot: Rc<RefCell<Option<F>>> = Rc::new(RefCell::new(Some(f)));
    let res_slot: Rc<RefCell<Option<T>>> = Rc::new(RefCell::new(None));
    let held_by_peer = Rc::new(Cell::new(false));

    if let Some((locks_obj, request_fn)) = get_lock_manager_and_request() {
        let opts = Object::new();
        let _ = Reflect::set(
            &opts,
            &JsValue::from_str("mode"),
            &JsValue::from_str("exclusive"),
        );
        let _ = Reflect::set(&opts, &JsValue::from_str("ifAvailable"), &JsValue::TRUE);

        let f_cb = Rc::clone(&f_slot);
        let res_cb = Rc::clone(&res_slot);
        let busy_cb = Rc::clone(&held_by_peer);
        let cb = Closure::once(move |lock: JsValue| -> JsValue {
            if lock.is_null() || lock.is_undefined() {
                busy_cb.set(true);
                return JsValue::from(Promise::resolve(&JsValue::FALSE));
            }
            if let Some(work) = f_cb.borrow_mut().take() {
                let promise = future_to_promise(async move {
                    let value = work().await;
                    *res_cb.borrow_mut() = Some(value);
                    Ok(JsValue::TRUE)
                });
                JsValue::from(promise)
            } else {
                JsValue::from(Promise::resolve(&JsValue::FALSE))
            }
        });
        let args = Array::of3(&JsValue::from_str(name), &opts.into(), cb.as_ref());
        if let Ok(outer_val) = Reflect::apply(&request_fn, &locks_obj, &args)
            && let Ok(outer_promise) = outer_val.dyn_into::<Promise>()
        {
            let _ = JsFuture::from(outer_promise).await;
        }
        drop(cb);

        if held_by_peer.get() {
            return None;
        }
        if let Some(value) = res_slot.borrow_mut().take() {
            return Some(value);
        }
    }

    let fallback_fn = f_slot.borrow_mut().take()?;
    let storage_key = sanitize_lock_storage_key(name);
    if !try_acquire_storage_lease(&storage_key, tab_id, lease_ms) {
        return None;
    }
    let value = fallback_fn().await;
    release_storage_lease(&storage_key, tab_id);
    Some(value)
}
