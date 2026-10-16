use crate::{
    api::{self, AuthPhase},
    i18n::{switch_language, t},
    theme,
};
use gloo_events::EventListener;
use gloo_timers::callback::Timeout;
use leptos::{portal::Portal, prelude::*};
use serde_json::json;
use wasm_bindgen::JsCast;

#[component]
pub fn LoadingState() -> impl IntoView {
    view! { <div class="xcss-loading" role="status" aria-live="polite">{t("正在加载…", "Loading…")}</div> }
}
#[component]
pub fn EmptyState(message: String) -> impl IntoView {
    view! { <div class="xcss-empty">{message}</div> }
}
#[component]
pub fn ErrorState(
    message: String,
    #[prop(optional_no_strip)] request_id: Option<String>,
) -> impl IntoView {
    view! { <div class="xcss-error" role="alert">{message}{request_id.filter(|v| crate::contracts::safe_request_id(v)).map(|id| view! { <p class="xcss-request-id">{t("请求标识：", "Request ID: ")}<code>{id}</code></p> })}</div> }
}

/// Native modal semantics retain focus containment and background inertness.
#[component]
pub fn Dialog(title: String, close: Callback<()>, children: ChildrenFn) -> impl IntoView {
    let dialog = NodeRef::<leptos::html::Dialog>::new();
    let previous = send_wrapper::SendWrapper::new(
        web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.active_element())
            .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok()),
    );
    Effect::new(move |_| {
        if let Some(dialog) = dialog.get() {
            let _ = dialog.show_modal();
            if let Ok(Some(element)) = dialog.query_selector("[data-xcss-initial-focus]")
                && let Ok(element) = element.dyn_into::<web_sys::HtmlElement>()
            {
                let _ = element.focus();
            }
        }
    });
    on_cleanup(move || {
        if let Some(dialog) = dialog.get_untracked() {
            dialog.close();
        }
        if let Some(previous) = previous.as_ref()
            && previous.is_connected()
        {
            let _ = previous.focus();
        }
    });
    view! { <dialog node_ref=dialog class="xcss-dialog" aria-label=title.clone() tabindex="-1"
        on:cancel=move |event: web_sys::Event| { event.prevent_default(); close.run(()); }
        on:keydown=move |event| {
            if event.key() != "Tab" { return; }
            let Some(dialog) = dialog.get_untracked() else { return; };
            if dialog.query_selector("video[controls],audio[controls]").ok().flatten().is_some() { return; }
            let Ok(nodes) = dialog.query_selector_all("button,input,select,textarea,a[href],[tabindex]") else { return; };
            let focusable: Vec<_> = (0..nodes.length()).filter_map(|i| nodes.item(i)).filter_map(|n| n.dyn_into::<web_sys::HtmlElement>().ok()).filter(|e| e.tab_index() >= 0 && !e.matches(":disabled,[hidden]").unwrap_or(true) && e.get_client_rects().length() > 0).collect();
            let active = web_sys::window().and_then(|w| w.document()).and_then(|d| d.active_element());
            if let (Some(first), Some(last)) = (focusable.first(), focusable.last()) {
                if event.shift_key() && (active.as_ref() == Some(first.as_ref()) || active.as_ref() == Some(dialog.as_ref())) { event.prevent_default(); let _ = last.focus(); }
                else if !event.shift_key() && active.as_ref() == Some(last.as_ref()) { event.prevent_default(); let _ = first.focus(); }
            } else { event.prevent_default(); let _ = dialog.focus(); }
        }>
        <div class="xcss-dialog-heading"><h2>{title.clone()}</h2><button class="xcss-button" type="button" aria-label=t("关闭对话框", "Close dialog") on:click=move |_| close.run(())>"×"</button></div>
        {children()}
    </dialog> }
}

