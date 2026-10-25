//! Application-owned single choice, adapted from isarmg/zxs
//! src/interface/src/components/select.rs (Apache-2.0 project; interface package
//! declares Apache-2.0 AND MIT). Views use this repository's ordinary Rust builders.
use std::sync::atomic::{AtomicUsize, Ordering};
use wasm_bindgen::{JsCast, closure::Closure};
use xcss_web::{html as h, prelude::*};

use crate::common::t;

static SEQUENCE: AtomicUsize = AtomicUsize::new(1);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectOption {
    pub value: String,
    pub label: String,
    pub disabled: bool,
}
impl SelectOption {
    pub fn new(value: &str, label: String) -> Self {
        Self {
            value: value.into(),
            label,
            disabled: false,
        }
    }
}

fn next_enabled(options: &[SelectOption], current: Option<usize>, forward: bool) -> Option<usize> {
    let length = options.len();
    if length == 0 {
        return None;
    }
    let start = current.filter(|index| *index < length);
    (1..=length)
        .map(|distance| match (start, forward) {
            (Some(index), true) => (index + distance) % length,
            (Some(index), false) => (index + length - distance % length) % length,
            (None, true) => distance - 1,
            (None, false) => length - distance,
        })
        .find(|index| !options[*index].disabled)
}

fn position(trigger: web_sys::HtmlElement, panel: web_sys::HtmlElement) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let bounds = trigger.get_bounding_client_rect();
    let width = window
        .inner_width()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(320.0);
    let height = window
        .inner_height()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(640.0);
    let panel_width = bounds.width().max(180.0).min((width - 16.0).max(0.0));
    let left = bounds.left().min(width - panel_width - 8.0).max(8.0);
    let below = height - bounds.bottom() - 12.0;
    let above = bounds.top() - 12.0;
    let style = panel.style();
    let _ = style.set_property("width", &format!("{panel_width}px"));
    let _ = style.set_property("left", &format!("{left}px"));
    if below >= 180.0 || below >= above {
        let _ = style.set_property("max-height", &format!("{}px", below.clamp(44.0, 320.0)));
        let _ = style.set_property("top", &format!("{}px", bounds.bottom() + 4.0));
        let _ = style.set_property("bottom", "auto");
    } else {
        let _ = style.set_property("max-height", &format!("{}px", above.clamp(44.0, 320.0)));
        let _ = style.set_property("top", "auto");
        let _ = style.set_property("bottom", &format!("{}px", height - bounds.top() + 4.0));
    }
}

fn listen(name: &'static str, capture: bool, callback: impl FnMut(web_sys::Event) + 'static) {
    let Some(window) = web_sys::window() else {
        return;
    };
    let callback = Closure::<dyn FnMut(web_sys::Event)>::new(callback);
    if let Err(error) = window.add_event_listener_with_callback_and_bool(
        name,
        callback.as_ref().unchecked_ref(),
        capture,
    ) {
        web_sys::console::error_1(&error);
        return;
    }
    let listener = send_wrapper::SendWrapper::new((window, callback));
    on_cleanup(move || {
        let (window, callback) = &*listener;
        let _ = window.remove_event_listener_with_callback_and_bool(
            name,
            callback.as_ref().unchecked_ref(),
            capture,
        );
    });
}

