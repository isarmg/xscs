# RustDesk 管理分支

本分支支持自定义 RustDesk 与 One-KVM；RustDesk 客户端通过 xscc 管理库嵌入原生进程，One-KVM 是另一种被控端实现。本仓库负责管理服务；客户端实现与原生验收见 [xscc 文档](https://github.com/isarmg/xscc/blob/main/docs/README.md)，接入边界见[原生集成与验收](rustdesk-integration.md)。

- [控制授权、角色与地址](access-control.md)
- [One-KVM 类型、授权与升级边界](one-kvm.md)
- [架构](architecture.md)
- [设备协议 v2](client-management-v2.md)
- [通信与执行恢复](communication-reliability.md)
- [RustDesk 稳定后的接入](rustdesk-integration.md)
- [开发验证](development.md)
- [配置](configuration.md)
- [管理维护](administration.md)
