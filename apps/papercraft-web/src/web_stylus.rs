//! W3C Pointer Events and Touch Events bridge for hardware stylus pressure, tilt, twist,
//! barrel buttons, inverted eraser tip, proximity, and palm-rejection classification in WebAssembly.

use std::sync::{Arc, Mutex, PoisonError};

use papercraft_ui_egui::stylus::{PenContactEvent, PenContactKind, PenSample, PointerDeviceKind};
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

use crate::web_files::SharedEguiCtx;

const MAX_QUEUED_SAMPLES: usize = 2048;
const MAX_QUEUED_CONTACTS: usize = 512;
const MAX_TRACKED_TOUCH_POINTERS: usize = 32;
const MIN_WEB_CONTACT_PRESSURE: f32 = 0.02;
const CONTEXTMENU_SUPPRESS_GRACE_MS: f64 = 750.0;
const STALE_PEN_HOVER_TIMEOUT_MS: f64 = 1500.0;
const PEN_TOUCH_MATCH_RADIUS_PX: f64 = 28.0;
const PEN_TOUCH_MATCH_WINDOW_MS: f64 = 250.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PointerPhase {
    Enter,
    Down,
    Move,
    Up,
    Cancel,
    Leave,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TouchPhase {
    Start,
    Move,
    End,
    Cancel,
}

#[derive(Default)]
struct WebStylusState {
    samples: Vec<PenSample>,
    contact_events: Vec<PenContactEvent>,
    pen_active: bool,
    pen_down: bool,
    active_pen_pointer_id: Option<i32>,
    active_pen_touch_id: Option<u64>,
    last_pen_client_pos: Option<(f64, f64)>,
    last_pen_event_ms: Option<f64>,
    cached_canvas_origin: Option<(f64, f64)>,
    active_touch_pointer_ids: Vec<i32>,
    pointer_device_kind: PointerDeviceKind,
    pen_end_eraser: Option<bool>,
    barrel_down: bool,
    secondary_barrel_down: bool,
    barrel_press_count: u32,
    secondary_barrel_press_count: u32,
    tracked_contact_pointer_id: Option<i32>,
    last_pen_barrel_time_ms: Option<f64>,
}

fn passive_capture_options() -> web_sys::AddEventListenerOptions {
    let opts = web_sys::AddEventListenerOptions::new();
    opts.set_capture(true);
    opts.set_passive(true);
    opts
}

/// Shared handle to browser `PointerEvent` and `TouchEvent` stylus state consumed by `PlatformHooks`.
#[derive(Clone, Default)]
pub struct WebStylusHandle {
    state: Arc<Mutex<WebStylusState>>,
}

impl WebStylusHandle {
    /// Installs capture-phase `pointer*`/`touch*`, `blur`, and `contextmenu` DOM listeners.
    pub fn install(canvas: &web_sys::HtmlCanvasElement, ctx_slot: SharedEguiCtx) -> Self {
        let handle = Self::default();
        for (name, phase) in [
            ("pointerenter", PointerPhase::Enter),
            ("pointerdown", PointerPhase::Down),
            ("pointermove", PointerPhase::Move),
            ("pointerup", PointerPhase::Up),
            ("pointercancel", PointerPhase::Cancel),
            ("pointerleave", PointerPhase::Leave),
        ] {
            handle.attach_pointer_listener(canvas, name, phase, &ctx_slot);
        }
        for (name, phase) in [
            ("touchstart", TouchPhase::Start),
            ("touchmove", TouchPhase::Move),
            ("touchend", TouchPhase::End),
            ("touchcancel", TouchPhase::Cancel),
        ] {
            handle.attach_touch_listener(canvas, name, phase, &ctx_slot);
        }
        handle.attach_contextmenu_listener(canvas);

        if let Some(window) = web_sys::window() {
            handle.attach_window_release_listener(
                &window,
                canvas,
                "pointerup",
                PointerPhase::Up,
                &ctx_slot,
            );
            handle.attach_window_release_listener(
                &window,
                canvas,
                "pointercancel",
                PointerPhase::Cancel,
                &ctx_slot,
            );
            handle.attach_window_touch_listener(&window, "touchend", TouchPhase::End, &ctx_slot);
            handle.attach_window_touch_listener(
                &window,
                "touchcancel",
                TouchPhase::Cancel,
                &ctx_slot,
            );
            handle.attach_window_blur_listener(&window);
            handle.attach_window_viewport_listener(&window, "resize");
            handle.attach_window_viewport_listener(&window, "scroll");
        }
        handle
    }

    pub fn drain_samples(&self) -> Vec<PenSample> {
        let mut g = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut g.samples)
    }

    pub fn drain_contact_events(&self) -> Vec<PenContactEvent> {
        let mut g = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut g.contact_events)
    }

    pub fn pen_pointer_id(&self) -> Option<u64> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .active_pen_touch_id
    }

    pub fn pen_in_proximity(&self) -> bool {
        let g = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        g.pen_active || g.pen_down || g.active_pen_touch_id.is_some()
    }

    pub fn pointer_device_kind(&self) -> PointerDeviceKind {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pointer_device_kind
    }

    pub fn pen_end(&self) -> Option<bool> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pen_end_eraser
    }

    pub fn barrel_down(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .barrel_down
    }

    pub fn secondary_barrel_down(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .secondary_barrel_down
    }

    pub fn take_barrel_press_count(&self) -> u32 {
        let mut g = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut g.barrel_press_count)
    }

    pub fn take_secondary_barrel_press_count(&self) -> u32 {
        let mut g = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        std::mem::take(&mut g.secondary_barrel_press_count)
    }

    fn attach_pointer_listener(
        &self,
        canvas: &web_sys::HtmlCanvasElement,
        event_name: &str,
        phase: PointerPhase,
        ctx_slot: &SharedEguiCtx,
    ) {
        let state = Arc::clone(&self.state);
        let canvas_ref = canvas.clone();
        let ctx_ref = Arc::clone(ctx_slot);
        let cb = Closure::<dyn FnMut(web_sys::PointerEvent)>::wrap(Box::new(
            move |ev: web_sys::PointerEvent| {
                if handle_pointer_event(&state, &canvas_ref, &ev, phase) {
                    request_repaint(&ctx_ref);
                }
            },
        ));
        let _ = canvas.add_event_listener_with_callback_and_add_event_listener_options(
            event_name,
            cb.as_ref().unchecked_ref(),
            &passive_capture_options(),
        );
        cb.forget();
    }

    fn attach_touch_listener(
        &self,
        canvas: &web_sys::HtmlCanvasElement,
        event_name: &str,
        phase: TouchPhase,
        ctx_slot: &SharedEguiCtx,
    ) {
        let state = Arc::clone(&self.state);
        let ctx_ref = Arc::clone(ctx_slot);
        let cb = Closure::<dyn FnMut(web_sys::TouchEvent)>::wrap(Box::new(
            move |ev: web_sys::TouchEvent| {
                if handle_touch_event(&state, &ev, phase) {
                    request_repaint(&ctx_ref);
                }
            },
        ));
        let _ = canvas.add_event_listener_with_callback_and_add_event_listener_options(
            event_name,
            cb.as_ref().unchecked_ref(),
            &passive_capture_options(),
        );
        cb.forget();
    }

    fn attach_window_release_listener(
        &self,
        window: &web_sys::Window,
        canvas: &web_sys::HtmlCanvasElement,
        event_name: &str,
        phase: PointerPhase,
        ctx_slot: &SharedEguiCtx,
    ) {
        let state = Arc::clone(&self.state);
        let canvas_ref = canvas.clone();
        let ctx_ref = Arc::clone(ctx_slot);
        let cb = Closure::<dyn FnMut(web_sys::PointerEvent)>::wrap(Box::new(
            move |ev: web_sys::PointerEvent| {
                if handle_window_release(&state, &canvas_ref, &ev, phase) {
                    request_repaint(&ctx_ref);
                }
            },
        ));
        let _ = window.add_event_listener_with_callback_and_add_event_listener_options(
            event_name,
            cb.as_ref().unchecked_ref(),
            &passive_capture_options(),
        );
        cb.forget();
    }

    fn attach_window_touch_listener(
        &self,
        window: &web_sys::Window,
        event_name: &str,
        phase: TouchPhase,
        ctx_slot: &SharedEguiCtx,
    ) {
        let state = Arc::clone(&self.state);
        let ctx_ref = Arc::clone(ctx_slot);
        let cb = Closure::<dyn FnMut(web_sys::TouchEvent)>::wrap(Box::new(
            move |ev: web_sys::TouchEvent| {
                if handle_touch_event(&state, &ev, phase) {
                    request_repaint(&ctx_ref);
                }
            },
        ));
        let _ = window.add_event_listener_with_callback_and_add_event_listener_options(
            event_name,
            cb.as_ref().unchecked_ref(),
            &passive_capture_options(),
        );
        cb.forget();
    }

    fn attach_window_blur_listener(&self, window: &web_sys::Window) {
        let state = Arc::clone(&self.state);
        let cb = Closure::<dyn FnMut(web_sys::Event)>::wrap(Box::new(move |_: web_sys::Event| {
            let mut g = state.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = clear_pen_state(&mut g);
            g.active_touch_pointer_ids.clear();
            g.pointer_device_kind = PointerDeviceKind::Unknown;
            g.cached_canvas_origin = None;
        }));
        let _ = window.add_event_listener_with_callback_and_add_event_listener_options(
            "blur",
            cb.as_ref().unchecked_ref(),
            &passive_capture_options(),
        );
        cb.forget();
    }

    fn attach_window_viewport_listener(&self, window: &web_sys::Window, event_name: &str) {
        let state = Arc::clone(&self.state);
        let cb = Closure::<dyn FnMut(web_sys::Event)>::wrap(Box::new(move |_: web_sys::Event| {
            let mut g = state.lock().unwrap_or_else(PoisonError::into_inner);
            g.cached_canvas_origin = None;
        }));
        let _ = window.add_event_listener_with_callback_and_add_event_listener_options(
            event_name,
            cb.as_ref().unchecked_ref(),
            &passive_capture_options(),
        );
        cb.forget();
    }

    fn attach_contextmenu_listener(&self, canvas: &web_sys::HtmlCanvasElement) {
        let state = Arc::clone(&self.state);
        let cb = Closure::<dyn FnMut(web_sys::MouseEvent)>::wrap(Box::new(
            move |ev: web_sys::MouseEvent| {
                let now_ms = ev.time_stamp();
                let g = state.lock().unwrap_or_else(PoisonError::into_inner);
                let recent_barrel = g.last_pen_barrel_time_ms.is_some_and(|t| {
                    now_ms.is_finite()
                        && t.is_finite()
                        && now_ms >= t
                        && (now_ms - t) <= CONTEXTMENU_SUPPRESS_GRACE_MS
                });
                if g.pen_active
                    || g.barrel_down
                    || g.secondary_barrel_down
                    || g.pen_end_eraser == Some(true)
                    || recent_barrel
                {
                    ev.prevent_default();
                }
            },
        ));
        let _ = canvas.add_event_listener_with_callback("contextmenu", cb.as_ref().unchecked_ref());
        cb.forget();
    }
}

