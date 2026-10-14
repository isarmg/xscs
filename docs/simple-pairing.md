# 简化配对（Server 1.0.0 / Client 1.0.0）

Client 只需填写 Server 地址、36 位小写英文字母数字实例授权码、本机 Sunshine HTTPS 回环地址及账号密码。Manager ID、
设备 ID 由授权码查询返回。Client 与 Server 需同时部署支持当前 v1 协议的版本。

`POST /xscc/v1/pairing` 接受唯一字段 `authorization_code`，返回 `manager_id`、`device_id`。
该入口采用 Client 的可信 HTTPS 限制、无 Cookie/Origin、16 KiB 请求上限和 `Cache-Control: no-store`。
查询不消费授权码；仅匹配待配对、未撤销且未取消的实例摘要。错误授权码统一拒绝，不回显秘密。
真正注册仍通过 `/xscc/v1/enroll` 原子绑定独立设备凭据；授权码不会被删除。查询后发生取消、撤销或授权码轮换会使注册失败。
客户端在注册前持久化独立随机凭据，回执丢失仍使用原有 identity 查询恢复，不创建另一套任务状态机。Server 更换授权码会撤销当前凭据；Client 先用新码验证仍为原实例，再通过显式 `pair replace` 重新绑定。

Manager HTTPS/WSS 始终使用系统信任库并验证名称；Windows LocalSystem 服务使用计算机信任上下文。本机
Sunshine 地址则必须是 `https://127.0.0.1:<port>/` 或等价的 IPv6 回环 IP 字面量，Client 不校验这条本机
连接的证书身份，以适配 Sunshine 默认自签名证书。请求不使用系统代理、不跟随重定向，也不接受主机名、
非回环地址、URL 凭据、查询参数、片段或额外路径；每个请求仍使用 Sunshine 用户名和密码认证。

Windows Client 为纯 CLI 与系统服务，不含托盘。Sunshine 重启与停止 Client 服务是两个操作；Client 仍独立于
Sunshine 进程，不修改视频串流链路。
