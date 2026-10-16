# XSCS

`feat/management-prep` 将 XSCS 收敛为只支持自定义 RustDesk 的管理服务。配套 xscc 直接维护原生应用，管理库运行在应用进程内。

本分支配合 xscc 中集成的自定义 RustDesk 原生应用。已保留管理员认证、实例绑定、凭据撤销、持久任务、结果收存和审计；支持控制端/被控端实例、双向编辑的控制授权、被控地址上报和授权列表同步；可投递任务仅有只读诊断。硬件、编码和画质等会话设置由 RustDesk 主控端选择，服务端不保存或下发这些设置。xscc 已接入原生会话证明、授权持续检查及解绑；正式部署迁移和物理主机串流仍需验收。

控制端支持 Linux amd64、macOS 26 Apple Silicon、Windows 11 amd64、iOS 26 arm64 和 Android arm64（API 29 起）；被控端支持前三个桌面平台。iOS/Android 不提供被控服务。

服务运行目标仍为 Linux x86_64 GNU，入口为 `xscs --root ROOT`。安装与离线维护继续由 XSSI 负责。本分支使用新建的 schema v3 状态；不会自动转换或接管旧实例。

- [架构与边界](docs/architecture.md)
- [当前设备协议](docs/client-management-v2.md)
- [开发与验证](docs/development.md)
- [原生集成与验收条件](docs/rustdesk-integration.md)

前端已迁移到 Leptos CSR，Axum 后端保持现有接口。构建与验收见 [Leptos 管理界面](docs/leptos-web.md)。
