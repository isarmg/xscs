# 开发与验证

输入边界、实时授权、错误传播与热路径简化遵循[避免过度防御性编程](defensive-programming.md)。

当前使用 Rust 1.99。在产品根目录执行 `cargo +1.99.0 run --quiet --locked --manifest-path tools/web-build/Cargo.toml -- build --product-root .` 生成嵌入资源；详见 [Rust 原生 Web 构建](rust-native-web.md)。浏览器验收另需 Node 26.7 与锁定 devDependencies，再运行 `npm --prefix web run test:browser`，使用 Chromium/Firefox 与管理 API 替身。

Rust 检查为 `cargo fmt --all -- --check`、`cargo test --locked --workspace --all-targets -- --test-threads=2`、`cargo clippy --locked --all-targets -- -D warnings`。测试操作隔离数据、端口和进程。两个测试线程沿用原基础库的验收参数，避免不同认证夹具竞争同一进程的两份密码哈希许可；生产并发和超时预算保持不变。

协议唯一源码位于 crates/protocol。xscc 配套分支通过完整 Git revision 固定消费；尚未推送时允许临时 Cargo 命令行 patch 做本地联调，不提交相邻 path 依赖。

变更 DDL 后，在本仓库执行 `python3 crates/foundation/tools/schema_compose.py --profile server-control-plane --capability admin-persistent --capability durable-operations --product schema/product.sql --output schema/generated/current_schema.sql`。基础库、DDL 与维护工具均由本仓库维护；schema identity 继续使用既有规范 sqlite_schema 指纹，同时更新 revision、release.json、安装校验材料和夹具。

配套 xscc 分支已接入实例绑定、平台私有存储、管理生命周期、授权列表与原生会话证明。客户端五平台和被控端三平台的范围见控制授权合同；物理设备的实际串流、前后台切换与恢复仍需要单独验收。

当前正式版号为 `2.0.0`，服务端、协议 crate、Web 包和 `release.json` 保持一致。当前开发工作树数据格式为 `v4` / schema revision 4，新增固定实例 backend。服务不会自动迁移旧库；确切 v3 可使用显式离线命令保留实例、凭据与任务升级到 v4，其它旧格式仍被拒绝；见 [One-KVM 接入](one-kvm.md)。安装与发行使用现有 XSSI 契约；正式打包要求干净源码和精确指向 HEAD 的 annotated `v2.0.0` 标签。见 [2.0.0 发行说明](releases/2.0.0.md)。
