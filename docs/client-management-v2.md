# 设备管理协议 v2

设备 HTTP/WSS 入口为 `/xscc/v2/pairing`、`/xscc/v2/enroll`、`/xscc/v2/identity`、`/xscc/v2/connect`。管理员业务 API 为 `/api/v1/rustdesk/`。旧设备合同、旧业务命令和非 embedded 执行方式均不接受。

Hello 绑定 manager_id、device_id、installation_id；能力声明只允许 backend=rustdesk、execution_mode=embedded，携带客户端版本、平台与诊断能力。管理通道认证不等于 RustDesk 对等会话授权。

当前唯一业务命令 ReadDiagnostics 返回集成状态、实际 RustDesk 版本。awaiting_integration 不得携带伪造的运行身份；ready 必须有实际运行版本。未实现的配置、应用、配对和服务控制不是当前合同的一部分。

WSS 消息最多 64 KiB，报告预留信封空间。Server 先保存任务再投递；嵌入库先通过宿主 Journal 保存结果再发送。ResultAccepted 校验操作指纹与报告摘要，提交后才能压缩已收存结果。日志不包含凭据。

配对、注册与撤销沿用一次性 enrollment 和独立 256-bit credential；原生宿主必须在注册前保存待完成身份，响应丢失后恢复原事务。当前分支没有 RustDesk 配对 UI 或原生存储实现。
