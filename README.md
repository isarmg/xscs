# XSCS

`feat/management-prep` 将 XSCS 收敛为只支持自定义 RustDesk 的管理服务。管理客户端 xscc 将作为 RustDesk 的库依赖直接运行在主机进程中。

本分支是迁移准备，尚未完成真实 RustDesk 接入。已保留管理员认证、实例绑定、凭据撤销、持久任务、结果收存和审计；当前业务只开放只读诊断。配置写入、会话授权、媒体控制及正式部署迁移等待 RustDesk 稳定后再实施。

服务运行目标仍为 Linux x86_64 GNU，入口为 `xscs --root ROOT`。安装与离线维护继续由 XSSI 负责。本分支使用新建的 v2 状态；不会自动转换或接管旧实例。

- [架构与边界](docs/architecture.md)
- [当前设备协议](docs/client-management-v2.md)
- [开发与验证](docs/development.md)
- [稳定后接入条件](docs/rustdesk-integration.md)
