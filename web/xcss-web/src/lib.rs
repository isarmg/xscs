use xcss_web::component::{component_props_builder, component_view};
pub mod common;
mod contracts;
mod models;
use common::*;
use models::*;
use xcss_web::prelude::*;
use xcss_web::{
    api::{self, ApiError, RequestOptions},
    components::{AccountPage, AdminShell, HeaderNavigation},
    html as h,
};

const API: &str = "/api/v1/rustdesk/devices";

#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    xcss_web::mount_server("root", || {
        component_view(
            &AdminShell,
            component_props_builder(&AdminShell)
                .product("xscs")
                .landing("#instances")
                .children(ToChildren::to_children(application))
                .build(),
        )
    });
}

fn failure(error: RwSignal<Option<ApiError>>) -> impl IntoView {
    component_view(
        &Failure,
        component_props_builder(&Failure).error(error).build(),
    )
}
fn role(v: &Device) -> String {
    if v.role == DeviceRole::Controller {
        t("控制端", "Controller")
    } else {
        t("被控端", "Controlled host")
    }
}
fn integration(v: &Device) -> String {
    if v.role == DeviceRole::Controller {
        if v.client_online {
            t("控制端已连接", "Controller connected")
        } else {
            t("控制端离线", "Controller offline")
        }
    } else {
        match v
            .diagnostics
            .as_ref()
            .map(|d| d.integration_state.as_str())
            .unwrap_or_default()
        {
            "ready" => t("已接入", "Integrated"),
            "unavailable" => t("主机暂不可用", "Host unavailable"),
            _ => t("等待原生客户端就绪", "Awaiting native client readiness"),
        }
    }
}
fn connection(v: &Device) -> String {
    if v.revoked {
        t("已撤销", "Revoked")
    } else if v.client_online {
        t("在线", "Online")
    } else if v.registered {
        t("离线", "Offline")
    } else {
        t("等待绑定", "Awaiting enrollment")
    }
}
fn can_diagnose(v: &Device) -> bool {
    v.role == DeviceRole::Controlled && v.registered && v.client_online && !v.revoked
}
fn matches_filter(v: &Device, query: &str, role_filter: &str) -> bool {
    let matches_role = match role_filter {
        "controller" => v.role == DeviceRole::Controller,
        "controlled" => v.role == DeviceRole::Controlled,
        _ => true,
    };
    let q = query.trim().to_lowercase();
    matches_role
        && (q.is_empty()
            || v.name.to_lowercase().contains(&q)
            || v.id.to_lowercase().contains(&q)
            || v.capabilities
                .as_ref()
                .is_some_and(|c| c.os.to_lowercase().contains(&q)))
}
fn micros(value: Option<serde_json::Number>) -> String {
    value
        .and_then(|v| v.as_f64())
        .map(|value| {
            timestamp(
                &js_sys::Date::new(&(value / 1000.).into())
                    .to_iso_string()
                    .as_string()
                    .unwrap_or_default(),
            )
        })
        .unwrap_or_else(|| t("尚未连接", "Not connected yet"))
}
fn badge(device: Signal<Device>) -> impl IntoView {
    h::span()
        .class("xscs-status")
        .attr("data-state", move || {
            device.with(|v| {
                if v.revoked {
                    "revoked"
                } else if v.client_online {
                    "online"
                } else if v.registered {
                    "offline"
                } else {
                    "pending"
                }
            })
        })
        .child(IntoRender::into_render(move || device.with(connection)))
}

