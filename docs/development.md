# 开发与验证

当前 Rust 1.99、Node 26.7。先执行 `npm ci --prefix web --ignore-scripts`，再运行 `npm --prefix web run build` 生成嵌入资源。Web 验收为 `npm --prefix web run test:browser`，使用 Chromium/Firefox 与管理 API 替身。

Rust 检查为 `cargo fmt --all -- --check`、`cargo test --locked --all-targets`、`cargo clippy --locked --all-targets -- -D warnings`。测试操作隔离数据、端口和进程。

协议唯一源码位于 crates/protocol。xscc 配套分支通过完整 Git revision 固定消费；尚未推送时允许临时 Cargo 命令行 patch 做本地联调，不提交相邻 path 依赖。

变更 DDL 后，从 Cargo.toml 固定的 xcss 源码执行 tools/xcss_schema_compose.py，指定 server-control-plane、admin-persistent、durable-operations 和产品 schema/product.sql。schema identity 使用 xcss 的规范 sqlite_schema 指纹；同时更新 revision、release.json、安装校验材料和夹具。

本分支只准备集成，真实 RustDesk、平台存储、主机生命周期和正式安装升级留到稳定版本验收。
