# 开发与验证

当前使用 Rust 1.99。在产品根目录执行 `cargo +1.99.0 run --quiet --locked --manifest-path tools/web-build/Cargo.toml -- build --product-root .` 生成嵌入资源；详见 [Rust 原生 Web 构建](rust-native-web.md)。浏览器验收另需 Node 26.7 与锁定 devDependencies，再运行 `npm --prefix web run test:browser`，使用 Chromium/Firefox 与管理 API 替身。

Rust 检查为 `cargo fmt --all -- --check`、`cargo test --locked --all-targets`、`cargo clippy --locked --all-targets -- -D warnings`。测试操作隔离数据、端口和进程。

协议唯一源码位于 crates/protocol。xscc 配套分支通过完整 Git revision 固定消费；尚未推送时允许临时 Cargo 命令行 patch 做本地联调，不提交相邻 path 依赖。

变更 DDL 后，从 Cargo.toml 固定的 xcss 源码执行 tools/xcss_schema_compose.py，指定 server-control-plane、admin-persistent、durable-operations 和产品 schema/product.sql。schema identity 使用 xcss 的规范 sqlite_schema 指纹；同时更新 revision、release.json、安装校验材料和夹具。

配套 xscc 分支已接入实例绑定、平台私有存储、管理生命周期、授权列表与原生会话证明。客户端五平台和被控端三平台的范围见控制授权合同；物理设备的实际串流、前后台切换与恢复仍需要单独验收。

当前正式版号为 `2.0.0`，服务端、协议 crate、Web 包和 `release.json` 保持一致。数据格式仍为 `v3` / schema revision 3。安装与发行使用现有 XSSI 契约；正式打包要求干净源码和精确指向 HEAD 的 annotated `v2.0.0` 标签。见 [2.0.0 发行说明](releases/2.0.0.md)。
