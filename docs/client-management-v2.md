# 设备管理协议 v2

设备 HTTP/WSS 入口为 `/xscc/v2/pairing`、`/xscc/v2/enroll`、`/xscc/v2/identity`、`/xscc/v2/connect`、`/xscc/v2/access`、`/xscc/v2/unbind`。管理员业务 API 为 `/api/v1/rustdesk/`。旧设备合同、旧业务命令和非 embedded 执行方式均不接受。

Hello 绑定 manager_id、device_id、installation_id；能力声明只允许 backend=rustdesk、execution_mode=embedded，携带客户端版本、平台与诊断能力。管理通道认证不等于 RustDesk 对等会话授权。

当前唯一业务命令 ReadDiagnostics 返回集成状态、实际 RustDesk 版本。awaiting_integration 不得携带伪造的运行身份；ready 必须有实际运行版本。未实现的配置、应用、配对和服务控制不是当前合同的一部分。

WSS 消息最多 64 KiB，报告预留信封空间。Server 先保存任务再投递；嵌入库先通过宿主 Journal 保存结果再发送。ResultAccepted 校验操作指纹与报告摘要，提交后才能压缩已收存结果。日志不包含凭据。

配对、注册与撤销沿用一次性 enrollment 和独立 256-bit credential；原生宿主必须在注册前保存待完成身份，响应丢失后恢复原事务。当前分支没有 RustDesk 配对 UI 或原生存储实现。

实例角色、授权关系、地址上报、授权租约见 [控制授权合同](access-control.md)。这是 v2 准备分支的合同扩展，Heartbeat 必须携带 addresses，客户端与服务端需同时升级。任务协议和只读诊断保持不变。硬件/编码等具体设置由 RustDesk 主控端选择，不通过管理任务下发。

授权列表包含保密的实例对会话证明密钥，仅供原生后台完成通道绑定的双向挑战证明；不得出现在 GUI、日志或持久执行记录中。新旧客户端必须随本分支合同扩展配套升级。