fn request_repaint(ctx_slot: &SharedEguiCtx) {
    if let Ok(guard) = ctx_slot.lock()
        && let Some(egui_ctx) = guard.as_ref()
    {
        egui_ctx.request_repaint();
    }
}

fn refresh_canvas_origin(
    g: &mut WebStylusState,
    canvas: &web_sys::HtmlCanvasElement,
) -> (f64, f64) {
    let rect = canvas.get_bounding_client_rect();
    let origin = (rect.left(), rect.top());
    g.cached_canvas_origin = Some(origin);
    origin
}

fn canvas_local_pos(
    g: &mut WebStylusState,
    canvas: &web_sys::HtmlCanvasElement,
    ev: &web_sys::PointerEvent,
    refresh_rect: bool,
) -> egui::Pos2 {
    let (left, top) = if refresh_rect || g.cached_canvas_origin.is_none() {
        refresh_canvas_origin(g, canvas)
    } else {
        g.cached_canvas_origin.unwrap_or((0.0, 0.0))
    };
    let x = f64::from(ev.client_x()) - left;
    let y = f64::from(ev.client_y()) - top;
    let x_f32 = if x.is_finite() { x as f32 } else { 0.0 };
    let y_f32 = if y.is_finite() { y as f32 } else { 0.0 };
    egui::pos2(x_f32, y_f32)
}

