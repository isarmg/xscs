# xscs 的公共依赖

服务端通过 `xcss` 使用配置、日志、SQLite、认证和生命周期机制；浏览器通过 `xcss-web` 使用 Rust CSR 组件与函数式视图。产品保留业务协议、权限、数据模型和页面状态。

| 输入 | 实际用途 | 来源与锁定位置 |
|---|---|---|
| `xcss` | Linux x86_64 GNU 原生服务端公共模块 | 根 `Cargo.toml` 的官方 Git URL、完整 revision 和精确版本；根 `Cargo.lock` |
| `xcss-web` | `wasm32-unknown-unknown` 浏览器 CSR | `web/xcss-web/vendor/xcss-web/SOURCE.json` 绑定官方修订与逐文件摘要；前端 `Cargo.lock` |
| `@xcss/web` | 当前使用的 CSS、字体与必要 DOM 适配 | `web/package.json` 的不可变发行 URL 和 `package-lock.json` 的真实完整性摘要 |

`xcss-product.toml` 声明 Axum、`web-csr-admin` 和实际公共能力，其原生依赖身份与根 Cargo 声明一致。浏览器内部核心和过程宏由公共入口引入，产品不直接依赖它们。源码快照不能手改；更新时从公共来源同步并重新核验摘要与原始许可证。

## 构建与检查

在产品根目录执行：

```sh
cargo metadata --locked --no-deps --format-version 1
cargo tree --locked -e normal
cargo +1.99.0 run --locked --manifest-path tools/web-build/Cargo.toml -- check --product-root .
cargo +1.99.0 run --locked --manifest-path tools/web-build/Cargo.toml -- test --product-root .
cargo +1.99.0 run --locked --manifest-path tools/web-build/Cargo.toml -- build --product-root .
```

生产 Web 构建使用仓库内的 Rust 工具，不要求安装 npm 依赖。Node/Playwright 用于真实浏览器验收；按需运行 `npm ci --prefix web` 准备测试工具。Rust 版本、wasm-bindgen 与浏览器测试命令见 [Rust 原生 Web 构建](rust-native-web.md)。

Axum 使用经摘要校验的内嵌资源。构建工具记录实际 WASM、锁文件、来源和资源摘要；正式发行还需通过产品自己的离线初始化材料、依赖物料和资源清单校验。编译通过不等于发行或平台验收完成。

更新公共依赖时同步清单、完整修订、锁文件、调用代码与相应测试；CSS/字体归档来源与浏览器 Rust 来源分别核验。公共库无需单独部署后台服务，安装维护由 xssi、升级恢复由 xssc 完成。

`crates/protocol` 是产品自己的业务类型与校验，不属于公共库；配套 Client 固定它的实际源码修订消费。
