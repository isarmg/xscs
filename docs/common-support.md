# 本仓库的基础代码

原生服务使用本地 `crates/foundation` 的配置、日志、SQLite、认证和生命周期代码；浏览器组件、构建器、CSS 和字体也已纳入本仓库。产品构建不需要克隆 xcss 或 xcsc，也不需要安装它们的 npm 包。

| 本地输入 | 实际用途 | 身份与许可证 |
|---|---|---|
| `crates/foundation` | Linux x86_64 GNU 原生服务端模块 | 私有 Cargo 包 `xscs-foundation`，根 workspace path 与锁文件；`ORIGIN.md` 和 Apache-2.0 `LICENSE` 保留历史来源 |
| `web/xcss-web/vendor/xcss-web` | `wasm32-unknown-unknown` 浏览器 CSR | 已提交的组件、内部核心和过程宏；各自的 `SOURCE.json`、锁文件与原始许可证 |
| `web/vendor/xcss` | 使用的 CSS、字体与必要 DOM 适配 | 已提交的源码与字体资源、逐文件摘要和原始许可证 |
| `tools/web-build` | Rust 原生 Web 构建和清单生成 | 本仓库维护的构建工具与独立锁文件 |

`xcss-product.toml` 记录产品实际使用的 Axum、`web-csr-admin` 和能力；foundation 身份指向本仓库路径。Rust 依赖别名和部分目录沿用原名，以保留已有调用和生成资源的兼容性；这些名称不代表外部仓库依赖。历史来源说明和许可证也不触发下载。

## 构建与检查

在产品根目录执行：

```sh
cargo metadata --locked --offline --format-version 1
cargo tree --locked -e normal
cargo +1.99.0 run --locked --manifest-path tools/web-build/Cargo.toml -- check --product-root .
cargo +1.99.0 run --locked --manifest-path tools/web-build/Cargo.toml -- test --product-root .
cargo +1.99.0 run --locked --manifest-path tools/web-build/Cargo.toml -- build --product-root .
python3 crates/foundation/tools/schema_compose.py --profile server-control-plane --capability admin-persistent --capability durable-operations --product schema/product.sql --output schema/generated/current_schema.sql --check
```

离线 Cargo 检查仍需要预先缓存锁文件中的 crates.io 包。生产 Web 构建使用仓库内 Rust 工具；Node/Playwright 仅用于真实浏览器验收。版本和浏览器命令见 [Rust 原生 Web 构建](rust-native-web.md)。

本地维护这些代码时同步相应清单、资源摘要和测试，保留原始许可证。编译、依赖和资源清单的身份绑定本产品的源码提交；原有来源记录用于历史追溯。数据库维护使用 [开发文档](development.md) 中的本地 schema 工具，既有数据格式和规范指纹保持不变。

`crates/protocol` 是产品自己的业务类型与校验；配套 Client 固定它的实际源码修订消费。
