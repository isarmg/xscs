# Leptos 0.8.19 CSR 裁剪实验

此分支基于官方 Leptos 提交 `fbc9ed631ff8c19c5eed02a1d4459e202cff7043`，只为现有 Axum REST API + Leptos CSR 应用提供可选的较小编译范围。它不是新的 UI 框架，也没有改变响应式图、Owner 清理或事件处理的实现。第二轮继续按功能开关拆分浏览器用不到的执行器与 Portal 的 Shadow DOM 支持。

## 使用方式

```toml
[dependencies]
leptos = { version = "=0.8.19", default-features = false, features = ["csr"] }

[patch.crates-io]
leptos = { path = "../leptos/leptos" }
leptos_macro = { path = "../leptos/csr_macro" }
```

路径应按应用目录调整。第二轮必须同时补丁 `leptos` 和配套的发布版 0.8.19 `leptos_macro`：新增的 `leptos_macro/full` 功能仅存在于 `csr_macro` 裁剪副本，官方发布版没有此功能，不能只替换 Leptos 主 crate。`csr_macro` 是独立包，不使用上游工作区中较早的宏版本。应用和共享组件库的 `leptos` 依赖都必须关闭默认功能；Cargo 会合并所有依赖边上的功能，任一依赖重新启用 `full` 都会恢复完整编译范围。

## 功能范围

- 默认启用新增的 `full` 功能，保留原始 0.8.19 的公开 API 和依赖功能范围。
- 关闭默认功能并启用 `csr` 后，保留 `view!` / `#[component]`、Signal、Memo、Effect、Owner、StoredValue、Callback、Context、Show、For、Portal、DOM 挂载、事件及异步任务等基础接口。
- 此最小模式不导出 `Resource` / `LocalResource` / `OnceResource` 等 `leptos_server` 资源类型、`ServerAction` / `ServerMultiAction`、服务端函数模块、`ActionForm` / `MultiActionForm`、`Await`、`Suspense` / `Transition`、`ErrorBoundary`、Leptos 服务端配置、`AutoReload` / `HydrationScripts` 等接口。当前试验的 xscs 与 xocs 使用自己的请求状态及错误反馈，不依赖这些接口。
- 通用 Action 类型仍来自 `reactive_graph`；它与被裁掉的 ServerAction 不同。xocs 的 `common::Action` 是应用自定义类型。
- 显式请求 `hydrate`、`hydration`、`ssr`、`islands` 或服务端序列化/传输功能时会自动启用 `full`。仅启用 `tracing` 不会重新引入 RPC 依赖。
- 第二轮将 `any_spawner/futures-executor` 移入 `full`，最小 CSR 仍启用 `any_spawner/wasm-bindgen`，挂载时仍通过 `Executor::init_wasm_bindgen()` 初始化浏览器执行器。其他依赖仍可能启用 `futures` 的默认 executor；此改动不等于移除所有 futures 执行器或异步任务支持。
- 第二轮新增 `shadow-dom` 功能，由 `full` 默认启用。最小 CSR 默认不编译 Portal 的 Shadow DOM 分支；仍保留 `use_shadow` prop，若未启用此功能却传入 `use_shadow=true`，会在挂载前明确 panic。需要 Shadow DOM 时可单独使用 `features = ["csr", "shadow-dom"]`，无需启用 `full`。
- 第二轮还关闭普通和 `cfg(erase_components)` 两条 `leptos_macro` 依赖边的默认功能，由顶层 `full` 显式恢复 `leptos_macro/full`，避免过程宏默认功能重新引入服务端函数宏依赖；组件擦除选项仍单独保留。

## 改动边界

`leptos/Cargo.toml` 以 crates.io 发布的 0.8.19 规范化清单为基础，保留发布版依赖版本范围，避免官方单一仓库中的路径继承把应用锁定的关联 crate 换成较早版本。应用原有 Cargo.lock 仍应保留并核对；本清单没有将所有关联 crate 固定到一个新的统一版本。

最小模式将 `leptos_config`、目标端 `leptos_hot_reload`、`leptos_server`、`server_fn`、`serde_qs` 和直接的 `hydration_context` 依赖变为可选，关闭 `tachys/reactive_stores`，并同步关闭依赖这些接口的模块及导出。这样可以取消由 `leptos_server` 传递启用的 `reactive_graph/hydration`，让 CSR Owner 不再携带供服务端渲染共享数据使用的上下文。

第一轮的 `Portal` 最小模式移除服务端共享上下文的浏览器判断，`full` 模式保留原判断。第二轮仅将创建 Shadow DOM 的分支及 Leptos 直接依赖中的 `web-sys/ShadowRoot`、`ShadowRootInit`、`ShadowRootMode` 绑定放到 `shadow-dom` 功能下；其他依赖仍可独立启用同名 web-sys 功能。两轮均保留 `wasm32` 目标判断、挂载位置、SVG 支持、Effect、视图卸载和 Owner 清理逻辑。`full` 或单独启用 `shadow-dom` 时的 Shadow DOM 分支保持原实现。

第一轮没有改动关联 crate 的实现。第二轮配套裁剪 `leptos_macro`，将服务端函数过程宏依赖设为可选，由 `leptos_macro/full` 恢复；构建主机上的 `leptos_hot_reload` 语法解析辅助函数仍保留。`tachys/testing` 也可能仍由未修改的 `leptos_dom` 启用，本实验不宣称关闭它。响应式图与 Tachys 的实现没有改动，SSR 与 hydration 的渲染 trait 在 Tachys 内仍然存在，并未全面删除。

减少源码编译范围不等于同比减少最终 WASM。需要分别比较依赖与功能图、干净构建时间、wasm-bindgen 产物及 gzip 大小，并运行现有真实浏览器回归。第二轮源码改动不能沿用第一轮结果作为验证；SSR、hydration、完整 API 的兼容性也需要对应的独立测试。测试结果由工作区实验报告记录；本文件只描述裁剪机制与兼容范围。

保留原 MIT 许可证及版权声明。