#[component]
pub fn Select(
    label: String,
    options: Signal<Vec<SelectOption>>,
    value: Signal<String>,
    on_change: Callback<String>,
    #[prop(default=Signal::derive(|| false))] disabled: Signal<bool>,
) -> impl IntoView {
    let id = format!("xscs-select-{}", SEQUENCE.fetch_add(1, Ordering::Relaxed));
    let control_id = StoredValue::new(id.clone());
    let list_id = StoredValue::new(format!("{id}-list"));
    let label_id = StoredValue::new(format!("{id}-label"));
    let root = NodeRef::<h::Div>::new();
    let trigger = NodeRef::<h::Button>::new();
    let panel = NodeRef::<h::Div>::new();
    let open = RwSignal::new(false);
    let active = RwSignal::new(None::<usize>);
    let prefix = StoredValue::new((String::new(), 0.0f64));
    let close = Callback::new(move |restore_focus: bool| {
        open.set(false);
        if restore_focus && let Some(trigger) = trigger.get_untracked() {
            let _ = trigger.focus();
        }
    });
    let choose = Callback::new(move |index: usize| {
        if disabled.get_untracked() {
            return;
        }
        if let Some(option) = options
            .get_untracked()
            .get(index)
            .filter(|option| !option.disabled)
        {
            let value = option.value.clone();
            close.run(true);
            on_change.run(value);
        }
    });
    let expand = Callback::new(move |()| {
        if disabled.get_untracked() {
            return;
        }
        let entries = options.get_untracked();
        let selected = value.get_untracked();
        active.set(
            entries
                .iter()
                .position(|item| item.value == selected && !item.disabled)
                .or_else(|| next_enabled(&entries, None, true)),
        );
        open.set(true);
    });
    Effect::new(move |_| {
        let Some(panel) = panel.get() else {
            return;
        };
        if open.get() && !disabled.get() {
            if let Some(trigger) = trigger.get() {
                position(trigger.unchecked_into(), panel.clone().unchecked_into());
                if panel.show_popover().is_err() {
                    open.set(false);
                }
            }
        } else {
            let _ = panel.hide_popover();
        }
    });
    Effect::new(move |_| {
        let entries = options.get();
        if disabled.get() && open.get() {
            open.set(false);
        }
        if open.get()
            && !active
                .get()
                .is_some_and(|index| entries.get(index).is_some_and(|item| !item.disabled))
        {
            let next = next_enabled(&entries, None, true);
            if active.get_untracked() != next {
                active.set(next);
            }
        }
    });
    Effect::new(move |_| {
        if !open.get() {
            return;
        }
        let Some(index) = active.get() else {
            return;
        };
        let Some(panel) = panel.get() else {
            return;
        };
        let Some(option) = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| {
                document.get_element_by_id(&format!("{}-{index}", list_id.get_value()))
            })
            .and_then(|element| element.dyn_into::<web_sys::HtmlElement>().ok())
        else {
            return;
        };
        let top = option.offset_top();
        let bottom = top + option.offset_height();
        if top < panel.scroll_top() {
            panel.set_scroll_top(top);
        } else if bottom > panel.scroll_top() + panel.client_height() {
            panel.set_scroll_top(bottom - panel.client_height());
        }
    });
    listen("pointerdown", false, move |event| {
        if !open.get_untracked() {
            return;
        }
        let inside = event
            .target()
            .and_then(|target| target.dyn_into::<web_sys::Node>().ok())
            .is_some_and(|target| {
                root.get_untracked()
                    .is_some_and(|root| root.contains(Some(&target)))
            });
        if !inside {
            close.run(false);
        }
    });
    listen("resize", false, move |_| close.run(false));
    listen("scroll", true, move |event| {
        if !open.get_untracked() {
            return;
        }
        let within_menu = event
            .target()
            .and_then(|target| target.dyn_into::<web_sys::Node>().ok())
            .is_some_and(|target| {
                panel
                    .get_untracked()
                    .is_some_and(|panel| panel.contains(Some(&target)))
            });
        if !within_menu {
            close.run(false);
        }
    });
    on_cleanup(move || {
        if let Some(panel) = panel.get_untracked() {
            let _ = panel.hide_popover();
        }
    });
    h::div()
        .class("xcss-select xscs-field")
        .node_ref(root)
        .child((
            h::label()
                .class("xcss-select-label")
                .id(label_id.get_value())
                .r#for(control_id.get_value())
                .child(label),
            h::button()
                .class("xcss-input xcss-select-trigger xcss-input--compact")
                .r#type("button")
                .node_ref(trigger)
                .id(control_id.get_value())
                .role("combobox")
                .attr("aria-haspopup", "listbox")
                .attr("aria-labelledby", label_id.get_value())
                .attr("aria-controls", list_id.get_value())
                .attr("aria-expanded", move || open.get().to_string())
                .attr("aria-activedescendant", move || {
                    if open.get() {
                        active
                            .get()
                            .map(|index| format!("{}-{index}", list_id.get_value()))
                    } else {
                        None
                    }
                })
                .disabled(IntoAttributeValue::into_attribute_value(move || {
                    disabled.get()
                }))
                .on(xcss_web::ev::click, move |_| {
                    if open.get_untracked() {
                        close.run(false);
                    } else {
                        expand.run(());
                    }
                })
                .on(xcss_web::ev::blur, move |_| close.run(false))
                .on(
                    xcss_web::ev::keydown,
                    move |event: web_sys::KeyboardEvent| {
                        if disabled.get_untracked() || event.is_composing() {
                            return;
                        }
                        let entries = options.get_untracked();
                        match event.key().as_str() {
                            "Escape" if open.get_untracked() => {
                                event.prevent_default();
                                event.stop_propagation();
                                close.run(true);
                            }
                            "Tab" => close.run(false),
                            "ArrowDown" | "ArrowUp" => {
                                event.prevent_default();
                                if !open.get_untracked() {
                                    expand.run(());
                                } else {
                                    active.set(next_enabled(
                                        &entries,
                                        active.get_untracked(),
                                        event.key() == "ArrowDown",
                                    ));
                                }
                            }
                            "Home" | "End" => {
                                event.prevent_default();
                                if !open.get_untracked() {
                                    expand.run(());
                                }
                                active.set(next_enabled(&entries, None, event.key() == "Home"));
                            }
                            "Enter" | " " => {
                                event.prevent_default();
                                if open.get_untracked() {
                                    if let Some(index) = active.get_untracked() {
                                        choose.run(index);
                                    }
                                } else {
                                    expand.run(());
                                }
                            }
                            _ if !event.ctrl_key()
                                && !event.meta_key()
                                && !event.alt_key()
                                && event.key().chars().count() == 1 =>
                            {
                                event.prevent_default();
                                if !open.get_untracked() {
                                    expand.run(());
                                }
                                let now = js_sys::Date::now();
                                let (previous, when) = prefix.get_value();
                                let key = event.key().to_lowercase();
                                let search = if now - when < 700.0 {
                                    format!("{previous}{key}")
                                } else {
                                    key
                                };
                                prefix.set_value((search.clone(), now));
                                if let Some(index) = entries.iter().position(|item| {
                                    !item.disabled && item.label.to_lowercase().starts_with(&search)
                                }) {
                                    active.set(Some(index));
                                }
                            }
                            _ => {}
                        }
                    },
                )
                .child((
                    h::span()
                        .class("xcss-select-value")
                        .child(IntoRender::into_render(move || {
                            options
                                .get()
                                .iter()
                                .find(|item| item.value == value.get())
                                .map(|item| item.label.clone())
                                .unwrap_or_else(|| t("请选择", "Choose an option"))
                        })),
                    xcss_web::svg::svg()
                        .class("xcss-select-arrow")
                        .attr("aria-hidden", "true")
                        .attr("viewBox", "0 0 16 16")
                        .child(
                            xcss_web::svg::path()
                                .attr("d", "m6 3 5 5-5 5")
                                .attr("fill", "none")
                                .attr("stroke", "currentColor")
                                .attr("stroke-width", "1.5"),
                        ),
                )),
            h::div()
                .class("xcss-select-panel")
                .node_ref(panel)
                .id(list_id.get_value())
                .role("listbox")
                .attr("popover", "manual")
                .attr("aria-labelledby", label_id.get_value())
                .child(IntoRender::into_render(move || {
                    let entries = options.get();
                    if entries.is_empty() {
                        return View::new(
                            h::div()
                                .class("xcss-select-empty xcss-row")
                                .role("status")
                                .child(t("没有可用选项", "No options available")),
                        )
                        .into_any();
                    }
                    View::new(
                        entries
                            .into_iter()
                            .enumerate()
                            .map(move |(index, option)| {
                                let option_value = option.value.clone();
                                h::div()
                                    .class("xcss-select-option xcss-row xcss-row--option")
                                    .id(format!("{}-{index}", list_id.get_value()))
                                    .role("option")
                                    .attr("aria-selected", move || {
                                        (value.get() == option_value).to_string()
                                    })
                                    .attr("aria-disabled", option.disabled.to_string())
                                    .attr("data-active", move || {
                                        (active.get() == Some(index)).to_string()
                                    })
                                    .on(
                                        xcss_web::ev::pointerdown,
                                        move |event: web_sys::PointerEvent| event.prevent_default(),
                                    )
                                    .on(xcss_web::ev::mouseover, move |_| {
                                        if !option.disabled {
                                            active.set(Some(index));
                                        }
                                    })
                                    .on(xcss_web::ev::click, move |_| choose.run(index))
                                    .child(option.label)
                            })
                            .collect_view(),
                    )
                    .into_any()
                })),
        ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keyboard_navigation_wraps_and_skips_disabled_choices() {
        let mut options = vec![
            SelectOption::new("all", "All".into()),
            SelectOption::new("controller", "Controller".into()),
            SelectOption::new("controlled", "Host".into()),
        ];
        options[1].disabled = true;
        assert_eq!(next_enabled(&options, Some(0), true), Some(2));
        assert_eq!(next_enabled(&options, Some(0), false), Some(2));
        assert_eq!(next_enabled(&options, Some(2), true), Some(0));
        for option in &mut options {
            option.disabled = true;
        }
        assert_eq!(next_enabled(&options, None, true), None);
    }
}
