//! Shared browser-only support. The Linux server crate remains independent.
pub mod api;
pub mod components;
pub mod contracts;
pub mod i18n;
pub mod server_corners;
pub mod theme;

extern crate self as xcss_web;

pub use wasm_bindgen;
pub use wasm_bindgen_futures;
pub use web_sys;
pub use xcss_web_core::*;

use wasm_bindgen::{JsCast, JsValue};

/// Mount functional UI independently of optional application font loading.
pub fn mount<F, V>(root_id: &str, app: F)
where
    F: FnOnce() -> V + 'static,
    V: IntoView + 'static,
{
    mount_application(root_id, app, false);
}

/// Mount a Server view with the shared Rust corner controller.
pub fn mount_server<F, V>(root_id: &str, app: F)
where
    F: FnOnce() -> V + 'static,
    V: IntoView + 'static,
{
    mount_application(root_id, app, true);
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    // Keep the controller alive for this document; its pagehide listener releases
    // observers, styles and URLs before an uncached page leaves.
    static SERVER_CORNERS: std::cell::RefCell<Option<server_corners::ServerCornersController>> = const { std::cell::RefCell::new(None) };
}

fn mount_application<F, V>(root_id: &str, app: F, server_corners: bool)
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
            let document = web_sys::window()
                .and_then(|window| window.document())
                .ok_or_else(|| JsValue::from_str("Browser document is unavailable"))?;
            let root = document
                .get_element_by_id(&root_id)
                .ok_or_else(|| JsValue::from_str("Application root is missing"))?
                .dyn_into::<web_sys::HtmlElement>()?;
            let html = document
                .document_element()
                .ok_or_else(|| JsValue::from_str("Browser document root is missing"))?;
            // A timed-out startup cannot later replace its visible failure page.
            if html.get_attribute("data-xcss-startup").as_deref() == Some("failed") {
                return Err(JsValue::from_str("Application startup has already failed"));
            }
            #[cfg(target_arch = "wasm32")]
            let fonts = optional_fonts::prepare(&document)?;
            xcss_web::mount::mount_to(root, app).forget();
            html.set_attribute("data-xcss-appearance", "content-blocks")?;
            html.set_attribute("data-xcss-selection", "underline")?;
            #[cfg(target_arch = "wasm32")]
            if server_corners {
                let controller = crate::server_corners::start(&document)?;
                SERVER_CORNERS.with(|value| *value.borrow_mut() = Some(controller));
            }
            #[cfg(not(target_arch = "wasm32"))]
            let _ = server_corners;
            html.set_attribute("data-xcss-runtime", "xcss-web")?;
            document.dispatch_event(&web_sys::Event::new("xcss-startup-ready")?)?;
            #[cfg(target_arch = "wasm32")]
            fonts.start();
            Ok(())
        }
        .await;
        if let Err(error) = result {
            web_sys::console::error_2(&JsValue::from_str("Application startup failed"), &error);
            if let Some(document) = web_sys::window().and_then(|window| window.document())
                && let Ok(event) = web_sys::Event::new("xcss-startup-failed")
            {
                let _ = document.dispatch_event(&event);
            }
        }
    });
}

#[cfg(target_arch = "wasm32")]
mod optional_fonts {
    use futures::{
        channel::oneshot,
        future::{Either, select},
    };
    use gloo_events::EventListener;
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };
    use wasm_bindgen::{JsCast, JsValue};

    pub(super) struct Loading {
        document: web_sys::Document,
        active: Rc<Cell<bool>>,
        cancelled: oneshot::Receiver<()>,
        _listeners: (EventListener, EventListener),
    }

    // Install cancellation before publishing readiness: a synchronous ready
    // listener can leave the document or reject startup before fonts begin.
    pub(super) fn prepare(document: &web_sys::Document) -> Result<Loading, JsValue> {
        let window = document
            .default_view()
            .ok_or_else(|| JsValue::from_str("Browser window is unavailable"))?;
        let html = document
            .document_element()
            .ok_or_else(|| JsValue::from_str("Browser document root is missing"))?;
        html.set_attribute("data-xcss-fonts", "pending")?;
        let active = Rc::new(Cell::new(true));
        let (cancel, cancelled) = oneshot::channel();
        let cancel = Rc::new(RefCell::new(Some(cancel)));
        let failed_active = active.clone();
        let failed_cancel = cancel.clone();
        let failed = EventListener::new(document, "xcss-startup-failed", move |_| {
            failed_active.set(false);
            if let Some(sender) = failed_cancel.borrow_mut().take() {
                let _ = sender.send(());
            }
        });
        let page_active = active.clone();
        let pagehide = EventListener::new(&window, "pagehide", move |event| {
            if event
                .dyn_ref::<web_sys::PageTransitionEvent>()
                .is_some_and(|event| event.persisted())
            {
                return;
            }
            page_active.set(false);
            if let Some(sender) = cancel.borrow_mut().take() {
                let _ = sender.send(());
            }
        });
        Ok(Loading {
            document: document.clone(),
            active,
            cancelled,
            _listeners: (pagehide, failed),
        })
    }

    async fn load(document: &web_sys::Document) -> Result<(), JsValue> {
        let fonts = document.fonts();
        let (regular, bold) = futures::try_join!(
            wasm_bindgen_futures::JsFuture::from(fonts.load("16px \"Sarmg Maple\"")),
            wasm_bindgen_futures::JsFuture::from(fonts.load("bold 16px \"Sarmg Maple\""))
        )?;
        if js_sys::Array::from(&regular).length() == 0 || js_sys::Array::from(&bold).length() == 0 {
            return Err(JsValue::from_str("Application font styles are missing"));
        }
        wasm_bindgen_futures::JsFuture::from(fonts.ready()?).await?;
        Ok(())
    }

    impl Loading {
        pub(super) fn start(self) {
            wasm_bindgen_futures::spawn_local(async move {
                let Self {
                    document,
                    active,
                    cancelled,
                    _listeners,
                } = self;
                if !active.get() {
                    return;
                }
                let result = match select(Box::pin(load(&document)), cancelled).await {
                    Either::Left((result, _)) => result,
                    Either::Right(_) => return,
                };
                let Some(html) = document.document_element() else {
                    return;
                };
                if !active.get()
                    || html.get_attribute("data-xcss-startup").as_deref() == Some("failed")
                {
                    return;
                }
                let state = if result.is_ok() { "ready" } else { "failed" };
                if let Err(error) = result {
                    web_sys::console::warn_2(
                        &JsValue::from_str(
                            "Optional application fonts unavailable; using fallback fonts",
                        ),
                        &error,
                    );
                }
                let _ = html.set_attribute("data-xcss-fonts", state);
                // Font metrics can change geometry after the first functional
                // render; schedule the existing observer/controller path.
                super::SERVER_CORNERS.with(|controller| {
                    if let Some(controller) = controller.borrow().as_ref() {
                        controller.refresh();
                    }
                });
                drop(_listeners);
            });
        }
    }
}