fn application() -> impl IntoView {
    let location = route();
    let revision = RwSignal::new(0);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None);
    let creating = RwSignal::new(false);
    let new_name = RwSignal::new(String::new());
    let new_role = RwSignal::new(DeviceRole::Controlled);
    let ticket = RwSignal::new(None::<Enrollment>);
    let (data, load_error) =
        resource_typed::<Vec<Device>>(Signal::derive(|| API.to_string()), revision, 5000);
    let devices = Signal::derive(move || data.get().unwrap_or_default());
    let selected = Signal::derive(move || {
        location
            .get()
            .split('/')
            .nth(1)
            .unwrap_or_default()
            .to_string()
    });
    let open_create = Callback::new(move |()| {
        creating.set(true);
        navigate("instances");
    });
    let create = Callback::new(move |()| {
        if busy.get_untracked() || !valid_name(&new_name.get_untracked()) {
            return;
        }
        mutation_json::<Enrollment>(
            API.into(),
            "POST",
            CreateDevice {
                name: new_name.get_untracked().trim().to_string(),
                role: new_role.get_untracked(),
            },
            busy,
            error,
            move |value| {
                let id = value.device.id.clone();
                ticket.set(Some(value));
                creating.set(false);
                new_name.set(String::new());
                revision.update(|v| *v += 1);
                navigate(&format!("details/{id}"));
            },
        );
    });
    View::new(h::section().class("xcss-content-stack xscs-workspace").child((
        component_view(&HeaderActions, component_props_builder(&HeaderActions).create(open_create).revision(revision).busy(busy).build()),
        component_view(&HeaderNavigation, component_props_builder(&HeaderNavigation)
            .label(t("实例导航", "Instance navigation"))
            .children(ToChildren::to_children(move || (
                h::a().href("#instances").attr("aria-current", move || (!location.get().starts_with("details/") && location.get() != "account").then_some("page"))
                    .child(IntoRender::into_render(move || t("实例", "Instances"))),
                IntoRender::into_render(move || (!selected.get().is_empty()).then(||
                    h::a().href(format!("#details/{}", enc(&selected.get())))
                        .attr("aria-current", move || location.get().starts_with("details/").then_some("page"))
                        .child(t("详细信息", "Details")))),
            ))).build()),
        failure(load_error), failure(error),
        IntoRender::into_render(move || creating.get().then(|| h::section().class("xscs-panel").aria_label(t("新建实例", "Create instance")).child((
            h::div().class("xscs-panel-heading").child((h::h2().child(t("新建实例", "Create instance")),
                h::p().child(t("先选择实例用途。创建后生成实例密码，再到原生客户端完成绑定。", "Choose the instance role. Creation generates a password for enrollment in the native client.")))),
            h::form().class("xscs-form-grid").on(xcss_web::ev::submit, move |event| { event.prevent_default(); create.run(()); }).child((
                h::label().class("xscs-field").child((h::span().child(t("实例名称", "Instance name")),
                    h::input().class("xcss-input").name("instance_name").maxlength("32").required(true)
                        .prop("value", move || new_name.get()).on(xcss_web::ev::input, move |event| new_name.set(event_target_value(&event)))
                        .disabled(IntoAttributeValue::into_attribute_value(move || busy.get())))),
                h::label().class("xscs-field").child((h::span().child(t("新实例角色", "New instance role")),
                    h::select().class("xcss-input").name("instance_role")
                        .prop("value", move || if new_role.get() == DeviceRole::Controller { "controller" } else { "controlled" })
                        .on(xcss_web::ev::change, move |event| new_role.set(if event_target_value(&event) == "controller" { DeviceRole::Controller } else { DeviceRole::Controlled }))
                        .disabled(IntoAttributeValue::into_attribute_value(move || busy.get()))
                        .child((h::option().value("controlled").child(t("被控端", "Controlled host")), h::option().value("controller").child(t("控制端", "Controller")))))),
                h::p().class("xscs-hint").child(t("控制端用于发起连接；被控端用于共享这台主机。角色在创建后保持固定。", "Controllers initiate connections; controlled hosts share their desktop. The role is fixed after creation.")),
                h::div().class("xcss-actions").child((
                    h::button().class("xcss-button xscs-primary").r#type("submit").disabled(IntoAttributeValue::into_attribute_value(move || busy.get() || !valid_name(&new_name.get())))
                        .child(t("创建实例并生成密码", "Create instance and generate password")),
                    h::button().class("xcss-button").r#type("button").disabled(IntoAttributeValue::into_attribute_value(move || busy.get()))
                        .on(xcss_web::ev::click, move |_| creating.set(false)).child(t("取消", "Cancel")),
                )),
            )),
        )))),
        IntoRender::into_render(move || {
            if location.get() == "account" {
                View::new(component_view(&AccountPage, component_props_builder(&AccountPage).build())).into_any()
            } else if location.get().starts_with("details/") {
                View::new(component_view(&Show, component_props_builder(&Show)
                    .when(move || data.with(|items| items.as_ref().is_some_and(|items| items.iter().any(|v| v.id == selected.get()))))
                    .fallback(move || h::section().class("xscs-panel xscs-empty").child(IntoRender::into_render(move || {
                        if data.get().is_none() && load_error.get().is_none() {
                            View::new(h::p().child(t("正在读取实例…", "Loading instance…"))).into_any()
                        } else { View::new((h::h2().child(t("实例不存在或暂不可读取", "Instance not found or unavailable")),
                            h::p().class("xscs-hint").child(t("实例可能已被删除。返回实例列表，或刷新后重试。", "The instance may have been deleted. Return to the instance list or refresh to retry.")),
                            h::a().class("xcss-button").href("#instances").child(t("返回实例列表", "Back to instances")))).into_any() }
                    })))
                    .children(ToChildren::to_children(move || component_view(&For, component_props_builder(&For)
                        .each(move || devices.get().into_iter().filter(|v| v.id == selected.get()).collect::<Vec<_>>())
                        .key(|v: &Device| v.id.clone())
                        .children(move |initial: Device| {
                            let id = initial.id.clone();
                            let device = Signal::derive(move || devices.with(|items| items.iter().find(|v| v.id == id).cloned()).unwrap_or_else(|| initial.clone()));
                            workspace(device, data, revision, ticket)
                        }).build()))).build())).into_any()
            } else {
                View::new(instances(devices, Signal::derive(move || data.get().is_some()), revision)).into_any()
            }
        }),
    )))
}