#[derive(Clone)]
struct ToastRecord {
    id: u64,
    message: String,
}
#[derive(Clone, Copy)]
struct Toasts {
    records: RwSignal<Vec<ToastRecord>>,
    sequence: RwSignal<u64>,
}
pub fn notify(message: String) {
    // Async completions have no reactive Owner. Dispatch on the current
    // document rather than trying to resolve a context after an await.
    let Some(window) = web_sys::window() else {
        return;
    };
    let init = web_sys::CustomEventInit::new();
    init.set_detail(&wasm_bindgen::JsValue::from_str(&message));
    if let Ok(event) = web_sys::CustomEvent::new_with_event_init_dict("xcss-notify", &init) {
        let _ = window.dispatch_event(&event);
    }
}
fn push_toast(toasts: Toasts, message: String) {
    toasts.sequence.update(|n| *n += 1);
    let id = toasts.sequence.get_untracked();
    toasts.records.update(|records| {
        if records.len() >= 5 {
            records.remove(0);
        }
        records.push(ToastRecord {
            id,
            message: message.chars().take(512).collect(),
        });
    });
    Timeout::new(5_000, move || {
        let _ = toasts
            .records
            .try_update(|records| records.retain(|r| r.id != id));
    })
    .forget();
}

#[derive(Clone, Copy)]
struct HeaderTargets {
    actions: NodeRef<leptos::html::Div>,
    navigation: NodeRef<leptos::html::Div>,
}

#[component]
pub fn HeaderActions(children: ChildrenFn) -> impl IntoView {
    let target = use_context::<HeaderTargets>().map(|targets| targets.actions);
    move || {
        let children = children.clone();
        target.and_then(|r| r.get()).map(|element| view! { <Portal mount={element.unchecked_into::<web_sys::Element>()}>{children()}</Portal> })
    }
}
#[component]
pub fn HeaderNavigation(
    children: ChildrenFn,
    #[prop(default=t("实例工作区", "Instance workspace"))] label: String,
) -> impl IntoView {
    let label = StoredValue::new(label);
    let target = use_context::<HeaderTargets>().map(|targets| targets.navigation);
    move || {
        let children = children.clone();
        target.and_then(|r| r.get()).map(|element| view! { <Portal mount={element.unchecked_into::<web_sys::Element>()}><nav class="xcss-header-navigation" aria-label=label.get_value()>{children()}</nav></Portal> })
    }
}

#[component]
pub fn ThemeToggle() -> impl IntoView {
    let current = RwSignal::new(theme::current());
    if let Some(window) = web_sys::window() {
        let listener = EventListener::new(&window, "xcss-theme-change", move |_| {
            let _ = current.try_set(theme::current());
        });
        let listener = send_wrapper::SendWrapper::new(listener);
        on_cleanup(move || drop(listener));
    }
    view! { <button type="button" class="xcss-button" title=move || if current.get() == "light" { t("切换到深色模式", "Switch to dark mode") } else { t("切换到浅色模式", "Switch to light mode") }
        aria-label=move || if current.get() == "light" { t("切换到深色模式", "Switch to dark mode") } else { t("切换到浅色模式", "Switch to light mode") }
        on:click=move |_| theme::toggle()>
        <svg aria-hidden="true" width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><path d=move || if current.get() == "light" { "M20.8 13A9 9 0 0 1 11 3.2 9 9 0 1 0 20.8 13Z" } else { "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8M12 2v2m0 16v2M2 12h2m16 0h2M5 5l1.4 1.4m11.2 11.2L19 19M5 19l1.4-1.4M17.6 6.4 19 5" } /></svg>
    </button> }
}

#[component]
pub fn LanguageToggle() -> impl IntoView {
    let label = t("切换为英文", "Switch to Chinese");
    view! { <button type="button" class="xcss-button" aria-label=label.clone() title=label on:click=move |_| switch_language()>
        <svg aria-hidden="true" width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><path d="M3 5h12M9 3v2M6 5c0 5 4 9 8 11M13 5c0 5-4 9-9 12m10 4 4-10 4 10m-6.5-4h5" /></svg>
    </button> }
}

