#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]

#[cfg(target_arch = "wasm32")]
mod web_files;
#[cfg(target_arch = "wasm32")]
mod web_pdf_worker;
#[cfg(target_arch = "wasm32")]
mod web_storage;
#[cfg(target_arch = "wasm32")]
mod web_stylus;
#[cfg(target_arch = "wasm32")]
mod web_sync;

#[cfg(target_arch = "wasm32")]
mod web {
    use std::sync::{Arc, Mutex};

    use papercraft_engine::Session;
    use papercraft_ui_egui::{AsyncFileEvent, PaperCraftApp, PlatformHooks};
    use wasm_bindgen::prelude::wasm_bindgen;
    use wasm_bindgen::{JsCast, JsValue};

    use super::{web_files, web_pdf_worker, web_storage, web_stylus, web_sync};

    /// Starts the PaperCraft web application on the specified HTML canvas and removes
    /// the loading overlay once initialization completes.
    pub async fn start_on_canvas(
        runner: &eframe::WebRunner,
        canvas_id: &str,
    ) -> Result<(), JsValue> {
        console_error_panic_hook::set_once();

        let document = web_sys::window()
            .and_then(|win| win.document())
            .ok_or_else(|| JsValue::from_str("failed to get window document"))?;

        let canvas = document
            .get_element_by_id(canvas_id)
            .ok_or_else(|| {
                JsValue::from_str(&format!("canvas element with id '{canvas_id}' not found"))
            })?
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .map_err(|_| JsValue::from_str("element is not an HtmlCanvasElement"))?;

        let stylus_canvas = canvas.clone();
        let ctx_slot: web_files::SharedEguiCtx = Arc::new(Mutex::new(None));
        let (storage_ctrl, loaded_state) =
            web_storage::WebStorageController::init_and_load(Arc::clone(&ctx_slot)).await;

        let web_options = eframe::WebOptions::default();
        runner
            .start(
                canvas,
                web_options,
                Box::new(move |cc| {
                    let session = Session::open_web_storage(loaded_state);
                    let file_events: web_files::SharedFileEvents = Arc::new(Mutex::new(Vec::new()));
                    if let Ok(mut guard) = ctx_slot.lock() {
                        *guard = Some(cc.egui_ctx.clone());
                    }

                    let stylus =
                        web_stylus::WebStylusHandle::install(&stylus_canvas, Arc::clone(&ctx_slot));
                    let stylus_poll = stylus.clone();
                    let stylus_pointer_id = stylus.clone();
                    let stylus_proximity = stylus.clone();
                    let stylus_device_kind = stylus.clone();
                    let stylus_end = stylus.clone();
                    let stylus_barrel = stylus.clone();
                    let stylus_secondary_barrel = stylus.clone();
                    let stylus_barrel_presses = stylus.clone();
                    let stylus_secondary_presses = stylus.clone();
                    let stylus_contacts = stylus;

                    let save_events = Arc::clone(&file_events);
                    let save_ctx = Arc::clone(&ctx_slot);
                    let open_events = Arc::clone(&file_events);
                    let open_ctx = Arc::clone(&ctx_slot);
                    let drop_events = Arc::clone(&file_events);
                    let drop_ctx = Arc::clone(&ctx_slot);
                    let image_events = Arc::clone(&file_events);
                    let image_ctx = Arc::clone(&ctx_slot);
                    let poll_events = Arc::clone(&file_events);
                    web_files::install_clipboard_image_paste_listener(
                        Arc::clone(&file_events),
                        Arc::clone(&ctx_slot),
                    );

                    let flush_ctrl = storage_ctrl.clone();
                    let poll_ctrl = storage_ctrl.clone();
                    let retry_ctrl = storage_ctrl.clone();
                    let writing_ctrl = storage_ctrl;

                    let hooks = PlatformHooks {
                        tablet_poll: Some(Box::new(move || stylus_poll.drain_samples())),
                        pen_pointer_id: Some(Box::new(move || stylus_pointer_id.pen_pointer_id())),
                        pen_in_proximity: Some(Box::new(move || {
                            stylus_proximity.pen_in_proximity()
                        })),
                        pointer_device_kind: Some(Box::new(move || {
                            stylus_device_kind.pointer_device_kind()
                        })),
                        pen_end: Some(Box::new(move || stylus_end.pen_end())),
                        barrel_down: Some(Box::new(move || stylus_barrel.barrel_down())),
                        secondary_barrel_down: Some(Box::new(move || {
                            stylus_secondary_barrel.secondary_barrel_down()
                        })),
                        barrel_press_count: Some(Box::new(move || {
                            stylus_barrel_presses.take_barrel_press_count()
                        })),
                        secondary_barrel_press_count: Some(Box::new(move || {
                            stylus_secondary_presses.take_secondary_barrel_press_count()
                        })),
                        pen_contact_events: Some(Box::new(move || {
                            stylus_contacts.drain_contact_events()
                        })),
                        save_bytes_dialog: Some(Box::new(move |kind, default_name, bytes| {
                            web_files::trigger_save_bytes(
                                kind,
                                default_name,
                                bytes,
                                Arc::clone(&save_events),
                                Arc::clone(&save_ctx),
                            );
                        })),
                        pick_open_file: Some(Box::new(move |kind| {
                            web_files::trigger_open_file(
                                kind,
                                Arc::clone(&open_events),
                                Arc::clone(&open_ctx),
                            );
                            None
                        })),
                        pick_image_file: Some(Box::new(move || {
                            web_files::trigger_open_image_file(
                                Arc::clone(&image_events),
                                Arc::clone(&image_ctx),
                            );
                            None
                        })),
                        poll_file_events: Some(Box::new(move || {
                            if let Ok(mut guard) = poll_events.lock() {
                                std::mem::take(&mut *guard)
                            } else {
                                Vec::<AsyncFileEvent>::new()
                            }
                        })),
                        open_dropped_file: Some(Box::new(move |file| {
                            web_files::handle_dropped_file(
                                file,
                                Arc::clone(&drop_events),
                                Arc::clone(&drop_ctx),
                            );
                        })),
                        flush_web_storage_delta: Some(Box::new(move |delta| {
                            flush_ctrl.enqueue_delta(delta);
                        })),
                        poll_web_storage_events: Some(Box::new(move || poll_ctrl.drain_events())),
                        retry_web_storage_write: Some(Box::new(move || {
                            retry_ctrl.retry_failed();
                        })),
                        notify_writing_active: Some(Box::new(move |active| {
                            writing_ctrl.set_writing_active(active);
                        })),
                        web_pdf_worker: web_pdf_worker::WebPdfWorkerBridge::try_spawn(Arc::clone(
                            &ctx_slot,
                        ))
                        .map(|bridge| bridge.into_hooks()),
                        web_sync_dispatch: Some(Box::new(|cmd, evt_tx, ctx| {
                            web_sync::dispatch_sync_command(cmd, evt_tx, ctx);
                        })),
                        ..PlatformHooks::default()
                    };
                    let mut app = PaperCraftApp::new(session, hooks);
                    app.init_wgpu(cc.wgpu_render_state.as_ref());
                    Ok(Box::new(app))
                }),
            )
            .await?;

        if let Some(loader) = document.get_element_by_id("loading") {
            loader.remove();
        }
        Ok(())
    }

    #[wasm_bindgen]
    pub struct WebHandle {
        runner: eframe::WebRunner,
    }

    #[wasm_bindgen]
    impl WebHandle {
        #[allow(clippy::new_without_default)]
        #[wasm_bindgen(constructor)]
        pub fn new() -> Self {
            Self {
                runner: eframe::WebRunner::new(),
            }
        }

        #[wasm_bindgen]
        pub async fn start(&self, canvas_id: &str) -> Result<(), JsValue> {
            start_on_canvas(&self.runner, canvas_id).await
        }
    }
}

#[cfg(target_arch = "wasm32")]
pub use web::{WebHandle, start_on_canvas};
#[cfg(target_arch = "wasm32")]
pub use web_pdf_worker::{papercraft_pdf_worker_register_doc, papercraft_pdf_worker_render_page};
