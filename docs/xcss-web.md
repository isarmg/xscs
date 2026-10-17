# xcss-web 管理界面

当前入口为 `web/xcss-web`，仅通过 `xcss-web` 路径 crate 接入；主分支使用 Rust 生产构建。构建与验收命令见 [Rust 原生 Web 构建与测试](rust-native-web.md)。本文保留首次迁移时的流程和测量记录，旧 Node/Vite 命令与体积数字不代表本次改名后的验证结果。

管理界面使用 xcss-web CSR：Rust 编译到浏览器 WASM，Axum 继续提供管理员认证、RustDesk 实例、授权、任务和静态资源接口。服务端接口及数据库格式不因前端迁移改变。

当前 `main` 已合并 xcss-web CSR 与 Rust 原生构建；直接在主分支构建和验收。下面的首次迁移流程与数字作为历史记录，正式发行和线上部署继续使用产品发布流程。 此分支基于既有 `feat/management-prep`，保留其五个管理与原生集成准备提交。

`web/xcss-web/src` 是界面、状态和请求校验的实现；`web/src/main-xcss-web.js` 负责加载字体、CSS、角标绘制适配器和 WASM。旧 React 实现只保存在 `web/tests/fixtures/react-baseline`，用于审查迁移差异，不参与生产构建或验收。

## 构建

需要 Node 26.7.0、Python 3.11 或更新版本，以及固定 Rust/WASM 工具：

```sh
rustup toolchain install 1.99.0 --profile minimal --component rustfmt,clippy --target wasm32-unknown-unknown
cargo +1.99.0 install wasm-bindgen-cli --version 0.2.129 --locked
npm ci --prefix web
npm run build --prefix web
```

默认构建生成优化 WASM，先在临时目录生成绑定，再更新 `web/pkg`。Vite 输出到 `web/dist`；现有 `xcss-build-server` 和 Axum 嵌入资源流程继续使用这个目录。`CARGO_TARGET_DIR` 和 `WASM_BINDGEN` 可以显式指定缓存目录和固定版本工具路径。

每份构建包含 `assets/xcss-web-build.json`，记录 Rust 输入、锁文件、公共组件输入、构建工具及生成文件的 SHA-256。编译期间输入变化会中止构建。WASM 上限 4 MiB，普通资产上限 512 KiB，完整资产上限 16 MiB；字体保留原许可证。

## 验证

```sh
cargo +1.99.0 run --locked --manifest-path tools/web-build/Cargo.toml -- check --product-root .
cargo +1.99.0 run --locked --manifest-path tools/web-build/Cargo.toml -- test --product-root .
python3 -I -B scripts/test-xcss-web-build.py
npm run test:browser --prefix web
```

浏览器验收使用 Chromium、Firefox，必须实际加载独立 `.wasm`，且 Rust 启动后出现 `data-xcss-runtime="xcss-web"`。覆盖双向授权、授权码显隐、诊断任务与幂等键、新实例角色、刷新时保留未保存名称和授权码。旧 React 对照代码通过测试不能作为迁移验收依据。

客户端和服务端发布、部署仍遵循原有流程；本次源码迁移不自动部署线上实例。

## Axum 对 WASM 的适配

后端保留 Axum，并对浏览器资源作针对性优化：构建时生成 `.wasm.gz`，运行时按 `Accept-Encoding` 协商，返回 `application/wasm`、`Content-Encoding: gzip` 和 `Vary: Accept-Encoding`。压缩与原始表示使用各自的强 ETag；HEAD、304 及缺失压缩文件回退保留原资源服务语义。HTML 预加载 WASM，让下载提前开始。沿用原有缓存策略，未以文件名猜测是否可永久缓存。

压缩适配只用于静态 WASM，不涉及认证/API 响应。通用实现维护在 `xcss/scripts/xcss-web-assets.rs` 与 `xcss/web/xcss-web-assets.mjs`；`vendor-xcss-web.py --check` 同时验证六个独立仓库中的副本。

CI 安装候选和正式 release 均单独审计 `web/xcss-web/Cargo.lock`，保留原根锁打包审计和 yanked 检查。内嵌 `xcss-web-build.json` 的前端 Rust 依赖清单补充现有 SBOM，当前并未将它合并进远端打包器生成的 CycloneDX 文件。
