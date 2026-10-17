# Rust 原生 Web 构建与测试

生产前端编译、资源整理、gzip、清单、预算/许可证检查和后端集成使用本仓库 `tools/web-build`，不需要 `npm ci`、Node 或 Vite。后端仍为 Axum。前端只使用裁剪的 xcss-web CSR；框架源码与公共组件由 xcss 统一维护与同步。

在产品根目录执行：

```sh
cargo +1.99.0 run --quiet --locked --manifest-path tools/web-build/Cargo.toml -- build --product-root .
cargo +1.99.0 run --quiet --locked --manifest-path tools/web-build/Cargo.toml -- serve --product-root . --debug --bind 127.0.0.1:5173
cargo +1.99.0 run --quiet --locked --manifest-path tools/web-build/Cargo.toml -- server --product-root . --config xcss-web-build.json --mode development
```

需要 Rust 1.99.0、wasm32-unknown-unknown target 和 wasm-bindgen-cli 0.2.129。默认输出 `web/dist`；`--dist` / `XCSS_WEB_DIST` 用于隔离验收构建。`serve` 监听 Rust/资源文件变化；`preview` 只提供已有资源，不重新构建。Cargo 缓存齐备时可显式传 `--offline`，默认允许获取锁定依赖。

package.json 的 build/dev/check/test Rust 入口只是这些命令的便利别名。Node、Playwright、Vite 和少量旧 JS 库只保留在 devDependencies，供浏览器验收与历史 TypeScript/React 对照测试使用；生产构建不读取 node_modules。Vite 的 transformWithOxc 历史测试继续执行，未用新的断言替代原断言。

`tools/web-build/test-preview.mjs` 是测试专用适配器：浏览器脚本仍通过 Node 执行，但 HTTP 资源由 Rust CLI `preview` 提供。测试默认申请端口 0，读取 listening JSON 后返回实际端口，并在清理时结束 Rust 子进程。可设置 `XCSS_WEB_BUILD_BIN` 复用预编译工具，避免每个测试重复启动 Cargo。

CI 与正式打包脚本的生产 build/server 直接调用 Cargo，保留原 source revision、receipt、精确安装器与包校验守卫。浏览器测试仍安装其锁定 devDependencies；它们属于验证工具，不属于 Axum 运行时或生产前端构建依赖。

## 函数式视图

当前 `web/xcss-web/src` 的界面使用普通 Rust builder、元组、迭代器和闭包组合，已移除 authored `view!`。`#[component]` 仅处理 xcss-web 组件参数与生命周期；DOM 构造直接调用原有 xcss-web API，没有添加新的模板语法。Rustfmt、WASM 类型检查、Clippy 与原浏览器断言继续适用。

一次性迁移由 xcss 的 `tools/rustify-views` 使用锁定 CSR 宏 codegen 完成，生产编译不调用此转换器。组件适配块保留了 codegen 对 let-and-return、unit props 的局部 lint 边界；业务逻辑没有全文件 lint 豁免。后续维护可直接编辑普通 Rust 视图，再执行相同构建与浏览器验收。
