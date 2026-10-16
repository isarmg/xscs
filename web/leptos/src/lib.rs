pub mod common;
mod contracts;
use common::*;
use leptos::prelude::*;
use serde_json::{Value, json};
use xcss_web::{
    api::{self, RequestOptions},
    components::{AccountPage, AdminShell},
};
const API: &str = "/api/v1/rustdesk/devices";
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    xcss_web::mount(
        "root",
        || view! {<AdminShell product="xscs" landing="#instances"><Application/></AdminShell>},
    );
}
fn role(v: &Value) -> String {
    if text(v, "role") == "controller" {
        t("控制端", "Controller")
    } else {
        t("被控端", "Controlled host")
    }
}
fn integration(v: &Value) -> String {
    if text(v, "role") == "controller" {
        if flag(v, "client_online") {
            t("控制端已连接", "Controller connected")
        } else {
            t("控制端离线", "Controller offline")
        }
    } else {
        match text(&v["diagnostics"], "integration_state").as_str() {
            "ready" => t("已接入", "Integrated"),
            "unavailable" => t("主机暂不可用", "Host unavailable"),
            _ => t("等待原生客户端就绪", "Awaiting native client readiness"),
        }
    }
}
#[component]
fn Application() -> impl IntoView {
    let location = route();
    let revision = RwSignal::new(0);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None);
    let new_role = RwSignal::new("controlled".to_string());
    let ticket = RwSignal::new(Value::Null);
    let (data, load_error) = resource(Signal::derive(|| API.to_string()), revision, 5000);
    let selected = Signal::derive(move || {
        location
            .get()
            .split('/')
            .nth(1)
            .unwrap_or_default()
            .to_string()
    });
    let create = Callback::new(move |()| {
        mutation(
            API.into(),
            "POST",
            Some(json!({"name":t("新实例","New instance"),"role":new_role.get_untracked()})),
            busy,
            error,
            move |v| {
                navigate(&format!("details/{}", text(&v["device"], "id")));
                ticket.set(v);
                revision.update(|v| *v += 1);
            },
        )
    });
    view! {<section class="xcss-content-stack"><HeaderActions create revision busy/><PageNav selected logs=false/><h1>{move||t("RustDesk 设备管理","RustDesk device management")}</h1><label>{move||t("新实例角色","New instance role")}<select class="xcss-input" prop:value=move||new_role.get() on:change=move|ev|new_role.set(event_target_value(&ev)) disabled=move||busy.get()><option value="controlled">{move||t("被控端","Controlled host")}</option><option value="controller">{move||t("控制端","Controller")}</option></select></label><Failure error=load_error message=t("无法读取管理状态","Unable to load management state")/><Failure error/>{move||{if location.get()=="account"{view!{<AccountPage/>}.into_any()}else if location.get().starts_with("details/"){view!{<For each={move||data.get().and_then(|v|v.as_array().cloned()).unwrap_or_default().into_iter().filter(|d|text(d,"id")==selected.get()).collect::<Vec<_>>()} key=|d|text(d,"id") children=move|initial|{let id=text(&initial,"id");let device=Signal::derive(move||data.get().and_then(|v|v.as_array().and_then(|a|a.iter().find(|d|text(d,"id")==id).cloned())).unwrap_or_else(||initial.clone()));let devices=Signal::derive(move||data.get().and_then(|v|v.as_array().cloned()).unwrap_or_default());view!{<Workspace device devices revision ticket/>}}/>}.into_any()}else{view!{<div>{move||data.get().map(|v|view!{<Instances devices=v.as_array().cloned().unwrap_or_default() revision/>})}</div>}.into_any()}}}</section>}
}
#[component]
fn Instances(devices: Vec<Value>, revision: RwSignal<u64>) -> impl IntoView {
    let candidate = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None);
    if devices.is_empty() {
        return view!{<p>{t("暂无实例。新建实例后，由 RustDesk 内嵌客户端绑定。","No instances. Create one for the embedded RustDesk client to enroll.")}</p>}.into_any();
    }
    view!{<Failure error message=t("删除未能确认，请刷新核对。","Deletion could not be confirmed. Refresh to check.")/><table class="xcss-table" aria-label=t("RustDesk 实例","RustDesk instances")><thead><tr>{[t("名称","Name"),t("角色","Role"),t("管理连接","Management connection"),t("RustDesk 状态","RustDesk status"),t("平台","Platform"),t("最近连接","Last seen"),t("操作","Actions")].into_iter().map(|h|view!{<th>{h}</th>}).collect_view()}</tr></thead><tbody>{devices.into_iter().map(|d|{let id=text(&d,"id");let id2=id.clone();let id3=id.clone();let id4=id.clone();let last=d["last_seen_at_micros"].as_f64().map(|m|{let date=js_sys::Date::new(&wasm_bindgen::JsValue::from_f64(m/1000.));timestamp(&date.to_iso_string().as_string().unwrap_or_default())}).unwrap_or("—".into());view!{<tr><th><a href=format!("#details/{id}")>{text(&d,"name")}</a></th><td>{role(&d)}</td><td>{if flag(&d,"revoked"){t("已撤销","Revoked")}else if flag(&d,"client_online"){t("在线","Online")}else if flag(&d,"registered"){t("离线","Offline")}else{t("等待绑定","Awaiting enrollment")}}</td><td>{integration(&d)}</td><td>{text(&d["capabilities"],"os")}</td><td>{last}</td><td>{move||if candidate.get()==id2{view!{<div class="xcss-actions"><button class="xcss-button" disabled=move||busy.get() on:click=move |_|candidate.set(String::new())>{t("取消","Cancel")}</button><button class="xcss-button" disabled=move||busy.get() on:click={let id=id3.clone();move |_|{let path=format!("{API}/{id}");mutation(path,"DELETE",None,busy,error,move |_|{candidate.set(String::new());revision.update(|v|*v+=1)})}}>{t("确认删除","Confirm delete")}</button></div>}.into_any()}else{view!{<button class="xcss-button" disabled=move||busy.get() on:click={let id=id4.clone();move |_|candidate.set(id.clone())}>{t("删除","Delete")}</button>}.into_any()}}</td></tr>}}).collect_view()}</tbody></table>}.into_any()
}
#[component]
fn Workspace(
    device: Signal<Value>,
    devices: Signal<Vec<Value>>,
    revision: RwSignal<u64>,
    ticket: RwSignal<Value>,
) -> impl IntoView {
    let id = text(&device.get_untracked(), "id");
    let path = format!("{API}/{id}");
    let name = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None);
    let local = RwSignal::new(0);
    let authorization = RwSignal::new(if text(&ticket.get_untracked()["device"], "id") == id {
        ticket.get_untracked()["token"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    } else {
        String::new()
    });
    let confirm = RwSignal::new(false);
    let diagnostic_key = RwSignal::new(String::new());
    let history_path = format!("{path}/tasks?recent=50");
    let (history, history_error) =
        resource(Signal::derive(move || history_path.clone()), local, 5000);
    let revoked = Signal::derive(move || flag(&device.get(), "revoked"));
    let registered = Signal::derive(move || flag(&device.get(), "registered"));
    let rename_path = path.clone();
    let auth_path = path.clone();
    let revoke_path = path.clone();
    let task_path = path.clone();
    view! {<section class="xcss-content-stack"><h2>{move||text(&device.get(),"name")}</h2><p>{move||integration(&device.get())}</p><p>{t("实例角色和控制权限由服务端管理，地址由被控端上报。在原生客户端绑定实例后，从授权列表发起连接。","The server manages instance roles and permissions; controlled hosts report their addresses. Bind an instance in the native client and connect from its allowed hosts.")}</p><form class="xcss-actions" on:submit=move|ev|{ev.prevent_default();mutation(rename_path.clone(),"PATCH",Some(json!({"name":name.get_untracked().unwrap_or_else(||text(&device.get_untracked(),"name"))})),busy,error,move |_|{name.set(None);revision.update(|v|*v+=1)})}><label>{t("实例名称","Instance name")}<input class="xcss-input" prop:value=move||name.get().unwrap_or_else(||text(&device.get(),"name")) maxlength="32" required on:input=move|ev|name.set(Some(event_target_value(&ev)))/></label><button class="xcss-button" type="submit" disabled=move||busy.get()||!valid_name(&name.get().unwrap_or_else(||text(&device.get(),"name")))>{t("保存名称","Save name")}</button></form><div class="xcss-actions"><button class="xcss-button" disabled=move||busy.get()||revoked.get() on:click=move |_|{if !authorization.get_untracked().is_empty(){authorization.set(String::new());}else{mutation(format!("{auth_path}/authorization"),"GET",None,busy,error,move|v|authorization.set(text(&v,"authorization_code")))}}>{move||if authorization.get().is_empty(){t("显示授权码","Show enrollment code")}else{t("隐藏授权码","Hide enrollment code")}}</button><Show when=move||!revoked.get()>{view!{<button class="xcss-button" disabled=move||busy.get() on:click=move |_|confirm.set(true)>{t("撤销客户端凭据","Revoke client credentials")}</button>}}</Show>{move||confirm.get().then(||{let p=revoke_path.clone();view!{<button class="xcss-button" disabled=move||busy.get() on:click=move |_|mutation(format!("{p}/revoke"),"POST",None,busy,error,move |_|{authorization.set(String::new());confirm.set(false);revision.update(|v|*v+=1)})>{t("确认撤销凭据","Confirm credential revocation")}</button><button class="xcss-button" on:click=move |_|confirm.set(false)>{t("取消","Cancel")}</button>}})}</div>{move||(!authorization.get().is_empty()).then(||view!{<div role="status"><p>{t("实例授权码：","Instance enrollment code:")}</p><code>{authorization.get()}</code></div>})}<dl><dt>{t("角色","Role")}</dt><dd>{move||role(&device.get())}</dd><dt>{t("运行方式","Execution mode")}</dt><dd>{t("RustDesk 进程内嵌入","Embedded in RustDesk")}</dd><dt>{t("RustDesk 版本","RustDesk version")}</dt><dd>{move||text(&device.get()["diagnostics"],"rustdesk_version")}</dd></dl>{(text(&device.get_untracked(),"role")=="controlled").then(||view!{<div><h3>{t("被控端上报地址","Reported host addresses")}</h3><ul>{move||rows(&device.get(),"addresses").into_iter().map(|a|{let host=text(&a,"host");let host=if host.contains(':'){format!("[{host}]")}else{host};view!{<li><code>{format!("{host}:{}",num(&a,"port"))}</code></li>}}).collect_view()}</ul><p>{t("最近上报：","Last report: ")}{move||device.get()["addresses_at_micros"].as_f64().map(|m|js_sys::Date::new(&m.div_euclid(1000.).into()).to_iso_string().as_string().unwrap_or_default()).unwrap_or("—".into())}</p><p>{t("离线或超过 45 秒的地址不会下发给控制端。","Addresses are omitted from controller lists when offline or older than 45 seconds.")}</p></div>})}<AccessEditor device devices revision/><div class="xcss-actions"><button class="xcss-button" disabled=move||busy.get()||!registered.get()||revoked.get() on:click=move |_|{if busy.get_untracked(){return;}busy.set(true);error.set(None);let p=format!("{task_path}/tasks");if diagnostic_key.get_untracked().is_empty(){diagnostic_key.set(web_sys::window().unwrap().crypto().unwrap().random_uuid());}let key=diagnostic_key.get_untracked();leptos::task::spawn_local(async move{let result=api::request_with_options(&p,"POST",Some(json!({"kind":"read_diagnostics"})),RequestOptions{headers:vec![("Idempotency-Key".into(),key)],..Default::default()}).await;match result{Ok(_)=>{diagnostic_key.set(String::new());local.update(|v|*v+=1);revision.update(|v|*v+=1)},Err(e)=>error.set(Some(e))}busy.set(false);});}>{t("读取主机诊断","Read host diagnostics")}</button></div><Failure error/><Failure error=history_error/><h3>{t("最近 50 条任务记录","Latest 50 tasks")}</h3>{move||history.get().map(|h|view!{<table class="xcss-table"><thead><tr><th>{t("时间","Time")}</th><th>{t("操作","Action")}</th><th>{t("状态","State")}</th><th>{t("结果","Result")}</th></tr></thead><tbody>{h.as_array().cloned().unwrap_or_default().into_iter().map(|op|view!{<tr><td>{text(&op,"created_at_server")}</td><td>{text(&op,"action")}</td><td>{text(&op,"state")}</td><td><pre>{serde_json::to_string_pretty(if op["reconciliation"].is_null(){&op["result"]}else{&op["reconciliation"]}).unwrap_or_default()}</pre></td></tr>}).collect_view()}</tbody></table>})}</section>}
}
#[component]
fn AccessEditor(
    device: Signal<Value>,
    devices: Signal<Vec<Value>>,
    revision: RwSignal<u64>,
) -> impl IntoView {
    let id = text(&device.get_untracked(), "id");
    let local = RwSignal::new(0);
    let path = format!("{API}/{id}/access");
    let p = path.clone();
    let (rules, load_error) = resource(Signal::derive(move || p.clone()), local, 0);
    let selected = RwSignal::new(Vec::<String>::new());
    let saved = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None);
    Effect::new(move |_| {
        if let Some(r) = rules.get() {
            selected.set(
                rows(&r, "peer_ids")
                    .into_iter()
                    .filter_map(|v| v.as_str().map(str::to_string))
                    .collect(),
            );
        }
    });
    let revoked = Signal::derive(move || flag(&device.get(), "revoked"));
    view! {<section class="xcss-content-stack" aria-label=t("控制权限","Control permissions")><h3>{if text(&device.get_untracked(),"role")=="controller"{t("允许控制的被控主机","Hosts this controller may control")}else{t("允许连接的控制端","Controllers allowed to connect")}}</h3><p>{t("两侧编辑同一份授权关系。未勾选即不允许，每个实例最多 32 个对端。","Both sides edit the same permission. Unselected peers are denied; up to 32 peers per instance.")}</p><Failure error=load_error/><Failure error/><fieldset disabled=move||busy.get()||revoked.get()><legend>{t("授权对端","Allowed peers")}</legend>{move||devices.get().into_iter().filter(|v|text(v,"role")!=text(&device.get(),"role")).map(|peer|{let id=text(&peer,"id");let i2=id.clone();let i3=id.clone();let is_revoked=flag(&peer,"revoked");view!{<label class="rustdesk-access-peer"><input type="checkbox" prop:checked=move||selected.get().contains(&id) disabled=move||!selected.get().contains(&i2)&&(selected.get().len()>=32||is_revoked) on:change=move|ev|{let yes=event_target_checked(&ev);selected.update(|v|{v.retain(|i|i!=&i3);if yes{v.push(i3.clone());}});saved.set(false);}/>{text(&peer,"name")}{if is_revoked{t("（已撤销，请移除）"," (revoked; remove)")}else if !flag(&peer,"registered"){t("（等待绑定）"," (awaiting enrollment)")}else{String::new()}}</label>}}).collect_view()}</fieldset><div class="xcss-actions"><button class="xcss-button" disabled=move||busy.get()||rules.get().is_none()||revoked.get() on:click=move |_|{let body=json!({"peer_ids":selected.get_untracked(),"expected_revision":rules.get_untracked().map(|v|text(&v,"revision")).unwrap_or_default()});mutation(path.clone(),"PUT",Some(body),busy,error,move|v|{rules.set(Some(v));saved.set(true);revision.update(|v|*v+=1);})}>{t("保存控制权限","Save control permissions")}</button><button class="xcss-button" disabled=move||busy.get() on:click=move |_|local.update(|v|*v+=1)>{t("重新读取授权","Reload permissions")}</button></div>{move||saved.get().then(||view!{<p role="status">{t("控制权限已保存。在线客户端将在 5 秒内更新。","Permissions saved. Connected clients update within 5 seconds.")}</p>})}</section>}
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn instance_names() {
        assert!(valid_name("实例 1"));
        assert!(!valid_name("\n"));
        assert!(!valid_name(&"a".repeat(33)));
    }
    #[test]
    fn access_limit_and_roles() {
        let peers = [
            json!({"id":"a","role":"controller"}),
            json!({"id":"b","role":"controlled"}),
        ];
        assert_eq!(
            peers
                .iter()
                .filter(|p| text(p, "role") == "controller")
                .count(),
            1
        );
    }
}
