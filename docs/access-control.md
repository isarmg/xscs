# 实例、授权与地址合同

管理员创建实例时必选 `role: controller | controlled`，角色创建后不可变。控制端与被控端各绑定一个同角色实例；安装身份在同一服务端唯一，不能同时绑定两个实例。授权默认拒绝。

硬件、编码器、编码格式、画质、帧率等设置由 RustDesk 主控端选择，并由 RustDesk 与被控端协商支持范围。xscs 只管理身份、绑定、控制关系、连接地址和只读诊断，不保存或下发这些具体设置。

## Web 与管理员 API

- `POST /api/v1/rustdesk/devices`：`{"name":"主控机","role":"controller"}`；name 可省略，role 必须给出。
- `GET /api/v1/rustdesk/devices/{id}/access`：返回 `peer_ids`、`revision`。
- `PUT /api/v1/rustdesk/devices/{id}/access`：提交 `peer_ids` 和从 GET 获得的 `expected_revision`，原子替换当前实例的全部关系。过期 revision 返回 409，重新读取后再保存。

控制端页面选择“允许控制的被控主机”；被控端页面选择“允许连接的控制端”。两侧操作同一张定向关系表，不要求重复批准。只允许角色相反、未撤销且存在的实例；可先配置未绑定实例，客户端列表仅下发已绑定的对端。禁止自身、重复 ID 和同角色授权，每个实例最多 32 个对端，写入时检查关系两侧容量。删除实例级联删除关系；凭据撤销/重新绑定期间不下发该实例。授权规则属于实例，重新绑定后继续沿用，列表中的 installation_id 随新绑定更新。

接口沿用管理员会话、同源和 CSRF 校验，授权修改写入审计。客户端无权修改角色、关系或其他实例地址。

## 被控端地址

客户端在已有 WSS 上发送 `{"type":"heartbeat","diagnostics":null,"addresses":[{"host":"host.example.com","port":21118},{"host":"2001:db8::10","port":21118}]}`。

host 是规范 IPv4、无方括号的规范 IPv6，或小写 ASCII DNS 名（国际域名先转 punycode）；port 是 1–65535。最多四个不重复地址，禁止 URL、路径、带端口的 host、未指定/组播 IP。上报的是主机明确提供的可连接地址，服务端不猜测公网地址、不解析 DNS 或探测网络；不保证穿透 NAT 或地址可达。主控端不能上报被控地址。

每个心跳整体替换地址，空数组撤回旧地址。服务端从当前已认证安装及会话确定上报者，不接受指定目标实例的地址更新。每次重新连接清除旧观察，地址时间超过 45 秒或主机离线时，控制端列表里的 addresses 为空。Web 可查看上次上报及时间，不能修改地址。

## RustDesk 获取列表

`GET /xscc/v2/access` 使用绑定后的 Bearer 管理凭据，返回自己的 `binding`、`role`、`valid_for_ms:30000` 和 `peers`。每个 peer 含完整 `binding`（manager/device/installation）、名称、相反角色、online 和 addresses；控制端获取获准控制的被控端，被控端获取获准接入的控制端。仅返回直接授权关系，不返回全局设备清单、管理凭据或授权码；响应 no-store，不接受浏览器 Origin/cookie。

现有 WSS 在 Hello 后及每 5 秒发送 `{"type":"access_policy","policy":{...}}`，因此在线实例无需重连即可看到关系、绑定、名称、地址和在线状态变化。删除、撤销或凭据更换会使旧客户端失效；其他客户端下次刷新会移除无效对端。HTTP 和 WSS 返回同一份数据库快照。

xscc 暴露 `ManagerConnection::access_policy()` watch 接收器。宿主每次使用观察时调用 `AccessObservation::policy()` 或 `permits(&authenticated_binding)`；使用单调时间检查 30 秒租约，断线、撤销或正常退出立即清空观察。宿主即使保留旧观察副本也不能越过租约。正常网络下策略变更在下一次 5 秒刷新传播；断线不能继续信任历史列表。

列表提供管理授权事实，不是可直接呈交给另一台 RustDesk 的签名票据。RustDesk 稳定后的原生集成必须认证对端安装身份、校验最新授权、拒绝未授权会话，并关闭不再授权的既有会话；不能以来源 IP、域名、实例名或自报 UUID 证明身份。本次只修改 xscs/xscc，不表示已实现 RustDesk 原生会话强制执行。

## 状态与升级

当前产品 schema revision 3、application_version v3。DDL、生成文件、release identity 和测试夹具同步更新；没有 v1/v2 自动迁移。保留旧状态，正式升级时再设计显式迁移。设备协议仍为准备分支的 v2，服务端和 xscc 必须配套升级。