fn event_modifiers(ev: &web_sys::PointerEvent) -> egui::Modifiers {
    egui::Modifiers {
        alt: ev.alt_key(),
        ctrl: ev.ctrl_key(),
        shift: ev.shift_key(),
        mac_cmd: ev.meta_key(),
        command: ev.ctrl_key() || ev.meta_key(),
    }
}

fn decode_pen_flags(phase: PointerPhase, button: i16, buttons: u16) -> (bool, bool, bool) {
    let is_down = phase == PointerPhase::Down;
    let is_release = matches!(
        phase,
        PointerPhase::Up | PointerPhase::Cancel | PointerPhase::Leave
    );
    let barrel =
        ((buttons & 2) != 0 || (is_down && button == 2)) && !(is_release && (buttons & 2) == 0);
    let secondary =
        ((buttons & 4) != 0 || (is_down && button == 1)) && !(is_release && (buttons & 4) == 0);
    let eraser =
        ((buttons & 32) != 0 || (is_down && button == 5)) && !(is_release && (buttons & 32) == 0);
    (barrel, secondary, eraser)
}

fn sample_pressure(phase: PointerPhase, buttons: u16, raw_pressure: f32) -> f32 {
    let clean = if raw_pressure.is_finite() {
        raw_pressure.clamp(0.0, 1.0)
    } else {
        0.0
    };
    match phase {
        PointerPhase::Down => clean.max(MIN_WEB_CONTACT_PRESSURE),
        PointerPhase::Move if (buttons & 1) != 0 => clean.max(MIN_WEB_CONTACT_PRESSURE),
        PointerPhase::Up | PointerPhase::Cancel | PointerPhase::Leave => 0.0,
        PointerPhase::Enter | PointerPhase::Move => clean,
    }
}

