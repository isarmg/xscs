# XSCS

`feat/management-prep` 将 XSCS 收敛为支持自定义 RustDesk 与 One-KVM 的管理服务。配套 xscc 直接维护原生应用，管理库运行在应用进程内。

本分支配合 xscc 中集成的自定义 RustDesk 原生应用。已保留管理员认证、实例绑定、凭据撤销、持久任务、结果收存和审计；支持控制端/被控端实例、双向编辑的控制授权、被控地址上报和授权列表同步；可投递任务仅有只读诊断。硬件、编码和画质等会话设置由 RustDesk 主控端选择，服务端不保存或下发这些设置。xscc 已接入原生会话证明、授权持续检查及解绑；正式安装生命周期和物理主机串流仍需验收。

控制端支持 Linux amd64、macOS 26 Apple Silicon、Windows 11 amd64、iOS 26 arm64 和 Android arm64（API 29 起）；被控端支持前三个桌面平台。iOS/Android 不提供被控服务。

服务运行目标仍为 Linux x86_64 GNU，入口为 `xscs --root ROOT`。安装与离线维护由本仓库 `tools/installer` 完成，保留导入代码的原始来源与许可证。本开发工作树使用 schema v4；不自动转换旧状态，确切 v3 可使用显式离线维护命令保留实例与凭据升级。

- [One-KVM 类型、授权与升级](docs/one-kvm.md)
- [架构与边界](docs/architecture.md)
- [当前设备协议](docs/client-management-v2.md)
- [开发与验证](docs/development.md)
- [原生集成与验收条件](docs/rustdesk-integration.md)

前端已迁移到 xcss-web CSR，Axum 后端保持现有接口。构建与验收见 [xcss-web 管理界面](docs/xcss-web.md)。


生产前端构建使用仓库内 Rust 工具，无需安装 npm 依赖。准备 Rust 1.99.0、wasm32-unknown-unknown target 和 wasm-bindgen-cli 0.2.129 后，在产品根目录执行：

```sh
cargo +1.99.0 run --quiet --locked --manifest-path tools/web-build/Cargo.toml -- build --product-root .
```

开发监听、Axum 集成和测试边界见 [Rust 原生 Web 构建与测试](docs/rust-native-web.md)。Node/Playwright 仅用于当前真实浏览器验收，退役的 Vite 与 React 对照实现已移除。

## 本机 HTTP 测试

本机测试使用 `config/config.http-test.json.example` 的配置：`development: true`，
并显式监听 `0.0.0.0:18104`。将配置应用到安装根目录 `ROOT/config/config.json`
时保留服务账号的读取权限，然后重启服务。

Windows 与 WSL 同机访问地址为 `http://127.0.0.1:18104`，管理员登录使用安装时创建的
用户名和密码。本机管理员 HTTP 会话只接受回环地址的同源请求；实例绑定使用管理页面
为设备生成的实例密码。客户端对 `localhost`、`127.0.0.1` 和 `::1` 支持 HTTP/WS。
生产配置保持默认的 HTTPS 管理员来源策略。
