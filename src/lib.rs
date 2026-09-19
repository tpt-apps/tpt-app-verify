// TPT Verify — free in-browser edition.
//
// Hub contract (web repo `WasmAppRunner`):
//   1. the runner dynamic-imports the wasm-bindgen `--target web` glue,
//   2. awaits its default export (wasm init),
//   3. calls the named export `mount(container)` with the hub's container div.
//
// Standalone dev (trunk serve) uses the same `mount_app` via the
// `#tpt-appfront-root` marker div that only index.html provides, so the two
// hosts never double-mount.

mod app;

use wasm_bindgen::prelude::*;

/// Hub entry point: mounted into the /tools/verify page's container.
#[wasm_bindgen]
pub fn mount(container: web_sys::Element) -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    app::mount_app(&container)
}

/// Standalone entry (trunk serve): index.html provides `#tpt-appfront-root`;
/// the hub page does not, so this is a no-op when embedded.
#[wasm_bindgen(start)]
pub fn start() -> Result<(), JsValue> {
    console_error_panic_hook::set_once();
    let window = web_sys::window().expect("no window");
    let document = window.document().expect("no document");
    if let Some(root) = document.get_element_by_id("tpt-appfront-root") {
        app::mount_app(&root)?;
    }
    Ok(())
}