#[component]
pub fn AdminShell(
    product: &'static str,
    #[prop(optional)] landing: &'static str,
    children: ChildrenFn,
) -> impl IntoView {
    let auth = RwSignal::new(api::snapshot());
    let pending_logout = RwSignal::new(false);
    let toasts = Toasts {
        records: RwSignal::new(Vec::new()),
        sequence: RwSignal::new(0),
    };
    provide_context(toasts);
    let targets = HeaderTargets {
        actions: NodeRef::new(),
        navigation: NodeRef::new(),
    };
    provide_context(targets);
    let notifications = send_wrapper::SendWrapper::new(EventListener::new(
        &web_sys::window().unwrap(),
        "xcss-notify",
        move |event| {
            if let Some(message) = event
                .dyn_ref::<web_sys::CustomEvent>()
                .and_then(|event| event.detail().as_string())
            {
                push_toast(toasts, message);
            }
        },
    ));
    on_cleanup(move || drop(notifications));
    let listener = send_wrapper::SendWrapper::new(EventListener::new(
        &web_sys::window().unwrap(),
        api::AUTH_CHANGE_EVENT,
        move |_| {
            let next = api::snapshot();
            if next.phase != AuthPhase::Authenticated {
                toasts.records.set(Vec::new());
            }
            auth.set(next);
        },
    ));
    on_cleanup(move || drop(listener));
    wasm_bindgen_futures::spawn_local(async {
        let _ = api::restore().await;
    });
    view! { {move || {
        let current = auth.get();
        if current.phase == AuthPhase::Loading { return view! { <div class="xcss-application-boot" role="status" aria-busy="true" aria-label=t("正在准备应用…", "Preparing application…") /> }.into_any(); }
        if current.phase != AuthPhase::Authenticated {
            return view! { <div class="xcss-auth-shell"><div class="xcss-auth-language" style:position="absolute" style:inset-block-start="1rem" style:inset-inline-end="1rem"><div class="xcss-login-controls" role="group" aria-label=t("显示设置", "Display settings")><LanguageToggle/><ThemeToggle/></div></div><div class="xcss-auth-content">
                {current.account_updated.then(|| view! { <p role="status">{t("账号已更新，请使用新账号信息登录。", "Account updated. Sign in with your updated credentials.")}</p> })}
                {(current.phase == AuthPhase::LogoutUnconfirmed).then(|| view! { <div><ErrorState message=t("本地已退出，但无法确认服务器会话已注销。请重试退出以确认注销。", "Signed out locally, but the server session could not be confirmed as revoked. Retry sign out to confirm revocation.") request_id=current.error.as_ref().and_then(|e| e.request_id.clone())/><button class="xcss-button" on:click=move |_| { wasm_bindgen_futures::spawn_local(async { let _ = api::logout().await; }); }>{t("重试退出", "Retry sign out")}</button></div> })}
                <div class="xcss-auth-card"><div class="xcss-product-identity"><strong>{product}</strong></div>
                {if current.phase == AuthPhase::Error {
                    view! { <div><ErrorState message=t("无法恢复管理员会话。", "Unable to restore administrator session.") request_id=current.error.and_then(|e| e.request_id)/><button class="xcss-button" on:click=move |_| { wasm_bindgen_futures::spawn_local(async { let _ = api::restore().await; }); }>{t("重试", "Retry")}</button></div> }.into_any()
                } else { view! { <LoginPage landing/> }.into_any() }}
                </div></div></div> }.into_any();
        }
        let children = children.clone();
        view! { <div class="xcss-admin-shell" style=("--xcss-header-icon-size", "1em")><a class="xcss-skip-link" href="#xcss-main-content" on:click=move |event| { event.prevent_default(); if let Some(main) = web_sys::window().unwrap().document().unwrap().get_element_by_id("xcss-main-content").and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok()) { let _ = main.focus(); } }>{t("跳至正文", "Skip to content")}</a>
            <header class="xcss-page-header"><div class="xcss-header-navigation-slot"><div class="xcss-header-brand-navigation"><div class="xcss-product-identity"><strong>{product}</strong></div><div node_ref=targets.navigation style:display="contents"/></div></div>
            <div class="xcss-header-actions" role="group" aria-label=t("全局操作", "Global actions")><div node_ref=targets.actions style:display="contents"/><LanguageToggle/><ThemeToggle/>
                <button class="xcss-button" disabled=move || pending_logout.get() aria-label=move || if pending_logout.get() { t("正在退出…", "Signing out…") } else { t("退出", "Sign out") } title=t("退出", "Sign out") on:click=move |_| { if pending_logout.get_untracked() { return; } pending_logout.set(true); wasm_bindgen_futures::spawn_local(async move { let _ = api::logout().await; let _ = pending_logout.try_set(false); }); }><svg aria-hidden="true" width="1em" height="1em" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7"><path d="M9 4H4v16h5m5-12 4 4-4 4M8 12h12"/></svg></button>
                <button class="xcss-button xcss-account-entry" data-xcss-server-corner="none" style:border-radius="50%" style:aspect-ratio="1" aria-label=t("账号设置", "Account settings") title=t("账号设置", "Account settings") on:click=move |_| { let _ = web_sys::window().unwrap().location().set_hash("account"); }><svg viewBox="0 0 24 24" width="1em" height="1em" fill="none" stroke="currentColor" stroke-width="1.7" aria-hidden="true"><circle cx="12" cy="8" r="3.5"/><path d="M4.5 21v-2a7.5 7.5 0 0 1 15 0v2"/></svg></button>
            </div></header>
            <Show when=move || !toasts.records.get().is_empty()><div class="xcss-toast-stack" role="region" aria-label=t("通知", "Notifications")><For each=move || toasts.records.get() key=|r| r.id children=move |r| { let id = r.id; view! { <div class="xcss-toast" role="status" aria-live="polite" aria-atomic="true"><span>{r.message}</span><button class="xcss-button" aria-label=t("关闭通知", "Dismiss notification") on:click=move |_| toasts.records.update(|v| v.retain(|r| r.id != id))>"×"</button></div> } }/></div></Show>
            <div class="xcss-shell-layout xcss-shell-layout--full"><main id="xcss-main-content" class="xcss-shell-main" tabindex="-1">{children()}</main></div>
        </div> }.into_any()
    }} }
}

