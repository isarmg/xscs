use serde_json::Value;
use wasm_bindgen::{JsCast, closure::Closure};
use xcss_web::api::{self, ApiError, RequestOptions};
use xcss_web::component::{component_props_builder, component_view};
pub use xcss_web::i18n::t;
use xcss_web::prelude::*;
pub fn text(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("—").to_string()
}
pub fn num(v: &Value, k: &str) -> f64 {
    v.get(k).and_then(Value::as_f64).unwrap_or(0.)
}
pub fn flag(v: &Value, k: &str) -> bool {
    v.get(k).and_then(Value::as_bool).unwrap_or(false)
}
pub fn rows(v: &Value, k: &str) -> Vec<Value> {
    v.get(k)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}
pub fn enc(v: &str) -> String {
    js_sys::encode_uri_component(v)
        .as_string()
        .unwrap_or_default()
}
pub fn hash() -> String {
    web_sys::window()
        .unwrap()
        .location()
        .hash()
        .unwrap_or_default()
        .trim_start_matches('#')
        .into()
}
pub fn navigate(v: &str) {
    let _ = web_sys::window().unwrap().location().set_hash(v);
}
pub fn route() -> RwSignal<String> {
    let state = RwSignal::new(hash());
    let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| state.set(hash()));
    let cb = send_wrapper::SendWrapper::new(cb);
    let win = web_sys::window().unwrap();
    let _ = win.add_event_listener_with_callback("hashchange", cb.as_ref().unchecked_ref());
    on_cleanup(move || {
        let _ = web_sys::window()
            .unwrap()
            .remove_event_listener_with_callback("hashchange", cb.as_ref().unchecked_ref());
    });
    state
}
pub fn resource(
    path: Signal<String>,
    revision: RwSignal<u64>,
    poll_ms: u32,
) -> (RwSignal<Option<Value>>, RwSignal<Option<ApiError>>) {
    resource_typed(path, revision, poll_ms)
}
pub fn resource_typed<T: serde::de::DeserializeOwned + Clone + Send + Sync + 'static>(
    path: Signal<String>,
    revision: RwSignal<u64>,
    poll_ms: u32,
) -> (RwSignal<Option<T>>, RwSignal<Option<ApiError>>) {
    let data = RwSignal::new(None);
    let error = RwSignal::new(None);
    let previous = StoredValue::new(String::new());
    let visible = RwSignal::new(0u64);
    if poll_ms > 0 {
        let cb = Closure::<dyn FnMut(web_sys::Event)>::new(move |_| {
            if !web_sys::window().unwrap().document().unwrap().hidden() {
                visible.update(|v| *v += 1)
            }
        });
        let cb = send_wrapper::SendWrapper::new(cb);
        let doc = web_sys::window().unwrap().document().unwrap();
        let _ =
            doc.add_event_listener_with_callback("visibilitychange", cb.as_ref().unchecked_ref());
        on_cleanup(move || {
            let _ = web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .remove_event_listener_with_callback(
                    "visibilitychange",
                    cb.as_ref().unchecked_ref(),
                );
        });
    }
    Effect::new(move |_| {
        let path = path.get();
        revision.get();
        visible.get();
        let controller = web_sys::AbortController::new().unwrap();
        let stop = controller.signal();
        let cleanup_controller = send_wrapper::SendWrapper::new(controller);
        on_cleanup(move || cleanup_controller.abort());
        if path.is_empty() {
            data.set(None);
            return;
        }
        if previous.get_value() != path {
            data.set(None);
            previous.set_value(path.clone());
        }
        error.set(None);
        xcss_web::task::spawn_local(async move {
            let mut first = true;
            loop {
                if stop.aborted() {
                    break;
                }
                if first || poll_ms == 0 || !web_sys::window().unwrap().document().unwrap().hidden()
                {
                    first = false;
                    let result = api::request_typed_validated_with_options::<T>(
                        &path,
                        "GET",
                        None,
                        RequestOptions {
                            signal: Some(stop.clone()),
                            timeout_ms: 10_000,
                            max_response_bytes: 2 * 1024 * 1024,
                            ..Default::default()
                        },
                        |v| crate::contracts::validate(&path, v),
                    )
                    .await;
                    if stop.aborted() {
                        break;
                    }
                    match result {
                        Ok(v) => {
                            data.set(Some(v));
                            error.set(None)
                        }
                        Err(e) => error.set(Some(response_error(e))),
                    }
                }
                if poll_ms == 0 {
                    break;
                }
                gloo_timers::future::TimeoutFuture::new(poll_ms).await;
            }
        });
    });
    (data, error)
}
/// Serialize a DTO before starting a mutation; failures leave the request unsent.
pub fn mutation_json<T: serde::de::DeserializeOwned + 'static>(
    path: String,
    method: &'static str,
    body: impl serde::Serialize,
    busy: RwSignal<bool>,
    error: RwSignal<Option<ApiError>>,
    done: impl FnOnce(T) + 'static,
) {
    if busy.get_untracked() {
        return;
    }
    match api::encode_json_body(&body) {
        Ok(body) => mutation_typed(path, method, Some(body), busy, error, done),
        Err(e) => error.set(Some(e)),
    }
}
pub fn mutation(
    path: String,
    method: &'static str,
    body: Option<Value>,
    busy: RwSignal<bool>,
    error: RwSignal<Option<ApiError>>,
    done: impl FnOnce(Value) + 'static,
) {
    mutation_typed(path, method, body, busy, error, done)
}
pub fn mutation_typed<T: serde::de::DeserializeOwned + 'static>(
    path: String,
    method: &'static str,
    body: Option<Value>,
    busy: RwSignal<bool>,
    error: RwSignal<Option<ApiError>>,
    done: impl FnOnce(T) + 'static,
) {
    if busy.get_untracked() {
        return;
    }
    busy.set(true);
    error.set(None);
    let controller = web_sys::AbortController::new().unwrap();
    let stop = controller.signal();
    let cleanup_controller = send_wrapper::SendWrapper::new(controller);
    on_cleanup(move || cleanup_controller.abort());
    xcss_web::task::spawn_local(async move {
        let result = api::request_typed_validated_with_options::<T>(
            &path,
            method,
            body,
            RequestOptions {
                signal: Some(stop.clone()),
                ..Default::default()
            },
            |v| valid_mutation_response(&path, method, v),
        )
        .await;
        if stop.aborted() {
            return;
        }
        match result {
            Ok(v) => done(v),
            Err(e) => error.set(Some(response_error(e))),
        }
        busy.set(false);
    });
}
pub fn form(ev: &web_sys::SubmitEvent) -> web_sys::FormData {
    web_sys::FormData::new_with_form(
        &ev.target()
            .unwrap()
            .unchecked_into::<web_sys::HtmlFormElement>(),
    )
    .unwrap()
}
pub fn field(data: &web_sys::FormData, k: &str) -> String {
    data.get(k).as_string().unwrap_or_default()
}
pub fn valid_name(v: &str) -> bool {
    let n = v.trim();
    !n.is_empty() && n.chars().count() <= 32 && !n.chars().any(char::is_control)
}
pub fn bytes(v: f64) -> String {
    let units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB", "EiB"];
    let mut amount = v.max(0.);
    let mut unit = 0;
    while amount >= 1024. && unit < units.len() - 1 {
        amount /= 1024.;
        unit += 1;
    }
    if unit == 0 || amount >= 100. {
        format!("{amount:.0} {}", units[unit])
    } else {
        format!("{amount:.1} {}", units[unit])
    }
}
pub fn timestamp(v: &str) -> String {
    if v == "—" || v.is_empty() {
        return t("尚未配对", "Not paired yet");
    }
    let date = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(v));
    if date.get_time().is_nan() {
        return t("未知", "Unknown");
    }
    let opts = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&opts, &"timeZone".into(), &"UTC".into());
    let _ = js_sys::Reflect::set(&opts, &"hourCycle".into(), &"h23".into());
    format!(
        "{} UTC",
        date.to_locale_string(&xcss_web::i18n::locale(), &opts)
            .as_string()
            .unwrap_or_default()
    )
}
#[component]
pub fn Failure(
    error: RwSignal<Option<ApiError>>,
    #[prop(default=t("操作未能完成，请刷新核对。","The operation did not complete. Refresh to check."))]
    message: String,
) -> impl IntoView {
    move || {
        error.get().map(|e| {
            View::new({
                #[allow(unreachable_code)]
                #[allow(unused_mut)]
                #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
                component_view(
                    #[allow(clippy::needless_borrows_for_generic_args)]
                    &xcss_web::components::ErrorState,
                    {
                        let mut props = component_props_builder(&xcss_web::components::ErrorState)
                            .message(
                                #[allow(unused_braces)]
                                {
                                    message.clone()
                                },
                            )
                            .request_id(
                                #[allow(unused_braces)]
                                {
                                    e.request_id
                                },
                            )
                            .build();
                        props
                    },
                )
            })
        })
    }
}
#[component]
pub fn Pagination(
    data: RwSignal<Option<Value>>,
    cursor: RwSignal<String>,
    revision: RwSignal<u64>,
    #[prop(default=t("实例分页","Instance pages"))] label: String,
) -> impl IntoView {
    View::new(
        xcss_web::html::nav()
            .child((
                xcss_web::html::button()
                    .child(
                        #[allow(unused_braces)]
                        {
                            IntoRender::into_render({ move || t("首页", "First page") })
                        },
                    )
                    .class("xcss-button")
                    .disabled(
                        ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move || {
                            cursor.get().is_empty()
                        }),
                    )
                    .on(xcss_web::ev::click, move |_| {
                        cursor.set(String::new());
                        revision.update(|v| *v += 1);
                    }),
                xcss_web::html::button()
                    .child(
                        #[allow(unused_braces)]
                        {
                            IntoRender::into_render({ move || t("上一页", "Previous page") })
                        },
                    )
                    .class("xcss-button")
                    .disabled(
                        ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move || {
                            data.get()
                                .and_then(|v| v["previous_cursor"].as_str().map(str::to_string))
                                .is_none()
                        }),
                    )
                    .on(xcss_web::ev::click, move |_| {
                        if let Some(c) = data
                            .get_untracked()
                            .and_then(|v| v["previous_cursor"].as_str().map(str::to_string))
                        {
                            cursor.set(c)
                        }
                    }),
                xcss_web::html::button()
                    .child(
                        #[allow(unused_braces)]
                        {
                            IntoRender::into_render({ move || t("下一页", "Next page") })
                        },
                    )
                    .class("xcss-button")
                    .disabled(
                        ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move || {
                            data.get()
                                .and_then(|v| v["next_cursor"].as_str().map(str::to_string))
                                .is_none()
                        }),
                    )
                    .on(xcss_web::ev::click, move |_| {
                        if let Some(c) = data
                            .get_untracked()
                            .and_then(|v| v["next_cursor"].as_str().map(str::to_string))
                        {
                            cursor.set(c)
                        }
                    }),
            ))
            .class("xcss-actions")
            .aria_label(::xcss_web::prelude::IntoAttributeValue::into_attribute_value(label)),
    )
}
#[component]
pub fn HeaderActions(
    create: Callback<()>,
    revision: RwSignal<u64>,
    busy: RwSignal<bool>,
) -> impl IntoView {
    View::new({
        #[allow(unreachable_code)]
        #[allow(unused_mut)]
        #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
        component_view(
            #[allow(clippy::needless_borrows_for_generic_args)]
            &xcss_web::components::HeaderActions,
            {
                let mut props =
                    component_props_builder(&xcss_web::components::HeaderActions)
                        .children({
                            ::xcss_web::children::ToChildren::to_children(move || {
                                (
                        xcss_web::html::button()
                            .child(
                                #[allow(unused_braces)]
                                {
                                    xcss_web::svg::svg()
                                        .child(
                                            #[allow(unused_braces)]
                                            {
                                                xcss_web::svg::path().attr("d", "M12 5v14M5 12h14")
                                            },
                                        )
                                        .attr("width", "1em")
                                        .attr("height", "1em")
                                        .attr("viewBox", "0 0 24 24")
                                        .attr("aria-hidden", "true")
                                        .attr("fill", "none")
                                        .attr("stroke", "currentColor")
                                        .attr("stroke-width", "1.7")
                                },
                            )
                            .class("xcss-button")
                            .aria_label(
                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                t("新建实例", "Create instance")),
                            )
                            .title(
                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                t("新建实例", "Create instance")),
                            )
                            .disabled(
                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                busy.get()),
                            )
                            .on(
                                xcss_web::ev::click,
                                move |_| create.run(()),
                            ),
                        xcss_web::html::button()
                            .child(
                                #[allow(unused_braces)]
                                {
                                    xcss_web::svg::svg()
                                        .child(
                                            #[allow(unused_braces)]
                                            {
                                                xcss_web::svg::path()
                                                    .attr("d", "M20 11a8 8 0 1 0-2 6M20 4v7h-7")
                                            },
                                        )
                                        .attr("width", "1em")
                                        .attr("height", "1em")
                                        .attr("viewBox", "0 0 24 24")
                                        .attr("aria-hidden", "true")
                                        .attr("fill", "none")
                                        .attr("stroke", "currentColor")
                                        .attr("stroke-width", "1.7")
                                },
                            )
                            .class("xcss-button")
                            .aria_label(
                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                t("刷新", "Refresh")),
                            )
                            .title(
                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                t("刷新", "Refresh")),
                            )
                            .disabled(
                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                busy.get()),
                            )
                            .on(
                                xcss_web::ev::click,
                                move |_| revision.update(|v| *v += 1),
                            ),
                    )
                            })
                        })
                        .build();
                props
            },
        )
    })
}
#[component]
pub fn PageNav(selected: Signal<String>, #[prop(default = true)] logs: bool) -> impl IntoView {
    let location = route();
    if !logs {
        return View::new(
            xcss_web::html::nav()
                .child((
                    xcss_web::html::a()
                        .child(
                            #[allow(unused_braces)]
                            {
                                IntoRender::into_render({ move || t("实例", "Instances") })
                            },
                        )
                        .href("#instances"),
                    xcss_web::html::a()
                        .child(
                            #[allow(unused_braces)]
                            {
                                IntoRender::into_render({ move || t("详细信息", "Details") })
                            },
                        )
                        .href(
                            ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                                move || format!("#details/{}", enc(&selected.get())),
                            ),
                        ),
                ))
                .class("xcss-actions")
                .aria_label(
                    ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move || {
                        t("实例导航", "Instance navigation")
                    }),
                ),
        )
        .into_any();
    }
    View::new({
        #[allow(unreachable_code)]
        #[allow(unused_mut)]
        #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
        component_view(
            #[allow(clippy::needless_borrows_for_generic_args)]
            &xcss_web::components::HeaderNavigation,
            {
                let mut props = component_props_builder(&xcss_web::components::HeaderNavigation)
                    .children({
                        ::xcss_web::children::ToChildren::to_children(move || {
                            (
                                xcss_web::html::button()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({
                                                move || t("实例列表", "Instance list")
                                            })
                                        },
                                    )
                                    .class("xcss-button")
                                    .aria_pressed(
                                        ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                                            move || {
                                                (!location.get().starts_with("details")
                                                    && !location.get().starts_with("logs")
                                                    && location.get() != "account")
                                                    .to_string()
                                            },
                                        ),
                                    )
                                    .on(xcss_web::ev::click, move |_| navigate("instances")),
                                xcss_web::html::button()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({
                                                move || t("详细信息", "Details")
                                            })
                                        },
                                    )
                                    .class("xcss-button")
                                    .aria_pressed(
                                        ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                                            move || {
                                                location.get().starts_with("details").to_string()
                                            },
                                        ),
                                    )
                                    .disabled(
                                        ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                                            move || selected.get().is_empty(),
                                        ),
                                    )
                                    .on(xcss_web::ev::click, move |_| {
                                        navigate(&format!(
                                            "details/{}",
                                            enc(&selected.get_untracked())
                                        ))
                                    }),
                                xcss_web::html::button()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({ move || t("日志", "Logs") })
                                        },
                                    )
                                    .class("xcss-button")
                                    .aria_pressed(
                                        ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                                            move || location.get().starts_with("logs").to_string(),
                                        ),
                                    )
                                    .on(xcss_web::ev::click, move |_| {
                                        navigate(&if selected.get_untracked().is_empty() {
                                            "logs".into()
                                        } else {
                                            format!("logs/{}", enc(&selected.get_untracked()))
                                        })
                                    }),
                            )
                        })
                    })
                    .build();
                props
            },
        )
    })
    .into_any()
}
#[component]
pub fn Confirm(
    title: String,
    description: String,
    busy: RwSignal<bool>,
    close: Callback<()>,
    confirm: Callback<()>,
    #[prop(default=RwSignal::new(None))] error: RwSignal<Option<ApiError>>,
    #[prop(default=t("操作未能确认，请刷新核对后再操作。", "The operation could not be confirmed. Refresh and verify before another action."))]
    error_message: String,
) -> impl IntoView {
    View::new({
        #[allow(unreachable_code)]
        #[allow(unused_mut)]
        #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
        component_view(
            #[allow(clippy::needless_borrows_for_generic_args)]
            &xcss_web::components::Dialog,
            {
                let mut props = component_props_builder(
                    &xcss_web::components::Dialog,
                )
                .title(#[allow(unused_braces)] { title })
                .close(
                    #[allow(unused_braces)]
                    {
                        Callback::new(move |()| {
                            if !busy.get_untracked() {
                                close.run(());
                            }
                        })
                    },
                )
                .children({
                    ::xcss_web::children::ToChildren::to_children(move || (
                        xcss_web::html::p()
                            .child(
                                #[allow(unused_braces)]
                                {
                                    IntoRender::into_render({
                                        description.clone()
                                    })
                                },
                            ),
                        {
                            #[allow(unreachable_code)] #[allow(unused_mut)]
                            #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
                            component_view(
                                #[allow(clippy::needless_borrows_for_generic_args)]
                                &Failure,
                                {
                                    let mut props = component_props_builder(
                                            &Failure,
                                        )
                                        .error(#[allow(unused_braces)] { error })
                                        .message(#[allow(unused_braces)] { error_message.clone() })
                                        .build();
                                    props
                                },
                            )
                        },
                        xcss_web::html::div()
                            .child((
                                xcss_web::html::button()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({
                                                t("取消", "Cancel")
                                            })
                                        },
                                    )
                                    .class("xcss-button")
                                    .r#type("button")
                                    .attr("data-xcss-initial-focus", true)
                                    .disabled(
                                        ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                        busy.get()),
                                    )
                                    .on(
                                        xcss_web::ev::click,
                                        move |_| close.run(()),
                                    ),
                                xcss_web::html::button()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({
                                                move || {
                                                    if busy.get() {
                                                        t("正在处理…", "Working…")
                                                    } else {
                                                        t("确认", "Confirm")
                                                    }
                                                }
                                            })
                                        },
                                    )
                                    .class("xcss-button xcss-danger")
                                    .r#type("button")
                                    .disabled(
                                        ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                        busy.get()),
                                    )
                                    .on(
                                        xcss_web::ev::click,
                                        move |_| confirm.run(()),
                                    ),
                            ))
                            .class("xcss-actions"),
                    ))
                })
                .build();
                props
            },
        )
    })
}
pub fn date_parts(date: &str) -> [String; 3] {
    let p: Vec<_> = date.split('-').collect();
    [
        p.first().unwrap_or(&"").to_string(),
        p.get(1)
            .and_then(|v| v.parse::<u32>().ok())
            .map(|v| v.to_string())
            .unwrap_or_default(),
        p.get(2)
            .and_then(|v| v.parse::<u32>().ok())
            .map(|v| v.to_string())
            .unwrap_or_default(),
    ]
}
pub fn date_validate(p: &[String; 3]) -> Result<String, usize> {
    for (i, part) in p.iter().enumerate() {
        if part.is_empty()
            || part.len() > (if i == 0 { 4 } else { 2 })
            || !part.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(i);
        }
    }
    let year = p[0]
        .parse::<u32>()
        .ok()
        .filter(|v| *v >= 1 && *v <= 9999)
        .ok_or(0usize)?;
    let month = p[1]
        .parse::<u32>()
        .ok()
        .filter(|v| *v >= 1 && *v <= 12)
        .ok_or(1usize)?;
    let max = match month {
        2 => {
            if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    let day = p[2]
        .parse::<u32>()
        .ok()
        .filter(|v| *v >= 1 && *v <= max)
        .ok_or(2usize)?;
    Ok(format!("{year:04}-{month:02}-{day:02}"))
}
pub fn date_errors(p: &[String; 3]) -> [bool; 3] {
    let digits = |i: usize, max| {
        !p[i].is_empty() && p[i].len() <= max && p[i].bytes().all(|b| b.is_ascii_digit())
    };
    let year = p[0].parse::<u32>().unwrap_or(0);
    let month = p[1].parse::<u32>().unwrap_or(0);
    let day = p[2].parse::<u32>().unwrap_or(0);
    let year_bad = !digits(0, 4) || !(1..=9999).contains(&year);
    let month_bad = !digits(1, 2) || !(1..=12).contains(&month);
    let max = if year_bad || month_bad {
        31
    } else {
        match month {
            2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
            2 => 28,
            4 | 6 | 9 | 11 => 30,
            _ => 31,
        }
    };
    [year_bad, month_bad, !digits(2, 2) || day < 1 || day > max]
}
#[component]
pub fn DateRange(
    id: &'static str,
    value: RwSignal<(String, String)>,
    apply: Callback<(String, String)>,
) -> impl IntoView {
    let draft = RwSignal::new(std::array::from_fn::<String, 6, _>(|_| String::new()));
    let editing = RwSignal::new(false);
    Effect::new(move |_| {
        let (start, end) = value.get();
        let s = date_parts(&start);
        let e = date_parts(&end);
        draft.set([
            s[0].clone(),
            s[1].clone(),
            s[2].clone(),
            e[0].clone(),
            e[1].clone(),
            e[2].clone(),
        ]);
        editing.set(false);
    });
    let validation = move || {
        let d = draft.get();
        let start = [d[0].clone(), d[1].clone(), d[2].clone()];
        let end = [d[3].clone(), d[4].clone(), d[5].clone()];
        let s = date_validate(&start);
        let e = date_validate(&end);
        let a = date_errors(&start);
        let b = date_errors(&end);
        let mut bad = [a[0], a[1], a[2], b[0], b[1], b[2]];
        let reversed = matches!((&s, &e), (Ok(s), Ok(e)) if s > e);
        if reversed {
            bad = [true; 6];
        }
        (s, e, bad, reversed)
    };
    View::new(
        xcss_web::html::div()
            .child((
                xcss_web::html::div()
                    .child(
                        #[allow(unused_braces)]
                        {
                            IntoRender::into_render({
                                (0..2)
                                    .map(|endpoint| {
                                        View::new(
                                            xcss_web::html::span()
                                                .child((
                                                    IntoRender::into_render({
                                                        (endpoint == 1).then(|| {
                                                            View::new(
                                                                xcss_web::html::span()
                                                                    .child(
                                                                        #[allow(unused_braces)]
                                                                        {
                                                                            "-"
                                                                        },
                                                                    )
                                                                    .class(
                                                                        "xcss-date-range-separator",
                                                                    )
                                                                    .aria_hidden("true"),
                                                            )
                                                        })
                                                    }),
                                                    IntoRender::into_render({
                                                        (0..3)
                                                            .map(move |field| {
                                                                let i = endpoint * 3 + field;
                                                                let part =
                                                                    ["year", "month", "day"][field];
                                                                let label = [
                                                                    t("年", "Year"),
                                                                    t("月", "Month"),
                                                                    t("日", "Day"),
                                                                ][field]
                                                                    .clone();
                                                                View::new(
    xcss_web::html::span()
        .child((
            IntoRender::into_render({
                (field > 0)
                    .then(|| {
                        View::new(
    xcss_web::html::span()
        .child(#[allow(unused_braces)] { "/" })
        .class("xcss-date-range-separator")
        .aria_hidden("true"),
)
                    })
            }),
            xcss_web::html::input()
                .class("xcss-date-range-number")
                .id(
                    ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                        format!(
                            "{id}-{}-{part}", if endpoint == 0 { "start" } else { "end"
                            }
                        ),
                    ),
                )
                .size(
                    ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                        if field == 0 { "4" } else { "2" },
                    ),
                )
                .r#type("text")
                .inputmode("numeric")
                .autocomplete("off")
                .spellcheck("false")
                .aria_label(
                    ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                        format!(
                            "{} {}", if endpoint == 0 { t("开始日期", "Start date")
                            } else { t("结束日期", "End date") }, label
                        ),
                    ),
                )
                .maxlength(
                    ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                        if field == 0 { "4" } else { "2" },
                    ),
                )
                .prop("value", move || draft.get()[i].clone())
                .aria_invalid(
                    ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                    (editing.get() && validation().2[i]).then_some("true")),
                )
                .aria_describedby(
                    ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                    {
                        (editing.get() && validation().2.into_iter().any(|b| b))
                            .then(|| format!("{id}-error"))
                    }),
                )
                .on(
                    xcss_web::ev::input,
                    move |ev| {
                        draft.update(|d| d[i] = event_target_value(&ev));
                        editing.set(true);
                    },
                )
                .on(
                    xcss_web::ev::keydown,
                    move |ev| {
                        if ev.key() == "Enter" && !ev.is_composing() {
                            ev.prevent_default();
                            editing.set(true);
                            let (s, e, bad, _) = validation();
                            if !bad.into_iter().any(|b| b) {
                                apply.run((s.unwrap(), e.unwrap()));
                            }
                        }
                    },
                ),
        ))
        .class("xcss-date-range-part"),
)
                                                            })
                                                            .collect_view()
                                                    }),
                                                ))
                                                .class("xcss-date-range-endpoint"),
                                        )
                                    })
                                    .collect_view()
                            })
                        },
                    )
                    .class("xcss-date-range-fields")
                    .role("group")
                    .aria_label(
                        ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(t(
                            "日期范围",
                            "Date range",
                        )),
                    ),
                IntoRender::into_render({
                    move || {
                        let (_, _, bad, reversed) = validation();
                        (editing.get() && bad.into_iter().any(|v| v)).then(|| {
                            View::new(
    xcss_web::html::p()
        .child(
            #[allow(unused_braces)]
            {
                IntoRender::into_render({
                    if reversed {
                        t(
                            "结束日期不能早于开始日期。",
                            "The end date must not precede the start date.",
                        )
                    } else {
                        t(
                            "请输入有效的年月日。",
                            "Enter a valid year, month and day.",
                        )
                    }
                })
            },
        )
        .class("xcss-date-range-error")
        .id(
            ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                format!("{id}-error"),
            ),
        )
        .role("alert"),
)
                        })
                    }
                }),
            ))
            .class("xcss-date-range")
            .id(::xcss_web::prelude::IntoAttributeValue::into_attribute_value(id)),
    )
}

