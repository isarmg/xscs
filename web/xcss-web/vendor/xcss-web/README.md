# xcss-web

面向浏览器的 Rust CSR 框架入口，同时提供共享组件、主题、国际化与管理 API 合同。
应用只依赖本 crate，通过 `xcss_web::prelude::*`、`xcss_web::html` 和
`xcss_web::component` 构建普通 Rust 函数式视图。后端继续使用 Axum。

内部运行时位于 `vendor/xcss-web-core`，过程宏位于 `vendor/xcss-web-macro`。
两者都以 CSR 为默认构建模式；产品构建校验禁止启用 SSR、hydration、full 与服务端函数依赖。

## 接入与函数式视图

独立产品将本目录的源码快照放入 `web/xcss-web/vendor/xcss-web`，在其前端 crate 中使用：

```toml
[dependencies]
xcss-web = { path = "vendor/xcss-web", default-features = false, features = ["csr"] }
```

```rust
use xcss_web::{ev, html, prelude::*};

#[component]
fn Counter() -> impl IntoView {
    let (count, set_count) = signal(0);
    html::button()
        .on(ev::click, move |_| set_count.update(|value| *value += 1))
        .child(move || count.get())
}

pub fn start() {
    xcss_web::mount("app", Counter);
}
```

所有组件、宏和运行时类型均通过同一个 `xcss_web` 入口访问，无需依赖内部 core 或 macro crate。

## 来源与许可证

框架实现基于 Leptos 0.8.19 裁剪并整合。原 MIT 许可证、版权通知及完整来源和修改记录
见两个内部 crate 的 `README.md` 与 `LICENSE`。共享组件和本仓库自有代码沿用仓库许可证。
`SOURCE.json` 为源码快照逐文件记录 SHA-256，包含这些 README 和许可证。
独立第三方依赖仍保留其真实 Cargo 包名和各自许可证。