#[component]
pub fn LoginPage(#[prop(optional)] landing: &'static str) -> impl IntoView {
    let username = NodeRef::<leptos::html::Input>::new();
    let password = NodeRef::<leptos::html::Input>::new();
    let pending = RwSignal::new(false);
    let failure = RwSignal::new(None::<(String, Option<String>)>);
    view! { <form novalidate aria-busy=move || pending.get().to_string() on:input=move |_| failure.set(None) on:submit=move |event| {
        event.prevent_default(); if pending.get_untracked() { return; }
        let name = username.get_untracked().unwrap(); let secret = password.get_untracked().unwrap();
        if name.value().is_empty() { failure.set(Some((t("请输入用户名。", "Enter your username."), None))); let _ = name.focus(); return; }
        if secret.value().is_empty() { failure.set(Some((t("请输入密码。", "Enter your password."), None))); let _ = secret.focus(); return; }
        pending.set(true); failure.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            match api::login(&name.value(), &secret.value()).await {
                Ok(_) => {
                    if !landing.is_empty() {
                        let window = web_sys::window().unwrap();
                        let current = window.location().href().unwrap();
                        if let Ok(url) = web_sys::Url::new_with_base(landing, &current) {
                            if web_sys::Url::new(&current).is_ok_and(|current| current.search_params().has("lang")) {
                                url.search_params().set("lang", &crate::i18n::locale());
                            }
                            let _ = window.history().unwrap().replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&url.href()));
                            let _ = window.dispatch_event(&web_sys::Event::new("hashchange").unwrap());
                        }
                    }
                },
                Err(error) => { let _ = failure.try_set(Some((t("登录失败，请检查用户名和密码后重试。", "Sign in failed. Check your credentials and try again."), error.request_id))); secret.set_value(""); let _ = secret.focus(); }
            }
            let _ = pending.try_set(false);
        });
    }><h1>{t("管理员登录", "Administrator sign in")}</h1>
    <label class="xcss-form-field"><span>{t("用户名", "Username")}</span><input node_ref=username class="xcss-input" name="username" autocomplete="username" required maxlength="64" readonly=move || pending.get()/></label>
    <label class="xcss-form-field"><span>{t("密码", "Password")}</span><input node_ref=password class="xcss-input" name="password" type="password" autocomplete="current-password" required maxlength="1024" readonly=move || pending.get()/></label>
    {move || failure.get().map(|(message, id)| view! { <ErrorState message request_id=id/> })}
    <button class="xcss-button" type="submit" disabled=move || pending.get()>{move || if pending.get() { t("正在登录…", "Signing in…") } else { t("登录", "Sign in") }}</button>
    </form> }
}