fn instances(
    devices: Signal<Vec<Device>>,
    loaded: Signal<bool>,
    revision: RwSignal<u64>,
) -> impl IntoView {
    let query = RwSignal::new(String::new());
    let role_filter = RwSignal::new("all".to_string());
    let candidate = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None);
    let filtered = Signal::derive(move || {
        devices
            .get()
            .into_iter()
            .filter(|v| matches_filter(v, &query.get(), &role_filter.get()))
            .collect::<Vec<_>>()
    });
    View::new(h::div().class("xcss-content-stack").child((
        h::div().class("xscs-page-heading").child(h::div().child((h::h1().child(t("实例管理", "Instance management")),
            h::p().child(t("管理原生控制端与被控主机。先创建、绑定，再配置连接权限。", "Manage native controllers and controlled hosts: create, enroll, then configure access."))))),
        h::div().class("xscs-metrics").child(
            [(0, t("全部实例", "All instances")), (1, t("在线实例", "Online instances")), (2, t("控制端", "Controllers")), (3, t("被控端", "Controlled hosts"))]
                .into_iter().map(move |(kind, label)| h::div().class("xscs-metric").child((h::span().child(label), h::strong().child(IntoRender::into_render(move || devices.with(|items| match kind {
                    0 => items.len(), 1 => items.iter().filter(|v| v.client_online && !v.revoked).count(), 2 => items.iter().filter(|v| v.role == DeviceRole::Controller).count(), _ => items.iter().filter(|v| v.role == DeviceRole::Controlled).count(),
                })))))).collect_view()),
        h::section().class("xscs-panel").child((
            h::div().class("xscs-toolbar").child((
                h::label().class("xscs-field").child((h::span().child(t("搜索实例", "Search instances")), h::input().class("xcss-input").r#type("search")
                    .placeholder(t("名称、实例 ID 或平台", "Name, instance ID or platform")).prop("value", move || query.get()).on(xcss_web::ev::input, move |event| query.set(event_target_value(&event))))),
                h::label().class("xscs-field").child((h::span().child(t("筛选角色", "Filter by role")), h::select().class("xcss-input").prop("value", move || role_filter.get())
                    .on(xcss_web::ev::change, move |event| role_filter.set(event_target_value(&event)))
                    .child((h::option().value("all").child(t("全部角色", "All roles")), h::option().value("controller").child(t("控制端", "Controller")), h::option().value("controlled").child(t("被控端", "Controlled host")))))),
                h::p().class("xscs-hint").role("status").child(IntoRender::into_render(move || format!("{} / {}", filtered.get().len(), devices.get().len()))),
            )), failure(error),
            IntoRender::into_render(move || (!loaded.get()).then(|| h::p().class("xscs-hint").child(t("正在读取实例…", "Loading instances…")))),
            IntoRender::into_render(move || (loaded.get() && filtered.get().is_empty()).then(|| h::p().class("xscs-empty").child(
                if devices.get().is_empty() { t("暂无实例。点击顶部“新建实例”，创建后在原生客户端绑定。", "No instances. Select Create instance above, then enroll in the native client.") }
                else { t("没有符合搜索或角色筛选的实例。", "No instances match the search and role filter.") }))),
            h::div().class("xcss-table-scroll").child(h::table().class("xcss-table").aria_label(t("RustDesk 实例", "RustDesk instances")).child((
                h::thead().child(h::tr().child([t("名称", "Name"), t("角色", "Role"), t("管理连接", "Management connection"), t("RustDesk 状态", "RustDesk status"), t("平台", "Platform"), t("最近连接", "Last seen"), t("操作", "Actions")].into_iter().map(|label| h::th().child(label)).collect_view())),
                h::tbody().child(component_view(&For, component_props_builder(&For).each(move || filtered.get()).key(|v: &Device| v.id.clone())
                    .children(move |initial: Device| {
                        let id = initial.id.clone();
                        let device = Signal::derive(move || devices.with(|items| items.iter().find(|v| v.id == id).cloned()).unwrap_or_else(|| initial.clone()));
                        instance_row(device, candidate, busy, error, revision)
                    }).build())),
            ))),
        )),
    )))
}

fn instance_row(
    device: Signal<Device>,
    candidate: RwSignal<String>,
    busy: RwSignal<bool>,
    error: RwSignal<Option<ApiError>>,
    revision: RwSignal<u64>,
) -> impl IntoView {
    let id = StoredValue::new(device.with_untracked(|v| v.id.clone()));
    let remove = Callback::new(move |()| {
        if busy.get_untracked() || candidate.get_untracked() != id.get_value() {
            return;
        }
        mutation(
            format!("{API}/{}", id.get_value()),
            "DELETE",
            None,
            busy,
            error,
            move |_| {
                candidate.set(String::new());
                revision.update(|v| *v += 1);
            },
        );
    });
    h::tr().child((
        h::th().attr("scope", "row").child(
            h::a()
                .href(format!("#details/{}", id.get_value()))
                .child(IntoRender::into_render(move || {
                    device.with(|v| v.name.clone())
                })),
        ),
        h::td().child(IntoRender::into_render(move || device.with(role))),
        h::td().child(badge(device)),
        h::td().child(IntoRender::into_render(move || device.with(integration))),
        h::td().child(IntoRender::into_render(move || {
            device.with(|v| {
                v.capabilities
                    .as_ref()
                    .map(|v| v.os.clone())
                    .unwrap_or("—".into())
            })
        })),
        h::td().child(IntoRender::into_render(move || {
            micros(device.with(|v| v.last_seen_at_micros.clone()))
        })),
        h::td().child(IntoRender::into_render(move || {
            if candidate.get() == id.get_value() {
                View::new(
                    h::div().class("xcss-actions").child((
                        h::button()
                            .class("xcss-button")
                            .disabled(IntoAttributeValue::into_attribute_value(move || busy.get()))
                            .on(xcss_web::ev::click, move |_| candidate.set(String::new()))
                            .child(t("取消", "Cancel")),
                        h::button()
                            .class("xcss-button xscs-danger")
                            .disabled(IntoAttributeValue::into_attribute_value(move || busy.get()))
                            .on(xcss_web::ev::click, move |_| remove.run(()))
                            .child(t("确认删除", "Confirm delete")),
                    )),
                )
                .into_any()
            } else {
                View::new(
                    h::button()
                        .class("xcss-button xscs-danger")
                        .disabled(IntoAttributeValue::into_attribute_value(move || busy.get()))
                        .on(xcss_web::ev::click, move |_| candidate.set(id.get_value()))
                        .child(t("删除", "Delete")),
                )
                .into_any()
            }
        })),
    ))
}

