use xcss_web::component::{component_props_builder, component_view};
pub mod common;
mod contracts;
mod models;
use common::*;
use models::*;
#[cfg(test)]
use serde_json::json;
use xcss_web::prelude::*;
use xcss_web::{
    api::{self, RequestOptions},
    components::{AccountPage, AdminShell},
};
const API: &str = "/api/v1/rustdesk/devices";
#[wasm_bindgen::prelude::wasm_bindgen(start)]
pub fn start() {
    xcss_web::mount("root", || {
        View::new({
            #[allow(unreachable_code)]
            #[allow(unused_mut)]
            #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
            component_view(
                #[allow(clippy::needless_borrows_for_generic_args)]
                &AdminShell,
                {
                    let mut props = component_props_builder(&AdminShell)
                        .product(
                            #[allow(unused_braces)]
                            {
                                "xscs"
                            },
                        )
                        .landing(
                            #[allow(unused_braces)]
                            {
                                "#instances"
                            },
                        )
                        .children({
                            ::xcss_web::children::ToChildren::to_children(move || {
                                #[allow(unreachable_code)]
                                #[allow(unused_mut)]
                                #[allow(
                                    clippy::let_and_return,
                                    clippy::let_unit_value,
                                    clippy::unit_arg
                                )]
                                component_view(
                                    #[allow(clippy::needless_borrows_for_generic_args)]
                                    &Application,
                                    {
                                        let mut props =
                                            component_props_builder(&Application).build();
                                        props
                                    },
                                )
                            })
                        })
                        .build();
                    props
                },
            )
        })
    });
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
#[component]
fn Application() -> impl IntoView {
    let location = route();
    let revision = RwSignal::new(0);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None);
    let new_role = RwSignal::new(DeviceRole::Controlled);
    let ticket = RwSignal::new(None::<Enrollment>);
    let (data, load_error) =
        resource_typed::<Vec<Device>>(Signal::derive(|| API.to_string()), revision, 5000);
    let selected = Signal::derive(move || {
        location
            .get()
            .split('/')
            .nth(1)
            .unwrap_or_default()
            .to_string()
    });
    let create = Callback::new(move |()| {
        mutation_json::<Enrollment>(
            API.into(),
            "POST",
            CreateDevice {
                name: t("新实例", "New instance"),
                role: new_role.get_untracked(),
            },
            busy,
            error,
            move |v| {
                navigate(&format!("details/{}", v.device.id));
                ticket.set(Some(v));
                revision.update(|v| *v += 1);
            },
        )
    });
    View::new(
        xcss_web::html::section()
            .child((
                {
                    #[allow(unreachable_code)]
                    #[allow(unused_mut)]
                    #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
                    component_view(
                        #[allow(clippy::needless_borrows_for_generic_args)]
                        &HeaderActions,
                        {
                            let mut props = component_props_builder(&HeaderActions)
                                .create(
                                    #[allow(unused_braces)]
                                    {
                                        create
                                    },
                                )
                                .revision(
                                    #[allow(unused_braces)]
                                    {
                                        revision
                                    },
                                )
                                .busy(
                                    #[allow(unused_braces)]
                                    {
                                        busy
                                    },
                                )
                                .build();
                            props
                        },
                    )
                },
                {
                    #[allow(unreachable_code)]
                    #[allow(unused_mut)]
                    #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
                    component_view(
                        #[allow(clippy::needless_borrows_for_generic_args)]
                        &PageNav,
                        {
                            let mut props = component_props_builder(&PageNav)
                                .selected(
                                    #[allow(unused_braces)]
                                    {
                                        selected
                                    },
                                )
                                .logs(
                                    #[allow(unused_braces)]
                                    {
                                        false
                                    },
                                )
                                .build();
                            props
                        },
                    )
                },
                xcss_web::html::h1().child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            move || t("RustDesk 设备管理", "RustDesk device management")
                        })
                    },
                ),
                xcss_web::html::label().child((
                    IntoRender::into_render(move || t("新实例角色", "New instance role")),
                    xcss_web::html::select()
                        .child((
                            xcss_web::html::option()
                                .child(
                                    #[allow(unused_braces)]
                                    {
                                        IntoRender::into_render({
                                            move || t("被控端", "Controlled host")
                                        })
                                    },
                                )
                                .value("controlled"),
                            xcss_web::html::option()
                                .child(
                                    #[allow(unused_braces)]
                                    {
                                        IntoRender::into_render({
                                            move || t("控制端", "Controller")
                                        })
                                    },
                                )
                                .value("controller"),
                        ))
                        .class("xcss-input")
                        .prop("value", move || {
                            if new_role.get() == DeviceRole::Controller {
                                "controller"
                            } else {
                                "controlled"
                            }
                        })
                        .on(xcss_web::ev::change, move |ev| {
                            new_role.set(if event_target_value(&ev) == "controller" {
                                DeviceRole::Controller
                            } else {
                                DeviceRole::Controlled
                            })
                        })
                        .disabled(::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                            move || busy.get(),
                        )),
                )),
                {
                    #[allow(unreachable_code)]
                    #[allow(unused_mut)]
                    #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
                    component_view(
                        #[allow(clippy::needless_borrows_for_generic_args)]
                        &Failure,
                        {
                            let mut props = component_props_builder(&Failure)
                                .error(
                                    #[allow(unused_braces)]
                                    {
                                        load_error
                                    },
                                )
                                .message(
                                    #[allow(unused_braces)]
                                    {
                                        t("无法读取管理状态", "Unable to load management state")
                                    },
                                )
                                .build();
                            props
                        },
                    )
                },
                {
                    #[allow(unreachable_code)]
                    #[allow(unused_mut)]
                    #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
                    component_view(
                        #[allow(clippy::needless_borrows_for_generic_args)]
                        &Failure,
                        {
                            let mut props = component_props_builder(&Failure)
                                .error(
                                    #[allow(unused_braces)]
                                    {
                                        error
                                    },
                                )
                                .build();
                            props
                        },
                    )
                },
                IntoRender::into_render({
                    move || {
                        if location.get() == "account" {
                            View::new({
                                #[allow(unreachable_code)]
                                #[allow(unused_mut)]
                                #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
                                component_view(
                                    #[allow(clippy::needless_borrows_for_generic_args)]
                                    &AccountPage,
                                    {
                                        let mut props =
                                            component_props_builder(&AccountPage).build();
                                        props
                                    },
                                )
                            })
                            .into_any()
                        } else if location.get().starts_with("details/") {
                            View::new({
                                #[allow(unreachable_code)]
                                #[allow(unused_mut)]
                                #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
                                component_view(
                                    #[allow(clippy::needless_borrows_for_generic_args)]
                                    &For,
                                    {
                                        let mut props = component_props_builder(&For)
                .each(
                    #[allow(unused_braces)]
                    {
                        {
                            move || {
                                data
                                    .get()
                                    .unwrap_or_default()
                                    .into_iter()
                                    .filter(|d| d.id.clone() == selected.get())
                                    .collect::<Vec<_>>()
                            }
                        }
                    },
                )
                .key(#[allow(unused_braces)] { |d| d.id.clone() })
                .children(
                    #[allow(unused_braces)]
                    {
                        move |initial| {
                            let id = initial.id.clone();
                            let device = Signal::derive(move || {
                                data
                                    .with(|v| {
                                        v
                                            .as_ref()
                                            .and_then(|v| v.iter().find(|d| d.id == id).cloned())
                                    })
                                    .unwrap_or_else(|| initial.clone())
                            });
                            let devices = Signal::derive(move || {
                                data.get().unwrap_or_default()
                            });
                            View::new({
    #[allow(unreachable_code)] #[allow(unused_mut)] #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
    component_view(
        #[allow(clippy::needless_borrows_for_generic_args)]
        &Workspace,
        {
            let mut props = component_props_builder(&Workspace)
                .device(#[allow(unused_braces)] { device })
                .devices(#[allow(unused_braces)] { devices })
                .revision(#[allow(unused_braces)] { revision })
                .ticket(#[allow(unused_braces)] { ticket })
                .build();
            props
        },
    )
})
                        }
                    },
                )
                .build();
                                        props
                                    },
                                )
                            })
                            .into_any()
                        } else {
                            View::new(xcss_web::html::div().child(
                                #[allow(unused_braces)]
                                {
                                    IntoRender::into_render({
                                        move || {
                                            data
                            .get()
                            .map(|v| {
                                View::new({
    #[allow(unreachable_code)] #[allow(unused_mut)] #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
    component_view(
        #[allow(clippy::needless_borrows_for_generic_args)]
        &Instances,
        {
            let mut props = component_props_builder(&Instances)
                .devices(#[allow(unused_braces)] { v })
                .revision(#[allow(unused_braces)] { revision })
                .build();
            props
        },
    )
})
                            })
                                        }
                                    })
                                },
                            ))
                            .into_any()
                        }
                    }
                }),
            ))
            .class("xcss-content-stack"),
    )
}
#[component]
fn Instances(devices: Vec<Device>, revision: RwSignal<u64>) -> impl IntoView {
    let candidate = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None);
    if devices.is_empty() {
        return View::new(xcss_web::html::p().child(
            #[allow(unused_braces)]
            {
                IntoRender::into_render({
                    t(
                        "暂无实例。新建实例后，由 RustDesk 内嵌客户端绑定。",
                        "No instances. Create one for the embedded RustDesk client to enroll.",
                    )
                })
            },
        ))
        .into_any();
    }
    View::new((
        {
            #[allow(unreachable_code)]
            #[allow(unused_mut)]
            #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
            component_view(
                #[allow(clippy::needless_borrows_for_generic_args)]
                &Failure,
                {
                    let mut props = component_props_builder(&Failure)
                        .error(
                            #[allow(unused_braces)]
                            {
                                error
                            },
                        )
                        .message(
                            #[allow(unused_braces)]
                            {
                                t(
                                    "删除未能确认，请刷新核对。",
                                    "Deletion could not be confirmed. Refresh to check.",
                                )
                            },
                        )
                        .build();
                    props
                },
            )
        },
        xcss_web::html::table()
            .child((
                xcss_web::html::thead().child(
                    #[allow(unused_braces)]
                    {
                        xcss_web::html::tr().child(
                            #[allow(unused_braces)]
                            {
                                IntoRender::into_render({
                                    [
                                        t("名称", "Name"),
                                        t("角色", "Role"),
                                        t("管理连接", "Management connection"),
                                        t("RustDesk 状态", "RustDesk status"),
                                        t("平台", "Platform"),
                                        t("最近连接", "Last seen"),
                                        t("操作", "Actions"),
                                    ]
                                    .into_iter()
                                    .map(|h| {
                                        View::new(xcss_web::html::th().child(
                                            #[allow(unused_braces)]
                                            {
                                                IntoRender::into_render({ h })
                                            },
                                        ))
                                    })
                                    .collect_view()
                                })
                            },
                        )
                    },
                ),
                xcss_web::html::tbody().child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            devices
                                .into_iter()
                                .map(|d| {
                                    let id = d.id.clone();
                                    let id2 = id.clone();
                                    let id3 = id.clone();
                                    let id4 = id.clone();
                                    let last = d
                                        .last_seen_at_micros
                                        .as_ref()
                                        .and_then(serde_json::Number::as_f64)
                                        .map(|m| {
                                            let date = js_sys::Date::new(
                                                &wasm_bindgen::JsValue::from_f64(m / 1000.),
                                            );
                                            timestamp(
                                                &date
                                                    .to_iso_string()
                                                    .as_string()
                                                    .unwrap_or_default(),
                                            )
                                        })
                                        .unwrap_or("—".into());
                                    View::new(xcss_web::html::tr().child((
                                        xcss_web::html::th().child(
                                            #[allow(unused_braces)]
                                            {
                                                xcss_web::html::a()
                            .child(
                                #[allow(unused_braces)]
                                {
                                    IntoRender::into_render({
                                        d.name.clone()
                                    })
                                },
                            )
                            .href(
                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                                    format!("#details/{id}"),
                                ),
                            )
                                            },
                                        ),
                                        xcss_web::html::td().child(
                                            #[allow(unused_braces)]
                                            {
                                                IntoRender::into_render({ role(&d) })
                                            },
                                        ),
                                        xcss_web::html::td().child(
                                            #[allow(unused_braces)]
                                            {
                                                IntoRender::into_render({
                                                    if d.revoked {
                                                        t("已撤销", "Revoked")
                                                    } else if d.client_online {
                                                        t("在线", "Online")
                                                    } else if d.registered {
                                                        t("离线", "Offline")
                                                    } else {
                                                        t("等待绑定", "Awaiting enrollment")
                                                    }
                                                })
                                            },
                                        ),
                                        xcss_web::html::td().child(
                                            #[allow(unused_braces)]
                                            {
                                                IntoRender::into_render({ integration(&d) })
                                            },
                                        ),
                                        xcss_web::html::td().child(
                                            #[allow(unused_braces)]
                                            {
                                                IntoRender::into_render({
                                                    d.capabilities
                                                        .as_ref()
                                                        .map(|c| c.os.clone())
                                                        .unwrap_or("—".into())
                                                })
                                            },
                                        ),
                                        xcss_web::html::td().child(
                                            #[allow(unused_braces)]
                                            {
                                                IntoRender::into_render({ last })
                                            },
                                        ),
                                        xcss_web::html::td().child(
                                            #[allow(unused_braces)]
                                            {
                                                IntoRender::into_render({
                                                    move || {
                                                        if candidate.get() == id2 {
                                                            View::new(
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
                .disabled(
                    ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                    busy.get()),
                )
                .on(
                    xcss_web::ev::click,
                    move |_| candidate.set(String::new()),
                ),
            xcss_web::html::button()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            t("确认删除", "Confirm delete")
                        })
                    },
                )
                .class("xcss-button")
                .disabled(
                    ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                    busy.get()),
                )
                .on(
                    xcss_web::ev::click,
                    {
                        let id = id3.clone();
                        move |_| {
                            let path = format!("{API}/{id}");
                            mutation(
                                path,
                                "DELETE",
                                None,
                                busy,
                                error,
                                move |_| {
                                    candidate.set(String::new());
                                    revision.update(|v| *v += 1)
                                },
                            )
                        }
                    },
                ),
        ))
        .class("xcss-actions"),
)
                                        .into_any()
                                                        } else {
                                                            View::new(
    xcss_web::html::button()
        .child(
            #[allow(unused_braces)]
            { IntoRender::into_render({ t("删除", "Delete") }) },
        )
        .class("xcss-button")
        .disabled(
            ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move || {
                busy.get()
            }),
        )
        .on(
            xcss_web::ev::click,
            {
                let id = id4.clone();
                move |_| candidate.set(id.clone())
            },
        ),
)
                                        .into_any()
                                                        }
                                                    }
                                                })
                                            },
                                        ),
                                    )))
                                })
                                .collect_view()
                        })
                    },
                ),
            ))
            .class("xcss-table")
            .aria_label(
                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(t(
                    "RustDesk 实例",
                    "RustDesk instances",
                )),
            ),
    ))
    .into_any()
}
#[component]
fn Workspace(
    device: Signal<Device>,
    devices: Signal<Vec<Device>>,
    revision: RwSignal<u64>,
    ticket: RwSignal<Option<Enrollment>>,
) -> impl IntoView {
    let id = device.get_untracked().id;
    let path = format!("{API}/{id}");
    let name = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None);
    let local = RwSignal::new(0);
    let authorization = RwSignal::new(
        ticket
            .get_untracked()
            .filter(|ticket| ticket.device.id == id)
            .map(|ticket| ticket.token)
            .unwrap_or_default(),
    );
    let confirm = RwSignal::new(false);
    let diagnostic_key = RwSignal::new(String::new());
    let history_path = format!("{path}/tasks?recent=50");
    let (history, history_error) =
        resource_typed::<Vec<Operation>>(Signal::derive(move || history_path.clone()), local, 5000);
    let revoked = Signal::derive(move || device.with(|d| d.revoked));
    let registered = Signal::derive(move || device.with(|d| d.registered));
    let rename_path = path.clone();
    let auth_path = path.clone();
    let revoke_path = path.clone();
    let task_path = path.clone();
    View::new(
    xcss_web::html::section()
        .child((
            xcss_web::html::h2()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            move || device.with(|d| d.name.clone())
                        })
                    },
                ),
            xcss_web::html::p()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            move || device.with(integration)
                        })
                    },
                ),
            xcss_web::html::p()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            t(
                                "实例角色和控制权限由服务端管理，地址由被控端上报。在原生客户端绑定实例后，从授权列表发起连接。",
                                "The server manages instance roles and permissions; controlled hosts report their addresses. Bind an instance in the native client and connect from its allowed hosts.",
                            )
                        })
                    },
                ),
            xcss_web::html::form()
                .child((
                    xcss_web::html::label()
                        .child((
                            IntoRender::into_render({
                                t("实例名称", "Instance name")
                            }),
                            xcss_web::html::input()
                                .class("xcss-input")
                                .prop(
                                    "value",
                                    move || {
                                        name
                                            .get()
                                            .unwrap_or_else(|| device.with(|d| d.name.clone()))
                                    },
                                )
                                .maxlength("32")
                                .required(true)
                                .on(
                                    xcss_web::ev::input,
                                    move |ev| name.set(Some(event_target_value(&ev))),
                                ),
                        )),
                    xcss_web::html::button()
                        .child(
                            #[allow(unused_braces)]
                            {
                                IntoRender::into_render({
                                    t("保存名称", "Save name")
                                })
                            },
                        )
                        .class("xcss-button")
                        .r#type("submit")
                        .disabled(
                            ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                            {
                                busy.get()
                                    || !valid_name(
                                        &name
                                            .get()
                                            .unwrap_or_else(|| device.with(|d| d.name.clone())),
                                    )
                            }),
                        ),
                ))
                .class("xcss-actions")
                .on(
                    xcss_web::ev::submit,
                    move |ev| {
                        ev.prevent_default();
                        mutation_json::<
                            Device,
                        >(
                            rename_path.clone(),
                            "PATCH",
                            RenameDevice {
                                name: name
                                    .get_untracked()
                                    .unwrap_or_else(|| {
                                        device.with_untracked(|d| d.name.clone())
                                    }),
                            },
                            busy,
                            error,
                            move |_| {
                                name.set(None);
                                revision.update(|v| *v += 1)
                            },
                        )
                    },
                ),
            xcss_web::html::div()
                .child((
                    xcss_web::html::button()
                        .child(
                            #[allow(unused_braces)]
                            {
                                IntoRender::into_render({
                                    move || {
                                        if authorization.get().is_empty() {
                                            t("显示授权码", "Show enrollment code")
                                        } else {
                                            t("隐藏授权码", "Hide enrollment code")
                                        }
                                    }
                                })
                            },
                        )
                        .class("xcss-button")
                        .disabled(
                            ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                            busy.get() || revoked.get()),
                        )
                        .on(
                            xcss_web::ev::click,
                            move |_| {
                                if !authorization.get_untracked().is_empty() {
                                    authorization.set(String::new());
                                } else {
                                    mutation_typed::<
                                        Authorization,
                                    >(
                                        format!("{auth_path}/authorization"),
                                        "GET",
                                        None,
                                        busy,
                                        error,
                                        move |v| authorization.set(v.authorization_code),
                                    )
                                }
                            },
                        ),
                    {
                        #[allow(unreachable_code)] #[allow(unused_mut)]
                        #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
                        component_view(
                            #[allow(clippy::needless_borrows_for_generic_args)]
                            &Show,
                            {
                                let mut props = component_props_builder(
                                        &Show,
                                    )
                                    .when(#[allow(unused_braces)] { move || !revoked.get() })
                                    .children({
                                        ::xcss_web::children::ToChildren::to_children(move || IntoRender::into_render({
                                            View::new(
    xcss_web::html::button()
        .child(
            #[allow(unused_braces)]
            {
                IntoRender::into_render({
                    t("撤销客户端凭据", "Revoke client credentials")
                })
            },
        )
        .class("xcss-button")
        .disabled(
            ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move || {
                busy.get()
            }),
        )
        .on(xcss_web::ev::click, move |_| confirm.set(true)),
)
                                        }))
                                    })
                                    .build();
                                props
                            },
                        )
                    },
                    IntoRender::into_render({
                        move || {
                            confirm
                                .get()
                                .then(|| {
                                    let p = revoke_path.clone();
                                    View::new((
    xcss_web::html::button()
        .child(
            #[allow(unused_braces)]
            {
                IntoRender::into_render({
                    t("确认撤销凭据", "Confirm credential revocation")
                })
            },
        )
        .class("xcss-button")
        .disabled(
            ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move || {
                busy.get()
            }),
        )
        .on(
            xcss_web::ev::click,
            move |_| mutation(
                format!("{p}/revoke"),
                "POST",
                None,
                busy,
                error,
                move |_| {
                    authorization.set(String::new());
                    confirm.set(false);
                    revision.update(|v| *v += 1)
                },
            ),
        ),
    xcss_web::html::button()
        .child(
            #[allow(unused_braces)]
            { IntoRender::into_render({ t("取消", "Cancel") }) },
        )
        .class("xcss-button")
        .on(xcss_web::ev::click, move |_| confirm.set(false)),
))
                                })
                        }
                    }),
                ))
                .class("xcss-actions"),
            IntoRender::into_render({
                move || {
                    (!authorization.get().is_empty())
                        .then(|| {
                            View::new(
    xcss_web::html::div()
        .child((
            xcss_web::html::p()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            t("实例授权码：", "Instance enrollment code:")
                        })
                    },
                ),
            xcss_web::html::code()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            authorization.get()
                        })
                    },
                ),
        ))
        .role("status"),
)
                        })
                }
            }),
            xcss_web::html::dl()
                .child((
                    xcss_web::html::dt()
                        .child(
                            #[allow(unused_braces)]
                            {
                                IntoRender::into_render({
                                    t("角色", "Role")
                                })
                            },
                        ),
                    xcss_web::html::dd()
                        .child(
                            #[allow(unused_braces)]
                            {
                                IntoRender::into_render({
                                    move || device.with(role)
                                })
                            },
                        ),
                    xcss_web::html::dt()
                        .child(
                            #[allow(unused_braces)]
                            {
                                IntoRender::into_render({
                                    t("运行方式", "Execution mode")
                                })
                            },
                        ),
                    xcss_web::html::dd()
                        .child(
                            #[allow(unused_braces)]
                            {
                                IntoRender::into_render({
                                    t("RustDesk 进程内嵌入", "Embedded in RustDesk")
                                })
                            },
                        ),
                    xcss_web::html::dt()
                        .child(
                            #[allow(unused_braces)]
                            {
                                IntoRender::into_render({
                                    t("RustDesk 版本", "RustDesk version")
                                })
                            },
                        ),
                    xcss_web::html::dd()
                        .child(
                            #[allow(unused_braces)]
                            {
                                IntoRender::into_render({
                                    move || {
                                        device
                                            .with(|d| {
                                                d
                                                    .diagnostics
                                                    .as_ref()
                                                    .and_then(|d| d.rustdesk_version.clone())
                                            })
                                            .unwrap_or("—".into())
                                    }
                                })
                            },
                        ),
                )),
            IntoRender::into_render({
                (device.with_untracked(|d| d.role) == DeviceRole::Controlled)
                    .then(|| {
                        View::new(
    xcss_web::html::div()
        .child((
            xcss_web::html::h3()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            t("被控端上报地址", "Reported host addresses")
                        })
                    },
                ),
            xcss_web::html::ul()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            move || {
                                device
                                    .with(|d| d.addresses.clone())
                                    .into_iter()
                                    .map(|a| {
                                        let host = a.host;
                                        let host = if host.contains(':') {
                                            format!("[{host}]")
                                        } else {
                                            host
                                        };
                                        View::new(
    xcss_web::html::li()
        .child(
            #[allow(unused_braces)]
            {
                xcss_web::html::code()
                    .child(
                        #[allow(unused_braces)]
                        {
                            IntoRender::into_render({
                                format!("{host}:{}", a.port)
                            })
                        },
                    )
            },
        ),
)
                                    })
                                    .collect_view()
                            }
                        })
                    },
                ),
            xcss_web::html::p()
                .child((
                    IntoRender::into_render({
                        t("最近上报：", "Last report: ")
                    }),
                    IntoRender::into_render({
                        move || {
                            device
                                .get()
                                .addresses_at_micros
                                .as_ref()
                                .and_then(serde_json::Number::as_f64)
                                .map(|m| {
                                    js_sys::Date::new(&m.div_euclid(1000.).into())
                                        .to_iso_string()
                                        .as_string()
                                        .unwrap_or_default()
                                })
                                .unwrap_or("—".into())
                        }
                    }),
                )),
            xcss_web::html::p()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            t(
                                "离线或超过 45 秒的地址不会下发给控制端。",
                                "Addresses are omitted from controller lists when offline or older than 45 seconds.",
                            )
                        })
                    },
                ),
        )),
)
                    })
            }),
            {
                #[allow(unreachable_code)] #[allow(unused_mut)]
                #[allow(clippy::let_and_return, clippy::let_unit_value, clippy::unit_arg)]
                component_view(
                    #[allow(clippy::needless_borrows_for_generic_args)]
                    &AccessEditor,
                    {
                        let mut props = component_props_builder(
                                &AccessEditor,
                            )
                            .device(#[allow(unused_braces)] { device })
                            .devices(#[allow(unused_braces)] { devices })
                            .revision(#[allow(unused_braces)] { revision })
                            .build();
                        props
                    },
                )
            },
            xcss_web::html::div()
                .child(
                    #[allow(unused_braces)]
                    {
                        xcss_web::html::button()
                            .child(
                                #[allow(unused_braces)]
                                {
                                    IntoRender::into_render({
                                        t("读取主机诊断", "Read host diagnostics")
                                    })
                                },
                            )
                            .class("xcss-button")
                            .disabled(
                                ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                                busy.get() || !registered.get() || revoked.get()),
                            )
                            .on(
                                xcss_web::ev::click,
                                move |_| {
                                    if busy.get_untracked() {
                                        return;
                                    }
                                    busy.set(true);
                                    error.set(None);
                                    let p = format!("{task_path}/tasks");
                                    if diagnostic_key.get_untracked().is_empty() {
                                        diagnostic_key
                                            .set(
                                                web_sys::window().unwrap().crypto().unwrap().random_uuid(),
                                            );
                                    }
                                    let key = diagnostic_key.get_untracked();
                                    let controller = web_sys::AbortController::new().unwrap();
                                    let stop = controller.signal();
                                    let cleanup = send_wrapper::SendWrapper::new(controller);
                                    on_cleanup(move || cleanup.abort());
                                    let body = match api::encode_json_body(
                                        &DeviceCommand::ReadDiagnostics,
                                    ) {
                                        Ok(body) => body,
                                        Err(e) => {
                                            error.set(Some(e));
                                            busy.set(false);
                                            return;
                                        }
                                    };
                                    xcss_web::task::spawn_local(async move {
                                        let result = api::request_typed_validated_with_options::<
                                            Operation,
                                        >(
                                                &p,
                                                "POST",
                                                Some(body),
                                                RequestOptions {
                                                    signal: Some(stop.clone()),
                                                    headers: vec![("Idempotency-Key".into(), key)],
                                                    ..Default::default()
                                                },
                                                |v| crate::contracts::validate(&p, v),
                                            )
                                            .await;
                                        if stop.aborted() {
                                            return;
                                        }
                                        match result {
                                            Ok(_) => {
                                                diagnostic_key.set(String::new());
                                                local.update(|v| *v += 1);
                                                revision.update(|v| *v += 1)
                                            }
                                            Err(e) => error.set(Some(e)),
                                        }
                                        busy.set(false);
                                    });
                                },
                            )
                    },
                )
                .class("xcss-actions"),
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
                            .build();
                        props
                    },
                )
            },
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
                            .error(#[allow(unused_braces)] { history_error })
                            .build();
                        props
                    },
                )
            },
            xcss_web::html::h3()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            t("最近 50 条任务记录", "Latest 50 tasks")
                        })
                    },
                ),
            IntoRender::into_render({
                move || {
                    history
                        .get()
                        .map(|h| {
                            View::new(
    xcss_web::html::table()
        .child((
            xcss_web::html::thead()
                .child(
                    #[allow(unused_braces)]
                    {
                        xcss_web::html::tr()
                            .child((
                                xcss_web::html::th()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({
                                                t("时间", "Time")
                                            })
                                        },
                                    ),
                                xcss_web::html::th()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({
                                                t("操作", "Action")
                                            })
                                        },
                                    ),
                                xcss_web::html::th()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({
                                                t("状态", "State")
                                            })
                                        },
                                    ),
                                xcss_web::html::th()
                                    .child(
                                        #[allow(unused_braces)]
                                        {
                                            IntoRender::into_render({
                                                t("结果", "Result")
                                            })
                                        },
                                    ),
                            ))
                    },
                ),
            xcss_web::html::tbody()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            h.into_iter()
                                .map(|op| {
                                    View::new(
    xcss_web::html::tr()
        .child((
            xcss_web::html::td()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            op.created_at_server.clone()
                        })
                    },
                ),
            xcss_web::html::td()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            op.action.clone()
                        })
                    },
                ),
            xcss_web::html::td()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            op.state.as_str()
                        })
                    },
                ),
            xcss_web::html::td()
                .child(
                    #[allow(unused_braces)]
                    {
                        xcss_web::html::pre()
                            .child(
                                #[allow(unused_braces)]
                                {
                                    IntoRender::into_render({
                                        op.display_result()
                                    })
                                },
                            )
                    },
                ),
        )),
)
                                })
                                .collect_view()
                        })
                    },
                ),
        ))
        .class("xcss-table"),
)
                        })
                }
            }),
        ))
        .class("xcss-content-stack"),
)
}
#[component]
fn AccessEditor(
    device: Signal<Device>,
    devices: Signal<Vec<Device>>,
    revision: RwSignal<u64>,
) -> impl IntoView {
    let id = device.get_untracked().id;
    let local = RwSignal::new(0);
    let path = format!("{API}/{id}/access");
    let p = path.clone();
    let (rules, load_error) = resource_typed::<Access>(Signal::derive(move || p.clone()), local, 0);
    let selected = RwSignal::new(Vec::<String>::new());
    let saved = RwSignal::new(false);
    let busy = RwSignal::new(false);
    let error = RwSignal::new(None);
    Effect::new(move |_| {
        if let Some(r) = rules.get() {
            selected.set(r.peer_ids);
        }
    });
    let revoked = Signal::derive(move || device.with(|d| d.revoked));
    View::new(
    xcss_web::html::section()
        .child((
            xcss_web::html::h3()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            if device.with_untracked(|d| d.role)
                                == DeviceRole::Controller
                            {
                                t(
                                    "允许控制的被控主机",
                                    "Hosts this controller may control",
                                )
                            } else {
                                t(
                                    "允许连接的控制端",
                                    "Controllers allowed to connect",
                                )
                            }
                        })
                    },
                ),
            xcss_web::html::p()
                .child(
                    #[allow(unused_braces)]
                    {
                        IntoRender::into_render({
                            t(
                                "两侧编辑同一份授权关系。未勾选即不允许，每个实例最多 32 个对端。",
                                "Both sides edit the same permission. Unselected peers are denied; up to 32 peers per instance.",
                            )
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
                            .error(#[allow(unused_braces)] { load_error })
                            .build();
                        props
                    },
                )
            },
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
                            .build();
                        props
                    },
                )
            },
            xcss_web::html::fieldset()
                .child((
                    xcss_web::html::legend()
                        .child(
                            #[allow(unused_braces)]
                            {
                                IntoRender::into_render({
                                    t("授权对端", "Allowed peers")
                                })
                            },
                        ),
                    IntoRender::into_render({
                        move || {
                            devices
                                .get()
                                .into_iter()
                                .filter(|v| v.role != device.with(|d| d.role))
                                .map(|peer| {
                                    let id = peer.id.clone();
                                    let i2 = id.clone();
                                    let i3 = id.clone();
                                    let is_revoked = peer.revoked;
                                    View::new(
    xcss_web::html::label()
        .child((
            xcss_web::html::input()
                .r#type("checkbox")
                .prop("checked", move || selected.get().contains(&id))
                .disabled(
                    ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                    {
                        !selected.get().contains(&i2)
                            && (selected.get().len() >= 32 || is_revoked)
                    }),
                )
                .on(
                    xcss_web::ev::change,
                    move |ev| {
                        let yes = event_target_checked(&ev);
                        selected
                            .update(|v| {
                                v.retain(|i| i != &i3);
                                if yes {
                                    v.push(i3.clone());
                                }
                            });
                        saved.set(false);
                    },
                ),
            IntoRender::into_render(peer.name.clone()),
            IntoRender::into_render({
                if is_revoked {
                    t("（已撤销，请移除）", " (revoked; remove)")
                } else if !peer.registered {
                    t("（等待绑定）", " (awaiting enrollment)")
                } else {
                    String::new()
                }
            }),
        ))
        .class("rustdesk-access-peer"),
)
                                })
                                .collect_view()
                        }
                    }),
                ))
                .disabled(
                    ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                    busy.get() || revoked.get()),
                ),
            xcss_web::html::div()
                .child((
                    xcss_web::html::button()
                        .child(
                            #[allow(unused_braces)]
                            {
                                IntoRender::into_render({
                                    t("保存控制权限", "Save control permissions")
                                })
                            },
                        )
                        .class("xcss-button")
                        .disabled(
                            ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                            busy.get() || rules.get().is_none() || revoked.get()),
                        )
                        .on(
                            xcss_web::ev::click,
                            move |_| {
                                let body = UpdateAccess {
                                    peer_ids: selected.get_untracked(),
                                    expected_revision: rules
                                        .get_untracked()
                                        .map(|v| v.revision)
                                        .unwrap_or_default(),
                                };
                                mutation_json::<
                                    Access,
                                >(
                                    path.clone(),
                                    "PUT",
                                    body,
                                    busy,
                                    error,
                                    move |v| {
                                        rules.set(Some(v));
                                        saved.set(true);
                                        revision.update(|v| *v += 1);
                                    },
                                )
                            },
                        ),
                    xcss_web::html::button()
                        .child(
                            #[allow(unused_braces)]
                            {
                                IntoRender::into_render({
                                    t("重新读取授权", "Reload permissions")
                                })
                            },
                        )
                        .class("xcss-button")
                        .disabled(
                            ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(move ||
                            busy.get()),
                        )
                        .on(
                            xcss_web::ev::click,
                            move |_| local.update(|v| *v += 1),
                        ),
                ))
                .class("xcss-actions"),
            IntoRender::into_render({
                move || {
                    saved
                        .get()
                        .then(|| {
                            View::new(
    xcss_web::html::p()
        .child(
            #[allow(unused_braces)]
            {
                IntoRender::into_render({
                    t(
                        "控制权限已保存。在线客户端将在 5 秒内更新。",
                        "Permissions saved. Connected clients update within 5 seconds.",
                    )
                })
            },
        )
        .role("status"),
)
                        })
                }
            }),
        ))
        .class("xcss-content-stack")
        .aria_label(
            ::xcss_web::prelude::IntoAttributeValue::into_attribute_value(
                t("控制权限", "Control permissions"),
            ),
        ),
)
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