fn push_bounded_sample(samples: &mut Vec<PenSample>, sample: PenSample) {
    if samples.len() >= MAX_QUEUED_SAMPLES {
        samples.remove(0);
    }
    samples.push(sample.sanitized());
}

fn push_bounded_contact(contacts: &mut Vec<PenContactEvent>, event: PenContactEvent) {
    if contacts.len() >= MAX_QUEUED_CONTACTS {
        contacts.remove(0);
    }
    contacts.push(event);
}

fn clear_pen_state(state: &mut WebStylusState) -> bool {
    let changed = state.pen_active
        || state.pen_down
        || state.active_pen_pointer_id.is_some()
        || state.active_pen_touch_id.is_some()
        || state.barrel_down
        || state.secondary_barrel_down
        || state.pen_end_eraser.is_some();
    state.pen_active = false;
    state.pen_down = false;
    state.active_pen_pointer_id = None;
    state.active_pen_touch_id = None;
    state.last_pen_client_pos = None;
    state.last_pen_event_ms = None;
    state.pen_end_eraser = None;
    state.barrel_down = false;
    state.secondary_barrel_down = false;
    state.tracked_contact_pointer_id = None;
    changed
}

fn expire_stale_pen_hover(g: &mut WebStylusState, now_ms: f64) -> bool {
    if g.pen_down || g.active_pen_touch_id.is_some() {
        return false;
    }
    let stale = g.last_pen_event_ms.is_none_or(|t| {
        !now_ms.is_finite()
            || !t.is_finite()
            || now_ms < t
            || (now_ms - t) > STALE_PEN_HOVER_TIMEOUT_MS
    });
    if stale { clear_pen_state(g) } else { false }
}

fn track_touch_pointer(ids: &mut Vec<i32>, pointer_id: i32, phase: PointerPhase) {
    match phase {
        PointerPhase::Down | PointerPhase::Move => {
            if !ids.contains(&pointer_id) {
                if ids.len() >= MAX_TRACKED_TOUCH_POINTERS {
                    ids.remove(0);
                }
                ids.push(pointer_id);
            }
        }
        PointerPhase::Up | PointerPhase::Cancel | PointerPhase::Leave => {
            ids.retain(|id| *id != pointer_id);
        }
        PointerPhase::Enter => {}
    }
}

fn coalesced_pointer_events(ev: &web_sys::PointerEvent) -> Vec<web_sys::PointerEvent> {
    let Ok(val) = js_sys::Reflect::get(
        ev.as_ref(),
        &wasm_bindgen::JsValue::from_str("getCoalescedEvents"),
    ) else {
        return Vec::new();
    };
    let Ok(func) = val.dyn_into::<js_sys::Function>() else {
        return Vec::new();
    };
    let Ok(arr_val) = func.call0(ev.as_ref()) else {
        return Vec::new();
    };
    let arr = js_sys::Array::from(&arr_val);
    let count = arr.length().min(64);
    let mut out = Vec::with_capacity(count as usize);
    for idx in 0..count {
        if let Ok(sub_ev) = arr.get(idx).dyn_into::<web_sys::PointerEvent>() {
            out.push(sub_ev);
        }
    }
    out
}