fn workspace(
    device: Signal<Device>,
    data: RwSignal<Option<Vec<Device>>>,
    revision: RwSignal<u64>,
    ticket: RwSignal<Option<Enrollment>>,
) -> impl IntoView {
    let devices = Signal::derive(move || data.get().unwrap_or_default());
    let id = device.with_untracked(|v| v.id.clone());
    let path = format!("{API}/{id}");
    let name = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None);
    let initial_password = ticket
        .try_update(|value| {
            if value.as_ref().is_some_and(|value| value.device.id == id) {
                value.take().map(|value| value.token)
            } else {
                None
            }
        })
        .flatten()
        .unwrap_or_default();
    let authorization = RwSignal::new(initial_password);
    let confirm = RwSignal::new(false);
    let reset_confirm = RwSignal::new(false);
    let binding_notice = RwSignal::new(String::new());
    let diagnostic_key = RwSignal::new(String::new());
    let history_path = format!("{path}/tasks?recent=50");
    let (history, history_error) = resource_typed::<Vec<Operation>>(
        Signal::derive(move || history_path.clone()),
        revision,
        5000,
    );
    let revoked = Signal::derive(move || device.with(|v| v.revoked));
    Effect::new(move |_| {
        if revoked.get() || !device.with(|v| v.pairing_pending) {
            authorization.set(String::new());
        }
        if revoked.get() {
            confirm.set(false);
            reset_confirm.set(false);
        } else if device.with(|v| v.registered && !v.pairing_pending) {
            binding_notice.set(String::new());
        }
    });
    let rename_path = path.clone();
    let rename = Callback::new(move |()| {
        let value = name
            .get_untracked()
            .unwrap_or_else(|| device.with_untracked(|v| v.name.clone()));
        if busy.get_untracked() || revoked.get_untracked() || !valid_name(&value) {
            return;
        }
        mutation_json::<Device>(
            rename_path.clone(),
            "PATCH",
            RenameDevice {
                name: value.trim().into(),
            },
            busy,
            error,
            move |_| {
                name.set(None);
                revision.update(|v| *v += 1);
            },
        );
    });
    let auth_path = path.clone();
    let reveal = Callback::new(move |()| {
        if busy.get_untracked()
            || revoked.get_untracked()
            || !device.with_untracked(|v| v.pairing_pending)
        {
            return;
        }
        if !authorization.get_untracked().is_empty() {
            authorization.set(String::new());
            return;
        }
        mutation_typed::<Authorization>(
            format!("{auth_path}/authorization"),
            "GET",
            None,
            busy,
            error,
            move |value| authorization.set(value.authorization_code),
        );
    });
    let reset_path = path.clone();
    let reset_device_id = id.clone();
    let reset_password = Callback::new(move |()| {
        if busy.get_untracked() || revoked.get_untracked() || !reset_confirm.get_untracked() {
            return;
        }
        let code = match fresh_instance_password() {
            Ok(code) => code,
            Err(value) => {
                error.set(Some(value));
                return;
            }
        };
        let reset_device_id = reset_device_id.clone();
        mutation_json::<Authorization>(
            format!("{reset_path}/authorization"),
            "PUT",
            UpdateAuthorization {
                authorization_code: code,
            },
            busy,
            error,
            move |value| {
                data.update(|items| {
                    if let Some(current) = items
                        .as_mut()
                        .and_then(|items| items.iter_mut().find(|v| v.id == reset_device_id))
                    {
                        current.registered = false;
                        current.pairing_pending = true;
                        current.client_online = false;
                        current.addresses.clear();
                        current.addresses_at_micros = None;
                        current.last_seen_at_micros = None;
                        current.capabilities = None;
                        current.diagnostics = None;
                    }
                });
                authorization.set(value.authorization_code);
                reset_confirm.set(false);
                binding_notice.set(t("新实例密码已生成。旧客户端凭据已失效，请在客户端解除原绑定后重新绑定。", "A new instance password was generated. Old client credentials are invalid; unbind the old instance in the client, then enroll again."));
                revision.update(|v| *v += 1);
            },
        );
    });
    let revoke_path = path.clone();
    let revoke_device_id = id.clone();
    let revoke = Callback::new(move |()| {
        if busy.get_untracked() || revoked.get_untracked() || !confirm.get_untracked() {
            return;
        }
        let revoke_device_id = revoke_device_id.clone();
        mutation(
            format!("{revoke_path}/revoke"),
            "POST",
            None,
            busy,
            error,
            move |_| {
                data.update(|items| {
                    if let Some(current) = items
                        .as_mut()
                        .and_then(|items| items.iter_mut().find(|v| v.id == revoke_device_id))
                    {
                        current.revoked = true;
                        current.pairing_pending = false;
                        current.client_online = false;
                    }
                });
                authorization.set(String::new());
                confirm.set(false);
                revision.update(|v| *v += 1);
            },
        );
    });
    let diagnose = Callback::new(move |()| {
        if busy.get_untracked() || !device.with_untracked(can_diagnose) {
            return;
        }
        busy.set(true);
        error.set(None);
        let p = format!("{path}/tasks");
        if diagnostic_key.get_untracked().is_empty() {
            let Some(crypto) = web_sys::window().and_then(|window| window.crypto().ok()) else {
                busy.set(false);
                error.set(Some(ApiError::local("request_identifier_unavailable")));
                return;
            };
            let mut bytes = [0u8; 16];
            if crypto.get_random_values_with_u8_array(&mut bytes).is_err() {
                busy.set(false);
                error.set(Some(ApiError::local("request_identifier_unavailable")));
                return;
            }
            // getRandomValues also works on development HTTP origins where randomUUID is absent.
            bytes[6] = (bytes[6] & 0x0f) | 0x40;
            bytes[8] = (bytes[8] & 0x3f) | 0x80;
            let hex = bytes
                .iter()
                .map(|value| format!("{value:02x}"))
                .collect::<String>();
            diagnostic_key.set(format!(
                "{}-{}-{}-{}-{}",
                &hex[..8],
                &hex[8..12],
                &hex[12..16],
                &hex[16..20],
                &hex[20..]
            ));
        }
        let key = diagnostic_key.get_untracked();
        let Ok(controller) = web_sys::AbortController::new() else {
            busy.set(false);
            error.set(Some(ApiError::local("request_unavailable")));
            return;
        };
        let stop = controller.signal();
        let cleanup = send_wrapper::SendWrapper::new(controller);
        on_cleanup(move || cleanup.abort());
        let body = match api::encode_json_body(&DeviceCommand::ReadDiagnostics) {
            Ok(body) => body,
            Err(value) => {
                error.set(Some(value));
                busy.set(false);
                return;
            }
        };
        xcss_web::task::spawn_local(async move {
            let result = api::request_typed_validated_with_options::<Operation>(
                &p,
                "POST",
                Some(body),
                RequestOptions {
                    signal: Some(stop.clone()),
                    headers: vec![("Idempotency-Key".into(), key)],
                    ..Default::default()
                },
                |value| crate::contracts::validate(&p, value),
            )
            .await;
            if stop.aborted() {
                return;
            }
            match result {
                Ok(_) => {
                    diagnostic_key.set(String::new());
                    revision.update(|v| *v += 1);
                }
                Err(value) => error.set(Some(value)),
            }
            busy.set(false);
        });
    });
    let origin = web_sys::window()
        .and_then(|window| window.location().origin().ok())
        .unwrap_or_default();
    View::new(h::div().class("xcss-content-stack").child((
        h::div().class("xscs-page-heading").child((h::div().child((h::h1().child(IntoRender::into_render(move || device.with(|v| v.name.clone()))),
            h::p().child(IntoRender::into_render(move || format!("{} · {}", device.with(role), device.with(integration)))))),
            h::a().class("xcss-button").href("#instances").child(t("返回实例列表", "Back to instances")))),
        failure(error),
        h::div().class("xscs-split").child((
            h::section().class("xscs-panel").child((h::h2().child(t("基本信息", "Basic information")), badge(device),
                h::form().class("xscs-form-grid").on(xcss_web::ev::submit, move |event| { event.prevent_default(); rename.run(()); }).child((
                    h::label().class("xscs-field xscs-field--wide").child((h::span().child(t("实例名称", "Instance name")), h::input().class("xcss-input").maxlength("32").required(true)
                        .prop("value", move || name.get().unwrap_or_else(|| device.with(|v| v.name.clone())))
                        .on(xcss_web::ev::input, move |event| name.set(Some(event_target_value(&event))))
                        .disabled(IntoAttributeValue::into_attribute_value(move || busy.get() || revoked.get())))),
                    h::div().class("xcss-actions").child(h::button().class("xcss-button xscs-primary").r#type("submit")
                        .disabled(IntoAttributeValue::into_attribute_value(move || busy.get() || revoked.get() || !valid_name(&name.get().unwrap_or_else(|| device.with(|v| v.name.clone())))))
                        .child(t("保存名称", "Save name"))),
                )),
                h::dl().child((h::dt().child(t("实例 ID", "Instance ID")), h::dd().child(h::code().child(id)),
                    h::dt().child(t("角色", "Role")), h::dd().child(IntoRender::into_render(move || device.with(role))),
                    h::dt().child(t("运行方式", "Execution mode")), h::dd().child(t("原生客户端内嵌远程引擎", "Remote engine embedded in the native client")),
                    h::dt().child(t("RustDesk 版本", "RustDesk version")), h::dd().child(IntoRender::into_render(move || device.with(|v| v.diagnostics.as_ref().and_then(|v| v.rustdesk_version.clone())).unwrap_or("—".into()))))),
                IntoRender::into_render(move || (device.with(|v| v.role) == DeviceRole::Controlled).then(|| h::div().class("xcss-content-stack").child((
                    h::h3().child(t("被控端上报地址", "Reported host addresses")),
                    h::ul().child(IntoRender::into_render(move || device.with(|v| v.addresses.clone()).into_iter().map(|address| {
                        let host = if address.host.contains(':') { format!("[{}]", address.host) } else { address.host };
                        h::li().child(h::code().child(format!("{host}:{}", address.port)))
                    }).collect_view())),
                    h::p().class("xscs-hint").child(IntoRender::into_render(move || format!("{}{}", t("最近上报：", "Last report: "), micros(device.with(|v| v.addresses_at_micros.clone()))))),
                    h::p().class("xscs-hint").child(t("离线或超过 45 秒的地址不会下发给控制端。", "Addresses are omitted from controller lists when offline or older than 45 seconds.")),
                )))),
            )),
            h::section().class("xscs-panel").child((h::h2().child(t("客户端绑定", "Client enrollment")),
                h::p().class("xscs-status").child(IntoRender::into_render(move || binding_state(&device.get()))),
                IntoRender::into_render(move || (!binding_notice.get().is_empty()).then(|| h::p().class("xscs-hint").role("status").child(binding_notice.get()))),
                h::p().class("xscs-hint").child(IntoRender::into_render(move || {
                    device.with(|v| if v.revoked {
                        t("该实例已永久撤销，不可再绑定。需要恢复请新建实例，在客户端解除原绑定后使用新实例密码绑定。", "This instance is permanently revoked and cannot enroll again. Create a new instance, unbind the old instance in the client, and enroll with the new instance password.")
                    } else if v.registered {
                        t("该实例已绑定。旧密码不可重复绑定；需要重新绑定时，请先重置实例密码。", "This instance is enrolled. The old password cannot be reused for enrollment; reset the instance password before enrolling again.")
                    } else if v.pairing_pending {
                        t("此实例正在等待绑定，可使用当前实例密码。", "This instance is awaiting enrollment; the current instance password can be used.")
                    } else {
                        t("当前没有可用的绑定密码。请重置实例密码后再绑定。", "There is no usable enrollment password. Reset the instance password before enrolling.")
                    })
                })),
                h::ol().class("xscs-step-list").child((h::li().child(t("在对应角色的原生客户端打开实例绑定。", "Open instance enrollment in the native client for this role.")),
                    h::li().child(t("填写以下服务器地址和实例密码。", "Enter the server address and instance password below.")),
                    h::li().child(t("绑定完成后，配置控制授权再发起连接。", "After enrollment, configure access before connecting.")))),
                h::label().class("xscs-field").child((h::span().child(t("服务器地址", "Server address")), h::input().class("xcss-input").attr("readonly", "").value(origin))),
                h::p().class("xscs-hint").child(t("跨设备绑定时请使用客户端可访问的服务器局域网地址或 HTTPS 域名；127.0.0.1 仅用于本机。", "Use a server LAN address or HTTPS domain reachable by the client. 127.0.0.1 is for the local machine only.")),
                h::div().class("xcss-actions").child(h::button().class("xcss-button").disabled(IntoAttributeValue::into_attribute_value(move || busy.get() || revoked.get() || !device.with(|v| v.pairing_pending)))
                    .on(xcss_web::ev::click, move |_| reveal.run(())).child(IntoRender::into_render(move || if authorization.get().is_empty() { t("显示实例密码", "Show instance password") } else { t("隐藏实例密码", "Hide instance password") }))),
                IntoRender::into_render(move || (!authorization.get().is_empty()).then(|| h::div().class("xcss-content-stack").role("status").child((
                    h::p().child(t("实例密码（授权码）：", "Instance password (enrollment code):")), h::code().class("xscs-sensitive").child(authorization.get()),
                    h::p().class("xscs-hint").child(t("仅交给该实例的客户端，完成绑定后请隐藏。此密码不是管理员登录密码。", "Share only with this instance's client, then hide it. This is not the administrator login password.")),
                )))),
                h::button().class("xcss-button").disabled(IntoAttributeValue::into_attribute_value(move || busy.get() || revoked.get())).on(xcss_web::ev::click, move |_| { authorization.set(String::new()); confirm.set(false); reset_confirm.set(true); }).child(t("重置实例密码", "Reset instance password")),
                IntoRender::into_render(move || reset_confirm.get().then(|| h::div().class("xcss-content-stack").child((
                    h::p().class("xscs-hint").child(t("重置会使旧密码及旧客户端凭据失效。请在客户端解除原绑定，再使用新密码重新绑定。", "Resetting invalidates the old password and client credentials. Unbind the old instance in the client, then enroll with the new password.")),
                    h::div().class("xcss-actions").child((
                        h::button().class("xcss-button xscs-primary").disabled(IntoAttributeValue::into_attribute_value(move || busy.get() || revoked.get())).on(xcss_web::ev::click, move |_| reset_password.run(())).child(t("确认重置实例密码", "Confirm instance password reset")),
                        h::button().class("xcss-button").disabled(IntoAttributeValue::into_attribute_value(move || busy.get())).on(xcss_web::ev::click, move |_| reset_confirm.set(false)).child(t("取消重置", "Cancel reset")),
                    )),
                )))),
                h::button().class("xcss-button xscs-danger").disabled(IntoAttributeValue::into_attribute_value(move || busy.get() || revoked.get())).on(xcss_web::ev::click, move |_| { reset_confirm.set(false); confirm.set(true); }).child(t("撤销客户端凭据", "Revoke client credentials")),
                IntoRender::into_render(move || confirm.get().then(|| h::div().class("xcss-content-stack").child((
                    h::p().class("xscs-hint").child(t("撤销会使该实例永久失效，不可再次绑定。需要恢复请新建实例，并在客户端解除原绑定后换用新实例密码。", "Revocation permanently disables this instance and further enrollment. To restore access, create a new instance, unbind the old instance in the client, and use the new instance password.")),
                    h::div().class("xcss-actions").child((h::button().class("xcss-button xscs-danger").disabled(IntoAttributeValue::into_attribute_value(move || busy.get())).on(xcss_web::ev::click, move |_| revoke.run(())).child(t("确认撤销凭据", "Confirm credential revocation")),
                        h::button().class("xcss-button").disabled(IntoAttributeValue::into_attribute_value(move || busy.get())).on(xcss_web::ev::click, move |_| confirm.set(false)).child(t("取消", "Cancel")))),
                )))),
            )),
        )),
        access_editor(device, devices, revision),
        h::section().class("xscs-panel").child((
            h::div().class("xscs-panel-heading").child((h::div().child((h::h2().child(t("诊断任务与结果", "Diagnostic tasks and results")), h::p().child(t("只读诊断用于核对被控端接入状态，不会启动远程连接。", "Read-only diagnostics inspect host integration without starting a remote session.")))),
                h::button().class("xcss-button").disabled(IntoAttributeValue::into_attribute_value(move || busy.get() || !device.with(can_diagnose))).on(xcss_web::ev::click, move |_| diagnose.run(())).child(t("读取主机诊断", "Read host diagnostics")))),
            IntoRender::into_render(move || (!device.with(can_diagnose)).then(|| h::p().class("xscs-hint").child(t("仅已绑定、在线且未撤销的被控端支持读取诊断。", "Diagnostics require an enrolled, online, non-revoked controlled host.")))),
            failure(history_error), h::h3().child(t("最近 50 条任务记录", "Latest 50 tasks")),
            IntoRender::into_render(move || history.get().map(|items| {
                if items.is_empty() { View::new(h::p().class("xscs-hint").child(t("暂无诊断任务。", "No diagnostic tasks yet."))).into_any() }
                else { View::new(h::div().class("xcss-table-scroll").child(h::table().class("xcss-table").child((
                    h::thead().child(h::tr().child([t("时间", "Time"), t("操作", "Action"), t("状态", "State"), t("结果", "Result")].into_iter().map(|v| h::th().child(v)).collect_view())),
                    h::tbody().child(items.into_iter().map(|op| h::tr().child((h::td().child(timestamp(&op.created_at_server)),
                        h::td().child(if op.action.contains("diagnostics") { t("读取主机诊断", "Read host diagnostics") } else { op.action.clone() }), h::td().child(h::span().class("xscs-status").attr("data-state", op.state.as_str()).child(operation_state(op.state))),
                        h::td().child((h::p().child(operation_summary(&op)), h::details().child((h::summary().child(t("原始 JSON", "Raw JSON")), h::pre().child(op.display_result()))))),
                    ))).collect_view()),
                )))).into_any() }
            })),
        )),
    )))
}