#[component]
pub fn AccountPage() -> impl IntoView {
    let username = NodeRef::<leptos::html::Input>::new();
    let current = NodeRef::<leptos::html::Input>::new();
    let new_password = NodeRef::<leptos::html::Input>::new();
    let confirmation = NodeRef::<leptos::html::Input>::new();
    let pending = RwSignal::new(false);
    let failure = RwSignal::new(None::<String>);
    let initial = api::current_session()
        .map(|s| s.username)
        .unwrap_or_default();
    on_cleanup(move || {
        for input in [current, new_password, confirmation] {
            if let Some(input) = input.get_untracked() {
                input.set_value("");
            }
        }
    });
    view! { <section class="xcss-content-stack xcss-account-page" aria-label=t("账号设置", "Account settings")><div class="xcss-content-panel">
        <form class="xcss-account-form" novalidate aria-busy=move || pending.get().to_string() on:input=move |_| failure.set(None) on:submit=move |event| {
            event.prevent_default(); if pending.get_untracked() { return; }
            let name = username.get_untracked().unwrap(); let secret = current.get_untracked().unwrap(); let password = new_password.get_untracked().unwrap(); let confirm = confirmation.get_untracked().unwrap();
            if name.value().len() < 3 || name.value().len() > 64 { failure.set(Some(t("请输入有效的用户名。", "Enter a valid username."))); let _ = name.focus(); return; }
            if secret.value().is_empty() { failure.set(Some(t("请输入当前密码。", "Enter your current password."))); let _ = secret.focus(); return; }
            if !password.value().is_empty() && !crate::contracts::valid_password(&password.value()) { failure.set(Some(t("新密码长度不符合要求。", "The new password length is invalid."))); let _ = password.focus(); return; }
            if password.value() != confirm.value() { failure.set(Some(t("两次输入的新密码不一致。", "The new passwords do not match."))); let _ = confirm.focus(); return; }
            let mut input = json!({"username":name.value(),"current_password":secret.value()}); if !password.value().is_empty() { input["new_password"] = json!(password.value()); }
            pending.set(true); failure.set(None);
            wasm_bindgen_futures::spawn_local(async move {
                if let Err(error) = api::update_account(input).await {
                    let message = match error.code.as_str() { "admin.current_password_invalid" => t("当前密码不正确。", "The current password is incorrect."), "admin.conflict" => t("用户名已被使用。", "The username is already in use."), _ => t("账号未能更新，请检查输入并重试。", "The account could not be updated. Check the input and retry.") };
                    let _ = failure.try_set(Some(message)); secret.set_value(""); password.set_value(""); confirm.set_value(""); let _ = if error.code == "admin.conflict" { name.focus() } else { secret.focus() };
                } else { secret.set_value(""); password.set_value(""); confirm.set_value(""); }
                let _ = pending.try_set(false);
            });
        }><div class="xcss-account-fields">
            <label class="xcss-form-field"><span>{t("用户名", "Username")}</span><input node_ref=username class="xcss-input" name="username" value=initial autocomplete="username" required minlength="3" maxlength="64" readonly=move || pending.get()/></label>
            <label class="xcss-form-field"><span>{t("当前密码", "Current password")}</span><input node_ref=current class="xcss-input" name="current_password" type="password" autocomplete="current-password" required maxlength="1024" readonly=move || pending.get()/></label>
            <label class="xcss-form-field"><span>{t("新密码", "New password")}</span><input node_ref=new_password class="xcss-input" name="new_password" type="password" autocomplete="new-password" maxlength="1024" readonly=move || pending.get()/></label>
            <label class="xcss-form-field"><span>{t("确认新密码", "Confirm new password")}</span><input node_ref=confirmation class="xcss-input" name="confirm_password" type="password" autocomplete="new-password" maxlength="1024" readonly=move || pending.get()/></label>
        </div>{move || failure.get().map(|message| view! { <ErrorState message/> })}<div class="xcss-actions"><button class="xcss-button" type="submit" disabled=move || pending.get()>{move || if pending.get() { t("保存中…", "Saving…") } else { t("保存", "Save") }}</button></div></form>
    </div></section> }
}
