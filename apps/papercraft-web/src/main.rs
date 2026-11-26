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
fn main() {
    console_error_panic_hook::set_once();
    // Dedicated Web Workers import and initialize this same WASM module without a DOM window.
    if web_sys::window().is_none() {
        return;
    }
    wasm_bindgen_futures::spawn_local(async {
        let runner = eframe::WebRunner::new();
        let _ = papercraft_web::start_on_canvas(&runner, "the_canvas_id").await;
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!(
        "papercraft-web targets WebAssembly (wasm32-unknown-unknown).\n\
         To run the web app locally, execute:\n\
         \n\
         cargo xtask web\n\
         \n\
         or:\n\
         \n\
         cd apps/papercraft-web\n\
         trunk serve\n\
         \n\
         Then open http://127.0.0.1:8080 in your browser."
    );
}