fn binding_state(device: &Device) -> String {
    if device.revoked {
        t("已永久撤销", "Permanently revoked")
    } else if device.registered {
        t("已绑定", "Enrolled")
    } else if device.pairing_pending {
        t("等待绑定", "Awaiting enrollment")
    } else {
        t("需要重置实例密码", "Instance password reset required")
    }
}

fn fresh_instance_password() -> Result<String, ApiError> {
    const ALPHABET: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    let crypto = web_sys::window()
        .and_then(|window| window.crypto().ok())
        .ok_or_else(|| ApiError::local("secure_random_unavailable"))?;
    let mut output = String::with_capacity(36);
    for _ in 0..4 {
        let mut bytes = [0u8; 64];
        crypto
            .get_random_values_with_u8_array(&mut bytes)
            .map_err(|_| ApiError::local("secure_random_unavailable"))?;
        for byte in bytes {
            // 252 is the largest multiple of 36 below 256; rejection avoids modulo bias.
            if byte < 252 {
                output.push(ALPHABET[(byte % 36) as usize] as char);
            }
            if output.len() == 36 {
                return Ok(output);
            }
        }
    }
    Err(ApiError::local("secure_random_unavailable"))
}

fn operation_state(state: OperationState) -> String {
    match state {
        OperationState::Pending => t("排队中", "Queued"),
        OperationState::Running => t("执行中", "Running"),
        OperationState::Succeeded => t("成功", "Succeeded"),
        OperationState::Failed => t("失败", "Failed"),
        OperationState::Unknown => t("等待核对", "Awaiting reconciliation"),
        OperationState::DeadLetter => t("需要处理", "Needs attention"),
        OperationState::Resolved => t("已核对", "Resolved"),
    }
}
fn operation_summary(op: &Operation) -> String {
    match op.reconciliation.as_ref().or(op.result.as_ref()) {
        Some(DiagnosticReport::DiagnosticsRead { snapshot, .. }) => format!(
            "{} · {}{}",
            snapshot.integration_state,
            snapshot.backend,
            snapshot
                .rustdesk_version
                .as_ref()
                .map(|v| format!(" · {v}"))
                .unwrap_or_default()
        ),
        Some(
            DiagnosticReport::Rejected { reason, .. } | DiagnosticReport::Unknown { reason, .. },
        ) => reason.clone(),
        None => t("尚无结果", "No result yet"),
    }
}