fn response_error(error: ApiError) -> ApiError {
    if error.status == 0
        && matches!(
            error.code.as_str(),
            "invalid_typed_response_shape" | "invalid_response_contract"
        )
    {
        invalid_response()
    } else {
        error
    }
}
fn invalid_response() -> ApiError {
    ApiError {
        status: 502,
        code: "platform.invalid_response".into(),
        message: "Invalid response".into(),
        request_id: None,
        retryable: false,
        retry_after_seconds: None,
    }
}

fn valid_mutation_response(path: &str, method: &str, value: &Value) -> bool {
    let no_content = method == "DELETE"
        || method == "POST" && path.ends_with("/revoke")
        || method == "PATCH" && path.starts_with("/api/v1/monitoring/client-instances/");
    if no_content {
        value.is_null()
    } else {
        crate::contracts::validate(path, value)
    }
}

#[cfg(test)]
mod typed_response_tests {
    use super::*;
    #[test]
    fn transport_errors_are_not_reclassified_as_contract_errors() {
        let old_transport = ApiError::local("invalid_response_shape");
        assert_eq!(response_error(old_transport.clone()), old_transport);
        let abort = ApiError::local("request_aborted");
        assert_eq!(response_error(abort.clone()), abort);
        let mut unauthorized = ApiError::local("auth.session_required");
        unauthorized.status = 401;
        unauthorized.request_id = Some("request-1".into());
        assert_eq!(response_error(unauthorized.clone()), unauthorized);
        let invalid = response_error(ApiError::local("invalid_typed_response_shape"));
        assert_eq!(invalid.status, 502);
        assert_eq!(invalid.code, "platform.invalid_response");
        assert!(!invalid.retryable);
    }
}
