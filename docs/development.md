# 开发与发行验证

当前 Server 目标为 Linux x86_64 GNU，Rust 使用根 `rust-toolchain.toml` 的精确版本，Node 与 npm 工具链及依赖使用 Web 配置和权威锁文件。公共 Rust 输入为 XCSS 1.0.0 的完整源码修订，Web 使用同版正式归档；不能将相邻源码覆盖冒充正式依赖。

在产品根目录先运行 `npm ci --prefix web`，再按 `xcss-web-build.json` 使用锁定的 XCSS Web 构建入口生成嵌入资源。Rust 验证执行 `cargo fmt --all -- --check`、`cargo test --locked --all-targets --all-features` 和 `cargo clippy --locked --all-targets --all-features -- -D warnings`。测试只操作隔离数据、端口和进程；本地不能执行的目标环境需独立记录。

`crates/server/src/config_definition.rs`、`installation_definition.rs`、当前 DDL 及构建入口生成与源码身份绑定的配置、凭据、资源和数据契约。独立 XSSI 辅助程序实施初始化、校验及维护；产品源码中不保留运行入口不可达的安装实现。

正式打包由 `scripts/package-release.py`从精确 `config/xssi.lock` 修订取得公开 XSSI 源码，要求干净工作树、当前不可变标签、真实编译来源与 Web 清单。SBOM 结合实际依赖图、最终文件、独立程序及第三方资源；打包后验证 `share/package.json`、`share/package.sha256` 和实际载荷，成功后才能发布。源码构建、模拟、实机运行与最终包验收分别记录。
