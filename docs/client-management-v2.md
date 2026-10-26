# 设备管理协议 v2

设备 HTTP/WSS 入口为 `/xscc/v2/pairing`、`/xscc/v2/enroll`、`/xscc/v2/identity`、`/xscc/v2/connect`、`/xscc/v2/access`、`/xscc/v2/unbind`。管理员业务 API 为 `/api/v1/rustdesk/`。旧设备合同、旧业务命令和非 embedded 执行方式均不接受。

Hello 绑定 manager_id、device_id、installation_id；能力声明允许 backend=rustdesk 或 one_kvm、execution_mode=embedded；backend 必须与实例创建时的固定类型一致，携带客户端版本、平台与诊断能力。管理通道认证不等于 RustDesk 对等会话授权。

当前唯一业务命令 ReadDiagnostics 返回集成状态和所属引擎的实际版本：RustDesk 使用 rustdesk_version，One-KVM 使用 one_kvm_version，不能混填。awaiting_integration 不得携带伪造的运行身份；ready 必须有实际运行版本。配置下发、应用管理、原生对等会话配对和服务控制不属于当前管理任务合同。

WSS 消息最多 64 KiB，报告预留信封空间。Server 先保存任务再投递；嵌入库先通过宿主 Journal 保存结果再发送。ResultAccepted 校验操作指纹与报告摘要，提交后才能压缩已收存结果。日志不包含凭据。

配对、注册与撤销沿用一次性 enrollment 和独立 256-bit credential；原生宿主必须在注册前保存待完成身份，响应丢失后恢复原事务。配对界面和受保护存储由客户端宿主负责，其实现与验收见 [xscc 文档](https://github.com/isarmg/xscc/blob/main/docs/README.md)；本仓库不实现这些客户端能力。

实例角色、授权关系、地址上报、授权租约见 [控制授权合同](access-control.md)。这是 v2 准备分支的合同扩展，Heartbeat 必须携带 addresses，客户端与服务端需同时升级。任务协议和只读诊断保持不变。硬件/编码等具体设置由对应客户端与设备处理，不通过管理任务下发。

授权列表包含保密的实例对会话证明密钥，仅供实例后台完成通道绑定的双向挑战证明；不得出现在 GUI、日志或持久执行记录中。接口两端必须使用同一当前合同，按输入内容和身份约束校验，不按发行版本猜测兼容性。

One-KVM 的 Linux ARM64 / ARMv7 能力、双端实例与旧库升级边界见 [One-KVM 接入](one-kvm.md)。原生 RustDesk 控制端可授权连接 One-KVM 被控端，AccessPolicy/AccessPeer 不增加类型字段，以保持现有客户端解析兼容。