fn push_pen_sample_from_event(
    g: &mut WebStylusState,
    ev: &web_sys::PointerEvent,
    phase: PointerPhase,
    flags: (bool, bool, bool),
) {
    let (barrel, secondary, eraser) = flags;
    let sample = PenSample {
        pressure: sample_pressure(phase, ev.buttons(), ev.pressure()),
        tilt_x: ev.tilt_x() as f32,
        tilt_y: ev.tilt_y() as f32,
        rotation: ev.twist() as f32,
        eraser,
        barrel,
        barrel_secondary: secondary,
        time_ms: Some(ev.time_stamp()),
    };
    push_bounded_sample(&mut g.samples, sample);
}

fn touch_type_string(touch: &web_sys::Touch) -> Option<String> {
    js_sys::Reflect::get(
        touch.as_ref(),
        &wasm_bindgen::JsValue::from_str("touchType"),
    )
    .ok()
    .and_then(|v| v.as_string())
}

fn touch_id_u64(touch: &web_sys::Touch) -> u64 {
    egui::TouchId::from(touch.identifier()).0
}

fn touch_matches_active_pen(g: &WebStylusState, touch: &web_sys::Touch, now_ms: f64) -> bool {
    let recent_pen = g.last_pen_event_ms.is_some_and(|t| {
        now_ms.is_finite()
            && t.is_finite()
            && now_ms >= t
            && (now_ms - t) <= PEN_TOUCH_MATCH_WINDOW_MS
    });
    if !g.pen_down && !recent_pen {
        return false;
    }
    if let Some((px, py)) = g.last_pen_client_pos {
        let dx = f64::from(touch.client_x()) - px;
        let dy = f64::from(touch.client_y()) - py;
        if dx.hypot(dy) <= PEN_TOUCH_MATCH_RADIUS_PX {
            return true;
        }
    }
    g.pen_down && g.active_pen_touch_id.is_none() && g.active_touch_pointer_ids.is_empty()
}

fn apply_touch_contact_to_state(
    g: &mut WebStylusState,
    touch: &web_sys::Touch,
    phase: TouchPhase,
    now_ms: f64,
) -> bool {
    let tid = touch_id_u64(touch);
    match phase {
        TouchPhase::End | TouchPhase::Cancel => {
            if g.active_pen_touch_id == Some(tid) {
                g.active_pen_touch_id = None;
                g.pen_down = false;
                return true;
            }
            false
        }
        TouchPhase::Start | TouchPhase::Move => {
            let touch_type = touch_type_string(touch);
            let is_stylus = match touch_type.as_deref() {
                Some("stylus") => true,
                Some("direct") => false,
                _ => {
                    g.active_pen_touch_id == Some(tid) || touch_matches_active_pen(g, touch, now_ms)
                }
            };
            if is_stylus {
                let changed = g.active_pen_touch_id != Some(tid) || !g.pen_active;
                g.active_pen_touch_id = Some(tid);
                g.pen_active = true;
                g.pen_down = true;
                g.pointer_device_kind = PointerDeviceKind::Pen;
                g.last_pen_client_pos =
                    Some((f64::from(touch.client_x()), f64::from(touch.client_y())));
                if now_ms.is_finite() {
                    g.last_pen_event_ms = Some(now_ms);
                }
                changed
            } else {
                let expired = expire_stale_pen_hover(g, now_ms);
                if !g.pen_down && g.active_pen_touch_id.is_none() {
                    g.pointer_device_kind = PointerDeviceKind::Touch;
                }
                expired
            }
        }
    }
}

fn handle_touch_event(
    state: &Arc<Mutex<WebStylusState>>,
    ev: &web_sys::TouchEvent,
    phase: TouchPhase,
) -> bool {
    let mut g = state.lock().unwrap_or_else(PoisonError::into_inner);
    let now_ms = ev.time_stamp();
    let changed_touches = ev.changed_touches();
    let mut needs_repaint = false;
    for idx in 0..changed_touches.length() {
        if let Some(touch) = changed_touches.item(idx) {
            needs_repaint |= apply_touch_contact_to_state(&mut g, &touch, phase, now_ms);
        }
    }
    needs_repaint
}

