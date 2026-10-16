//! Shared browser-only support. The Linux server crate remains independent.
pub mod api;
pub mod components;
pub mod contracts;
pub mod i18n;
pub mod theme;

pub use leptos;
pub use wasm_bindgen;
pub use wasm_bindgen_futures;
pub use web_sys;

use leptos::prelude::*;
use wasm_bindgen::{JsCast, JsValue};

/// Mount only after fonts have decoded, preserving the existing startup contract.
pub fn mount<F, V>(root_id: &str, app: F)
where
    F: FnOnce() -> V + 'static,
    V: IntoView + 'static,
{
    console_error_panic_hook::set_once();
    i18n::initialize();
    theme::initialize();
    let root_id = root_id.to_owned();
    wasm_bindgen_futures::spawn_local(async move {
        let result: Result<(), JsValue> = async {
            let document = web_sys::window().unwrap().document().unwrap();
            let fonts = document.fonts();
            // Loading both weights up front also covers glyphs absent from the first view.
            let regular = fonts.load("16px \"Sarmg Maple\"");
            let bold = fonts.load("bold 16px \"Sarmg Maple\"");
            wasm_bindgen_futures::JsFuture::from(regular).await?;
            wasm_bindgen_futures::JsFuture::from(bold).await?;
            wasm_bindgen_futures::JsFuture::from(fonts.ready()?).await?;
            let root = document
                .get_element_by_id(&root_id)
                .ok_or_else(|| JsValue::from_str("Application root is missing"))?
                .dyn_into::<web_sys::HtmlElement>()?;
            if let Some(html) = document.document_element() {
                html.set_attribute("data-xcss-fonts", "ready")?;
                html.set_attribute("data-xcss-appearance", "content-blocks")?;
                html.set_attribute("data-xcss-selection", "underline")?;
                html.set_attribute("data-xcss-runtime", "leptos")?;
            }
            leptos::mount::mount_to(root, app).forget();
            Ok(())
        }
        .await;
        if let Err(error) = result {
            web_sys::console::error_2(&JsValue::from_str("Application startup failed"), &error);
        }
    });
}
