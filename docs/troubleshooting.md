# 排查 xscs 问题

先记录发生时间、软件 `identity`、HTTP 状态和错误 `code` / request ID。以下检查按从本机到代理、再到客户端的顺序进行。

| 现象 | 检查 | 预期与下一步 |
|---|---|---|
| 服务立即退出 | `sudo journalctl -u xscs.service -n 100 --no-pager` | 按首个错误核对发行目录、环境文件、权限与初始化结果 |
| `release root must be releases/1.0.2` | 查看发行实体路径和 current 链接 | 使用 `/opt/isarmg/xscs/releases/1.0.2` 及同安装树的绝对 current 链接 |
| `/healthz` 正常，`/readyz` 返回 503 | 查看运行日志；执行只读 `config validate --json` | 运行依赖全部健康后 ready 才为 true；需要 doctor 时先停服 |
| 后端就绪，浏览器打不开 | 从入口主机检查配置的后端地址 | 公网使用有效 HTTPS；同机入口可回源回环端口，跨服务器使用配置的内网地址与端口 |
| 登录失败或 403 | 核对用户名、主机时钟、Host/Origin 和 Cookie | 使用同一 HTTPS 源站；用户名规则见配置参考 |
| 页面报 `invalid_error_response` | 检查代理返回的状态和 Content-Type | API 错误应保持原始 JSON；修复代理改写 |
| 页面报 `invalid_response_shape` | 核对服务与内嵌 Web 的发行身份 | 使用同一已验证发行树，按 request ID 排查响应 |
| 数据库身份或完整性错误 | 停服后 doctor；核对配置和密钥来源 | 保留数据库和日志；由当前结构验证定位问题 |
| 出现锁冲突 | 检查服务和维护进程 | 同一数据库只运行一个服务；维护前等待它完全停止 |
| Client 一直离线 | Client `doctor --network`，检查 `/xscc/v1/connect` 代理 | HTTPS 信任与 WSS Upgrade 均成功后连接可恢复 |
| Client 在线但 Sunshine 不可用 | Client `doctor --sunshine` | 检查本机回环 HTTPS、Sunshine 版本与凭据 |
| 配置显示待重启 | 查看保存任务结果 | 在合适时间重启 Sunshine，再读取配置及服务状态 |
| 任务结果 unknown | 查看操作 ID、Client 执行记录和 Sunshine 实际状态 | 人工确认结果；页面刷新或再次登录不会取消既有任务 |

需要完整诊断命令见[日常运维](administration.md)。分享 `identity`、脱敏错误和所做检查即可，凭据、原始配置与数据库保持私有。