fn handle_non_pen_pointer(
    g: &mut WebStylusState,
    pointer_type: &str,
    ev: &web_sys::PointerEvent,
    phase: PointerPhase,
    now_ms: f64,
) -> bool {
    if pointer_type == "mouse" {
        g.active_touch_pointer_ids.clear();
        g.pointer_device_kind = PointerDeviceKind::Mouse;
        return clear_pen_state(g);
    }
    if pointer_type == "touch" {
        track_touch_pointer(&mut g.active_touch_pointer_ids, ev.pointer_id(), phase);
        let expired = if matches!(
            phase,
            PointerPhase::Enter | PointerPhase::Down | PointerPhase::Move
        ) {
            expire_stale_pen_hover(g, now_ms)
        } else {
            false
        };
        if !g.pen_down && g.active_pen_touch_id.is_none() {
            g.pointer_device_kind = PointerDeviceKind::Touch;
        }
        return expired;
    }
    false
}

fn handle_pointer_event(
    state: &Arc<Mutex<WebStylusState>>,
    canvas: &web_sys::HtmlCanvasElement,
    ev: &web_sys::PointerEvent,
    phase: PointerPhase,
) -> bool {
    let pointer_type = ev.pointer_type();
    let now_ms = ev.time_stamp();
    let mut g = state.lock().unwrap_or_else(PoisonError::into_inner);
    if pointer_type != "pen" {
        return handle_non_pen_pointer(&mut g, &pointer_type, ev, phase, now_ms);
    }

    if matches!(phase, PointerPhase::Cancel | PointerPhase::Leave) {
        let mut needs_repaint = clear_pen_state(&mut g);
        if phase == PointerPhase::Cancel {
            let pos = canvas_local_pos(&mut g, canvas, ev, false);
            push_bounded_contact(
                &mut g.contact_events,
                PenContactEvent {
                    kind: PenContactKind::Cancel,
                    pos,
                    modifiers: event_modifiers(ev),
                },
            );
            needs_repaint = true;
        }
        push_bounded_sample(
            &mut g.samples,
            PenSample {
                pressure: 0.0,
                time_ms: Some(now_ms),
                ..Default::default()
            },
        );
        return needs_repaint;
    }

    if matches!(phase, PointerPhase::Enter | PointerPhase::Down) {
        let _ = refresh_canvas_origin(&mut g, canvas);
    }

    let button = ev.button();
    let buttons = ev.buttons();
    let (barrel, secondary, eraser) = decode_pen_flags(phase, button, buttons);
    let prev_active = g.pen_active;
    let prev_barrel = g.barrel_down;
    let prev_secondary = g.secondary_barrel_down;
    let prev_eraser = g.pen_end_eraser;

    g.pen_active = true;
    g.active_pen_pointer_id = Some(ev.pointer_id());
    g.pointer_device_kind = PointerDeviceKind::Pen;
    g.last_pen_client_pos = Some((f64::from(ev.client_x()), f64::from(ev.client_y())));
    if now_ms.is_finite() {
        g.last_pen_event_ms = Some(now_ms);
    }
    match phase {
        PointerPhase::Down => g.pen_down = true,
        PointerPhase::Up => g.pen_down = false,
        PointerPhase::Move => g.pen_down = (buttons & 1) != 0 || (buttons & 32) != 0,
        PointerPhase::Enter | PointerPhase::Cancel | PointerPhase::Leave => {}
    }

    g.barrel_down = barrel;
    g.secondary_barrel_down = secondary;
    g.pen_end_eraser = Some(eraser);

    if barrel && !prev_barrel {
        g.barrel_press_count = g.barrel_press_count.saturating_add(1);
    }
    if secondary && !prev_secondary {
        g.secondary_barrel_press_count = g.secondary_barrel_press_count.saturating_add(1);
    }
    if (barrel || secondary || eraser) && now_ms.is_finite() {
        g.last_pen_barrel_time_ms = Some(now_ms);
    }

    let coalesced = if phase == PointerPhase::Move {
        coalesced_pointer_events(ev)
    } else {
        Vec::new()
    };
    if coalesced.is_empty() {
        push_pen_sample_from_event(&mut g, ev, phase, (barrel, secondary, eraser));
    } else {
        for sub_ev in &coalesced {
            let sub_flags = decode_pen_flags(phase, sub_ev.button(), sub_ev.buttons());
            push_pen_sample_from_event(&mut g, sub_ev, phase, sub_flags);
        }
    }

    let contact_queued = record_fallback_contact(&mut g, canvas, ev, phase, button, eraser);
    contact_queued
        || !prev_active
        || prev_barrel != barrel
        || prev_secondary != secondary
        || prev_eraser != Some(eraser)
}

