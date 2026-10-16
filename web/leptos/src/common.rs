use leptos::prelude::*;
use serde_json::Value;
use wasm_bindgen::{JsCast, closure::Closure};
use xcss_web::api::{self, ApiError, RequestOptions};
pub use xcss_web::i18n::t;
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
        leptos::task::spawn_local(async move {
            let mut first = true;
            loop {
                if stop.aborted() {
                    break;
                }
                if first || poll_ms == 0 || !web_sys::window().unwrap().document().unwrap().hidden()
                {
                    first = false;
                    let result = api::request_with_options(
                        &path,
                        "GET",
                        None,
                        RequestOptions {
                            signal: Some(stop.clone()),
                            timeout_ms: 10_000,
                            max_response_bytes: 2 * 1024 * 1024,
                            ..Default::default()
                        },
                    )
                    .await;
                    if stop.aborted() {
                        break;
                    }
                    match result {
                        Ok(v) if crate::contracts::validate(&path, &v) => {
                            data.set(Some(v));
                            error.set(None)
                        }
                        Ok(_) => error.set(Some(invalid_response())),
                        Err(e) => error.set(Some(e)),
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
pub fn mutation(
    path: String,
    method: &'static str,
    body: Option<Value>,
    busy: RwSignal<bool>,
    error: RwSignal<Option<ApiError>>,
    done: impl FnOnce(Value) + 'static,
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
    leptos::task::spawn_local(async move {
        let result = api::request_with_options(
            &path,
            method,
            body,
            RequestOptions {
                signal: Some(stop.clone()),
                ..Default::default()
            },
        )
        .await;
        if stop.aborted() {
            return;
        }
        match result {
            Ok(v) if valid_mutation_response(&path, method, &v) => done(v),
            Ok(_) => error.set(Some(invalid_response())),
            Err(e) => error.set(Some(e)),
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
        error.get().map(|e|view!{<xcss_web::components::ErrorState message=message.clone() request_id=e.request_id/>})
    }
}
#[component]
pub fn Pagination(
    data: RwSignal<Option<Value>>,
    cursor: RwSignal<String>,
    revision: RwSignal<u64>,
    #[prop(default=t("实例分页","Instance pages"))] label: String,
) -> impl IntoView {
    view! {<nav class="xcss-actions" aria-label=label><button class="xcss-button" disabled=move||cursor.get().is_empty() on:click=move |_|{cursor.set(String::new());revision.update(|v|*v+=1);}>{move||t("首页","First page")}</button><button class="xcss-button" disabled=move||data.get().and_then(|v|v["previous_cursor"].as_str().map(str::to_string)).is_none() on:click=move |_|{if let Some(c)=data.get_untracked().and_then(|v|v["previous_cursor"].as_str().map(str::to_string)){cursor.set(c)}}>{move||t("上一页","Previous page")}</button><button class="xcss-button" disabled=move||data.get().and_then(|v|v["next_cursor"].as_str().map(str::to_string)).is_none() on:click=move |_|{if let Some(c)=data.get_untracked().and_then(|v|v["next_cursor"].as_str().map(str::to_string)){cursor.set(c)}}>{move||t("下一页","Next page")}</button></nav>}
}
#[component]
pub fn HeaderActions(
    create: Callback<()>,
    revision: RwSignal<u64>,
    busy: RwSignal<bool>,
) -> impl IntoView {
    view! {<xcss_web::components::HeaderActions><button class="xcss-button" aria-label=move||t("新建实例","Create instance") title=move||t("新建实例","Create instance") disabled=move||busy.get() on:click=move |_|create.run(())><svg width="1em" height="1em" viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.7"><path d="M12 5v14M5 12h14"/></svg></button><button class="xcss-button" aria-label=move||t("刷新","Refresh") title=move||t("刷新","Refresh") disabled=move||busy.get() on:click=move |_|revision.update(|v|*v+=1)><svg width="1em" height="1em" viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" stroke-width="1.7"><path d="M20 11a8 8 0 1 0-2 6M20 4v7h-7"/></svg></button></xcss_web::components::HeaderActions>}
}
#[component]
pub fn PageNav(selected: Signal<String>, #[prop(default = true)] logs: bool) -> impl IntoView {
    let location = route();
    if !logs {
        return view!{<nav class="xcss-actions" aria-label=move||t("实例导航","Instance navigation")><a href="#instances">{move||t("实例","Instances")}</a><a href=move||format!("#details/{}",enc(&selected.get()))>{move||t("详细信息","Details")}</a></nav>}.into_any();
    }
    view!{<xcss_web::components::HeaderNavigation><button class="xcss-button" aria-pressed=move||(!location.get().starts_with("details")&&!location.get().starts_with("logs")&&location.get()!="account").to_string() on:click=move |_|navigate("instances")>{move||t("实例列表","Instance list")}</button><button class="xcss-button" aria-pressed=move||location.get().starts_with("details").to_string() disabled=move||selected.get().is_empty() on:click=move |_|navigate(&format!("details/{}",enc(&selected.get_untracked())))>{move||t("详细信息","Details")}</button><button class="xcss-button" aria-pressed=move||location.get().starts_with("logs").to_string() on:click=move |_|navigate(&if selected.get_untracked().is_empty(){"logs".into()}else{format!("logs/{}",enc(&selected.get_untracked()))})>{move||t("日志","Logs")}</button></xcss_web::components::HeaderNavigation>}.into_any()
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
    view! {<xcss_web::components::Dialog title close=Callback::new(move |()| { if !busy.get_untracked() { close.run(()); } })>
        <p>{description.clone()}</p><Failure error message=error_message.clone()/>
        <div class="xcss-actions"><button class="xcss-button" type="button" data-xcss-initial-focus disabled=move||busy.get() on:click=move |_|close.run(())>{t("取消","Cancel")}</button><button class="xcss-button xcss-danger" type="button" disabled=move||busy.get() on:click=move |_|confirm.run(())>{move||if busy.get(){t("正在处理…","Working…")}else{t("确认","Confirm")}}</button></div>
    </xcss_web::components::Dialog>}
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
    view! {
        <div id=id class="xcss-date-range">
            <div class="xcss-date-range-fields" role="group" aria-label=t("日期范围", "Date range")>
                {(0..2).map(|endpoint| view! {
                    <span class="xcss-date-range-endpoint">
                        {(endpoint == 1).then(|| view! { <span class="xcss-date-range-separator" aria-hidden="true">"-"</span> })}
                        {(0..3).map(move |field| {
                            let i = endpoint * 3 + field;
                            let part = ["year", "month", "day"][field];
                            let label = [t("年", "Year"), t("月", "Month"), t("日", "Day")][field].clone();
                            view! {
                                <span class="xcss-date-range-part">
                                    {(field > 0).then(|| view! { <span class="xcss-date-range-separator" aria-hidden="true">"/"</span> })}
                                    <input id=format!("{id}-{}-{part}", if endpoint == 0 { "start" } else { "end" })
                                        class="xcss-date-range-number" size=if field == 0 { "4" } else { "2" }
                                        type="text" inputmode="numeric" autocomplete="off" spellcheck="false"
                                        aria-label=format!("{} {}", if endpoint == 0 { t("开始日期", "Start date") } else { t("结束日期", "End date") }, label)
                                        maxlength=if field == 0 { "4" } else { "2" }
                                        prop:value=move || draft.get()[i].clone()
                                        aria-invalid=move || (editing.get() && validation().2[i]).then_some("true")
                                        aria-describedby=move || (editing.get() && validation().2.into_iter().any(|b| b)).then(|| format!("{id}-error"))
                                        on:input=move |ev| { draft.update(|d| d[i] = event_target_value(&ev)); editing.set(true); }
                                        on:keydown=move |ev| { if ev.key() == "Enter" && !ev.is_composing() { ev.prevent_default(); editing.set(true); let (s, e, bad, _) = validation(); if !bad.into_iter().any(|b| b) { apply.run((s.unwrap(), e.unwrap())); } } }/>
                                </span>
                            }
                        }).collect_view()}
                    </span>
                }).collect_view()}
            </div>
            {move || { let (_, _, bad, reversed) = validation(); (editing.get() && bad.into_iter().any(|v| v)).then(|| view! {
                <p id=format!("{id}-error") class="xcss-date-range-error" role="alert">{if reversed { t("结束日期不能早于开始日期。", "The end date must not precede the start date.") } else { t("请输入有效的年月日。", "Enter a valid year, month and day.") }}</p>
            }) }}
        </div>
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
