# One-KVM 实例接入

One-KVM 是 XSCC 被控端的一种实现。XSCS 管理实例身份、授权关系和只读诊断；视频、HID、ATX、虚拟媒体和串流参数继续由 One-KVM 设备执行。管理服务不下发硬件配置或控制任务。

## 实例与授权码

管理员创建实例时选择角色和类型：

| 用途 | role | backend |
| --- | --- | --- |
| 现有 XSCC 原生控制端或原生被控端 | `controller` / `controlled` | `rustdesk` |
| One-KVM 设备 | `controlled` | `one_kvm` |
| One-KVM 浏览器控制端 | `controller` | `one_kvm` |

创建 API 继续使用 `POST /api/v1/rustdesk/devices`。例如 `{"name":"机房 KVM","role":"controlled","backend":"one_kvm"}`。省略 backend 保持原有 RustDesk 行为。角色和 backend 创建后不可变；更换类型需创建新实例，旧实例和审计由管理员保留或删除。

原生 XSCC 控制端和 One-KVM 浏览器控制端都通过同一授权编辑器获得允许连接的 One-KVM 实例。授权要求角色相反，不要求 backend 相同；授权默认拒绝，列表仍只包含已绑定对端。服务端不把 KVM 类型作为绕过原生证明、密码或授权检查的许可。

浏览器分别绑定控制端实例，KVM 后台绑定被控端实例。One-KVM 后端持有管理凭据并维护 HTTPS/WSS；浏览器只使用同源应用会话，不直接访问拒绝 Origin/cookie 的设备管理接口。初始化、HTTP 会话与流式连接的具体权限边界由 One-KVM 实现和验收。

## 设备协议与兼容

管理 namespace、协议版本、Binding、AccessPolicy 和 AccessPeer 的字段保持原有 v2 合同。KVM 在 Hello 中报告 `backend: "one_kvm"`，Linux ARM64 报告 `os: "linux_aarch64"`，ARMv7 报告 `os: "linux_armv7"`；这两种平台只由 One-KVM backend 报告，不扩展原生 XSCC 的支持平台。Hello 的 backend 必须与该实例固定类型一致，移动端被控角色仍被拒绝。

RustDesk 的 pairing 响应保持原字段形状；KVM pairing 额外返回 `backend: "one_kvm"`，供 KVM 在消耗绑定前核对。旧 XSCC 对误填的 KVM 授权码会拒绝解析，原生控制端应绑定自己的 RustDesk 控制端实例，再经授权关系访问 KVM。

KVM 诊断使用 `one_kvm_version`，`rustdesk_version` 保持 null。原生 RustDesk 响应不输出新增的空字段。就绪状态必须来自实际宿主观察并携带该 backend 的真实版本；服务端拒绝与实例类型不一致的心跳或结果诊断。只读诊断沿用现有任务合同，不增加 KVM 配置写任务。

授权每五秒刷新、租约三十秒；断线、撤权、解绑和凭据轮换继续使旧代际失效。配对密钥派生、双方安装绑定及会话证明算法保持不变，密钥不进入管理 Web API。

## 数据格式与升级

本开发工作树使用 schema revision **4**、application_version **v4**。新列 `devices.backend` 和不可变触发器纳入规范 schema 指纹；`release.json`、运行时定义和安装材料生成器共同引用新身份。软件版本号和 v2 设备协议没有因此被声明为已发布新版本。

运行时仍会在写入前拒绝 v3 及更早身份，**不会在服务启动时自动迁移或覆盖旧库**。已有 v3 环境可以使用专门的离线命令升级，并保留实例、安装绑定、凭据、授权、管理员、任务和审计。

1. 停止旧服务并让 SQLite 正常 checkpoint/关闭，保全完整 ROOT，尤其是 `state/secrets.json`。存在 WAL / SHM / journal 时工具拒绝运行；不要为绕过检查删除这些文件。
2. 在私有备份目录选一个尚不存在的备份文件，显式执行：

   ```sh
   python3 scripts/migrate-v3-v4.py \
     --database /absolute/ROOT/state/xscs.sqlite3 \
     --backup /absolute/private-backups/xscs-before-kvm-v3.sqlite3
   ```

3. 命令核对旧 v3 metadata、规范 schema 指纹、完整性和外键，取得服务使用的维护锁，先生成 0600 的一致性 SQLite 备份并同步，再在事务中重建规范 `devices` 表，为所有既有实例补 `backend=rustdesk`、恢复触发器并更新 v4 identity。备份不覆盖既有文件；迁移不修改凭据和密钥材料。
4. 新指纹、完整性和外键验证成功后提交，再使用新二进制在同一 ROOT 启动。原生实例无需重新注册，授权关系和既有持久任务仍可恢复；新增 KVM 实例再选择相应类型。

直接 `ALTER TABLE ADD COLUMN` 生成的 sqlite_schema SQL 与规范 v4 DDL 不一致，因此工具在独占维护状态下事务重建单张表。失败时回滚，旧备份保留；重复对 v4 执行明确拒绝并保持原数据。坏 schema 在任何文件写入之前拒绝。工具只支持确切 v3 来源，v1/v2 或未知结构需另行维护，不猜测迁移。

也可选择在新的 ROOT 初始化独立 v4 环境进行验收；新环境有新的 manager_id，不自动接管旧凭据、绑定或历史。旧 ROOT 与备份的删除属于另行授权的运维操作。

## 验证边界

协议测试覆盖旧 RustDesk 序列化形状与 KVM 真实版本、ARM64 / ARMv7。授权测试覆盖原生/浏览器控制端到 KVM、双方配对密钥和撤权；回环 WebSocket 测试覆盖 KVM Hello、地址更新和实时撤权；旧 v3 身份拒绝测试检查目录字节保持不变；迁移测试使用真实原生实例的加密授权码、管理凭据、配对授权和持久请求验证保留能力。

管理服务测试不替代 One-KVM 的采集、HID、BIOS 画面、ATX、虚拟媒体、原生 XSCC 音视频或物理设备验收。

2026-10-26 本地验证：工作区 314 项测试通过、0 失败，基础库 3 个专用子进程 helper 按既有设计标记 ignored；管理 Web 14 项纯逻辑测试通过。Clippy 工作区全 target `-D warnings`、格式、schema-compose 和发行边界 11 项检查通过。显式迁移另有 5 项 Python 行为测试及工作区内真实加密凭据/任务保留测试通过；迁移未用于任何既有部署 ROOT。

管理 Web 已从当前源码生成真实 release WASM 与资源，构建输入哈希逐项匹配。隔离的新 v4 进程已验证 readyz 与管理员登录。ARM64 / ARMv7 合同接受测试不代表本轮完成交叉构建或实机验收；本记录不声明新的发布、推送或部署。