fn record_fallback_contact(
    g: &mut WebStylusState,
    canvas: &web_sys::HtmlCanvasElement,
    ev: &web_sys::PointerEvent,
    phase: PointerPhase,
    button: i16,
    eraser: bool,
) -> bool {
    let pointer_id = ev.pointer_id();
    let is_special_contact = matches!(button, 1 | 2 | 5) || eraser;
    match phase {
        PointerPhase::Down if is_special_contact => {
            g.tracked_contact_pointer_id = Some(pointer_id);
            let pos = canvas_local_pos(g, canvas, ev, true);
            push_bounded_contact(
                &mut g.contact_events,
                PenContactEvent {
                    kind: PenContactKind::Down,
                    pos,
                    modifiers: event_modifiers(ev),
                },
            );
            true
        }
        PointerPhase::Move if g.tracked_contact_pointer_id == Some(pointer_id) => {
            let pos = canvas_local_pos(g, canvas, ev, false);
            push_bounded_contact(
                &mut g.contact_events,
                PenContactEvent {
                    kind: PenContactKind::Move,
                    pos,
                    modifiers: event_modifiers(ev),
                },
            );
            true
        }
        PointerPhase::Up if g.tracked_contact_pointer_id == Some(pointer_id) => {
            g.tracked_contact_pointer_id = None;
            let pos = canvas_local_pos(g, canvas, ev, false);
            push_bounded_contact(
                &mut g.contact_events,
                PenContactEvent {
                    kind: PenContactKind::Up,
                    pos,
                    modifiers: event_modifiers(ev),
                },
            );
            true
        }
        _ => false,
    }
}

fn handle_window_release(
    state: &Arc<Mutex<WebStylusState>>,
    canvas: &web_sys::HtmlCanvasElement,
    ev: &web_sys::PointerEvent,
    phase: PointerPhase,
) -> bool {
    let pointer_type = ev.pointer_type();
    let mut g = state.lock().unwrap_or_else(PoisonError::into_inner);
    if pointer_type == "touch" {
        track_touch_pointer(&mut g.active_touch_pointer_ids, ev.pointer_id(), phase);
        return false;
    }
    if pointer_type != "pen" {
        return false;
    }

    let pointer_id = ev.pointer_id();
    if phase == PointerPhase::Cancel {
        let mut needs_repaint = clear_pen_state(&mut g);
        if g.tracked_contact_pointer_id == Some(pointer_id) {
            g.tracked_contact_pointer_id = None;
            let pos = canvas_local_pos(&mut g, canvas, ev, false);
            push_bounded_contact(
                &mut g.contact_events,
                PenContactEvent {
                    kind: PenContactKind::Cancel,
                    pos,
                    modifiers: event_modifiers(ev),
                },
            );
            needs_repaint = true;
        }
        return needs_repaint;
    }

    g.pen_down = false;
    let had_contact = g.tracked_contact_pointer_id == Some(pointer_id);
    let had_buttons = g.barrel_down || g.secondary_barrel_down || g.pen_end_eraser == Some(true);
    if !had_contact && !had_buttons {
        return false;
    }
    let buttons = ev.buttons();
    g.barrel_down = (buttons & 2) != 0;
    g.secondary_barrel_down = (buttons & 4) != 0;
    g.pen_end_eraser = Some((buttons & 32) != 0);
    if had_contact {
        g.tracked_contact_pointer_id = None;
        let pos = canvas_local_pos(&mut g, canvas, ev, false);
        push_bounded_contact(
            &mut g.contact_events,
            PenContactEvent {
                kind: PenContactKind::Up,
                pos,
                modifiers: event_modifiers(ev),
            },
        );
    }
    push_bounded_sample(
        &mut g.samples,
        PenSample {
            pressure: 0.0,
            eraser: (buttons & 32) != 0,
            barrel: (buttons & 2) != 0,
            barrel_secondary: (buttons & 4) != 0,
            time_ms: Some(ev.time_stamp()),
            ..Default::default()
        },
    );
    true
}