fn access_editor(
    device: Signal<Device>,
    devices: Signal<Vec<Device>>,
    revision: RwSignal<u64>,
) -> impl IntoView {
    let path = format!("{API}/{}/access", device.with_untracked(|v| v.id.clone()));
    let reload = RwSignal::new(0u64);
    let seen_reload = StoredValue::new(0u64);
    let rules = RwSignal::new(None::<Access>);
    let selected = RwSignal::new(Vec::<String>::new());
    let loading = RwSignal::new(true);
    let dirty = RwSignal::new(false);
    let stale = RwSignal::new(false);
    let saved = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let load_error = RwSignal::new(None);
    let error = RwSignal::new(None::<ApiError>);
    let load_path = path.clone();
    Effect::new(move |_| {
        revision.get();
        let generation = reload.get();
        let explicit = generation != seen_reload.get_value();
        seen_reload.set_value(generation);
        if dirty.get_untracked() && !explicit {
            return;
        }
        let Ok(controller) = web_sys::AbortController::new() else {
            rules.set(None);
            loading.set(false);
            load_error.set(Some(ApiError::local("request_unavailable")));
            return;
        };
        let stop = controller.signal();
        let cleanup = send_wrapper::SendWrapper::new(controller);
        on_cleanup(move || cleanup.abort());
        let path = load_path.clone();
        loading.set(true);
        rules.set(None);
        load_error.set(None);
        xcss_web::task::spawn_local(async move {
            let result = api::request_typed_validated_with_options::<Access>(
                &path,
                "GET",
                None,
                RequestOptions {
                    signal: Some(stop.clone()),
                    ..Default::default()
                },
                |value| crate::contracts::validate(&path, value),
            )
            .await;
            if stop.aborted() {
                return;
            }
            match result {
                Ok(value) => {
                    selected.set(value.peer_ids.clone());
                    rules.set(Some(value));
                    dirty.set(false);
                    stale.set(false);
                    error.set(None);
                }
                Err(value) => load_error.set(Some(value)),
            }
            loading.set(false);
        });
    });
    Effect::new(move |_| {
        if error.get().is_some_and(|value| value.status == 409) {
            stale.set(true);
            saved.set(false);
        }
    });
    let blocked = Signal::derive(move || {
        loading.get()
            || busy.get()
            || rules.get().is_none()
            || load_error.get().is_some()
            || device.with(|v| v.revoked)
    });
    let save = Callback::new(move |()| {
        if blocked.get_untracked() || stale.get_untracked() || selected.get_untracked().len() > 32 {
            return;
        }
        let Some(current) = rules.get_untracked() else {
            return;
        };
        mutation_json::<Access>(
            path.clone(),
            "PUT",
            UpdateAccess {
                peer_ids: selected.get_untracked(),
                expected_revision: current.revision,
            },
            busy,
            error,
            move |value| {
                selected.set(value.peer_ids.clone());
                rules.set(Some(value));
                dirty.set(false);
                stale.set(false);
                saved.set(true);
                revision.update(|v| *v += 1);
            },
        );
    });
    View::new(h::section().class("xscs-panel").aria_label(t("控制权限", "Control permissions")).child((
        h::div().class("xscs-panel-heading").child(h::div().child((h::h2().child(IntoRender::into_render(move || if device.with(|v| v.role) == DeviceRole::Controller { t("允许控制的被控主机", "Hosts this controller may control") } else { t("允许连接的控制端", "Controllers allowed to connect") })),
            h::p().child(t("两侧编辑同一份授权关系。未勾选即不允许，每个实例最多 32 个对端。", "Both sides edit the same permission. Unselected peers are denied; up to 32 peers per instance."))))),
        failure(load_error), failure(error),
        IntoRender::into_render(move || loading.get().then(|| h::p().class("xscs-hint").role("status").child(t("正在读取授权，完成前不能编辑或保存。", "Loading permissions; editing and saving are unavailable until the read succeeds.")))),
        IntoRender::into_render(move || (dirty.get() || stale.get()).then(|| h::p().class("xscs-hint").role("status").child(
            if stale.get() { t("已保留未保存的选择。授权需要重新核对；点击“重新读取授权”后将以服务端结果替换当前选择。", "Unsaved selections are preserved. Reload permissions to reconcile; reloading replaces this draft with server values.") }
            else { t("有未保存的选择，实例刷新不会覆盖。保存时核对授权版本；重新读取授权会以服务端结果替换当前选择。", "Unsaved selections survive instance refreshes. Saving checks the access revision; reloading replaces the draft with server values.") }))),
        h::fieldset().disabled(IntoAttributeValue::into_attribute_value(move || blocked.get())).child((h::legend().child(t("授权对端", "Allowed peers")),
            component_view(&For, component_props_builder(&For).each(move || devices.get().into_iter().filter(|v| v.role != device.with(|v| v.role)).collect::<Vec<_>>()).key(|v: &Device| v.id.clone())
                .children(move |initial: Device| {
                    let id = StoredValue::new(initial.id.clone());
                    let peer = Signal::derive(move || devices.with(|items| items.iter().find(|v| v.id == id.get_value()).cloned()).unwrap_or_else(|| initial.clone()));
                    h::label().class("rustdesk-access-peer xscs-check").child((
                        h::input().r#type("checkbox").prop("checked", move || selected.get().contains(&id.get_value()))
                            .disabled(IntoAttributeValue::into_attribute_value(move || blocked.get() || (!selected.get().contains(&id.get_value()) && (selected.get().len() >= 32 || peer.with(|v| v.revoked)))))
                            .on(xcss_web::ev::change, move |event| {
                                if blocked.get_untracked() { return; }
                                let yes = event_target_checked(&event);
                                if yes && (selected.get_untracked().len() >= 32 || peer.with_untracked(|v| v.revoked)) { return; }
                                selected.update(|items| { items.retain(|value| *value != id.get_value()); if yes { items.push(id.get_value()); } });
                                dirty.set(true); saved.set(false);
                            }),
                        h::span().child(IntoRender::into_render(move || peer.with(|v| v.name.clone()))),
                        h::small().class("xscs-hint").child(IntoRender::into_render(move || peer.with(|v| if v.revoked { t("（已撤销，请移除）", " (revoked; remove)") } else if !v.registered { t("（等待绑定）", " (awaiting enrollment)") } else { String::new() }))),
                    ))
                }).build()),
        )),
        IntoRender::into_render(move || devices.with(|items| !items.iter().any(|v| v.role != device.with(|v| v.role))).then(|| h::p().class("xscs-hint").child(t("暂无可授权的相反角色实例，请先创建并绑定。", "No instances of the opposite role are available. Create and enroll one first.")))),
        h::div().class("xcss-actions").child((
            h::button().class("xcss-button xscs-primary").disabled(IntoAttributeValue::into_attribute_value(move || blocked.get() || stale.get())).on(xcss_web::ev::click, move |_| save.run(())).child(t("保存控制权限", "Save control permissions")),
            h::button().class("xcss-button").disabled(IntoAttributeValue::into_attribute_value(move || busy.get() || loading.get())).on(xcss_web::ev::click, move |_| { saved.set(false); reload.update(|v| *v += 1); }).child(t("重新读取授权", "Reload permissions")),
        )),
        IntoRender::into_render(move || saved.get().then(|| h::p().role("status").child(t("控制权限已保存。在线客户端将在 5 秒内更新。", "Permissions saved. Connected clients update within 5 seconds.")))),
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn device() -> Device {
        Device {
            id: "device-1".into(),
            name: "Office 主机".into(),
            role: DeviceRole::Controlled,
            addresses: vec![],
            registered: true,
            pairing_pending: false,
            revoked: false,
            client_online: true,
            addresses_at_micros: None,
            last_seen_at_micros: None,
            diagnostics: None,
            capabilities: Some(Capabilities {
                os: "windows_x86_64".into(),
            }),
        }
    }
    #[test]
    fn instance_names() {
        assert!(valid_name("实例 1"));
        assert!(!valid_name("\n"));
        assert!(!valid_name(&"a".repeat(33)));
    }
    #[test]
    fn instance_filters_match_name_id_platform_and_role() {
        let v = device();
        assert!(matches_filter(&v, " OFFICE ", "controlled"));
        assert!(matches_filter(&v, "device-1", "all"));
        assert!(matches_filter(&v, "WINDOWS", "all"));
        assert!(!matches_filter(&v, "office", "controller"));
        assert!(!matches_filter(&v, "missing", "all"));
    }
    #[test]
    fn only_a_ready_controlled_host_can_receive_diagnostics() {
        let mut v = device();
        assert!(can_diagnose(&v));
        v.role = DeviceRole::Controller;
        assert!(!can_diagnose(&v));
        v.role = DeviceRole::Controlled;
        v.client_online = false;
        assert!(!can_diagnose(&v));
        v.client_online = true;
        v.registered = false;
        assert!(!can_diagnose(&v));
        v.registered = true;
        v.revoked = true;
        assert!(!can_diagnose(&v));
    }
}
