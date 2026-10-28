# 避免过度防御性编程

本规范参考用户提供的 Rust 开发规范及 ZXS 审查报告，按 XSCS 的管理服务边界取舍。ZXS 的存储引擎、SMB 和发布协议方案不直接适用于本项目。

## 开发规则

1. 在 HTTP / WebSocket、配置和持久化数据入口解析外部输入，内部沿用类型保证。
2. 查询只读取和解析需要的领域数据。授权列表不应依赖无关诊断快照的完整投影。
3. 撤权、会话所有权和授权租约是可变事实；保留当前状态复核及事务边界，不能因已验证输入就省略。
4. 数据库失败、数据损坏和记录不存在分别处理，保留错误来源；禁止把查询异常统一报成“实例不存在”。
5. 用到的持久化字段只解析一次，损坏时保留来源并返回错误；无需使用的字段直接跳过读取，避免先读入再通过 `.ok()` 丢弃解析错误。
6. 优先借用或 move，只复制输出需要的字段。新增缓存、trait、后台巡检或状态机必须有实际需求。
7. `checked_*`、`saturating_*`、`.ok()` 和 `validate_*` 只是审阅线索，依据调用链逐项判断；保留认证、CSRF、origin、路径、FFI、资源上限与持久化顺序。

## 信任边界

| 入口或事实 | 验证与使用方式 |
| --- | --- |
| 管理员 HTTP 请求 | 保留认证、同源 / CSRF、请求大小及 DTO 校验 |
| 设备 HTTPS / WSS 消息 | 保留凭据、绑定、会话归属、消息大小及协议验证 |
| 数据库记录 | 使用字段时按持久化边界解析；损坏与缺失分别返回错误 |
| 运行期授权 | 从当前事务读取撤权、凭据和会话状态，再生成租约策略 |
| 执行结果 | 持久提交成功后确认；历史最终结果与后到观察分别保存 |

## 本轮改造

授权策略构造直接使用角色、在线状态和地址，移除 `Device::view()` 带来的诊断与 capabilities JSON 解析。本端角色和凭据取出一次，循环中复用；对端名称直接 move。完整设备列表仍验证它实际消费的诊断字段。

轮换授权码时，设备查询的数据库错误向上传播；只有实际缺失才返回 `NotFound`。最终结果重放先比较已持久化的权威结果；相同时直接沿原事务完成幂等确认，省去历史观察查询。结果不同时才读取既有观察并解析一次，损坏时保留错误。

授权列表仍遍历 P 个授权对端，移除了每个对端的诊断 / capabilities 反序列化和完整视图构造。SQL 查询仍读取完整设备行，因此数据库读取与字符串分配仍受这些字段大小影响；字段投影可另行评估。数据库/schema、消息协议、授权刷新周期和结果持久化顺序保持原有合同，本轮没有新增 fsync 或后台任务。以上是源码复杂度分析，不是延迟或吞吐量实测。

既有运行路径的改动范围：

| 生产文件 | 运行路径与必要性 |
| --- | --- |
| `crates/server/src/access.rs` | 授权策略构造与地址读取，避免完整设备视图和本端角色的重复解析 |
| `crates/server/src/db.rs` | 复用在线状态计算；设备视图将持久化诊断损坏报为内部错误；授权码轮换保留数据库查询错误 |
| `crates/server/src/operations.rs` | 最终结果重放按需读取既有观察并解析一次；保持原有相同结果确认、不同结果比较和错误语义 |

`http.rs` 仅扩充既有回环 WebSocket 行为测试，验证诊断损坏与 ACL 的独立性，以及实时撤权。

## 审阅与验证

每次修改说明输入来源、验证位置、依赖的类型保证、改变的既有运行路径及复杂度。测试断言 API 返回值与错误语义，复用现有 SQLite 和回环 WebSocket 夹具。

先按[开发说明](development.md)准备嵌入资源，再在仓库根目录运行：

```sh
cargo fmt --all -- --check
cargo test --locked --workspace --all-targets -- --test-threads=2
cargo clippy --locked --workspace --all-targets -- -D warnings
git diff --check
```

现有 CI 配置包含格式、Clippy、协议和工作区测试。新增代码不能仅以关键字扫描作为通过或失败依据。Linux 自动化测试、浏览器验收、真实部署和客户端物理设备验收分别记录。

2026-10-26 本地 Linux 验证：当前前端源码经 Rust 1.99.0、wasm-bindgen 0.2.129 和本仓库构建工具离线生成真实 release WASM 与 dist，再运行 `cargo test --workspace --locked --offline -- --test-threads=2`。共 311 项通过、0 项失败；基础库另有 3 个专用子进程辅助测试按既有设置标记 ignored，由父测试使用隔离流程调用。最终 Clippy `--workspace --all-targets --locked --offline -- -D warnings`、格式检查和 `git diff --check` 均通过。

验证涵盖回环 WebSocket 的授权发布与撤权、诊断损坏和 ACL 的独立性、相同结果的幂等重放、真正需要比较时的损坏观察错误、真实服务进程重启保留状态，以及当前嵌入 WASM 的 MIME / gzip / HEAD / 304 行为。未执行本轮真实浏览器、跨平台客户端、物理设备串流或部署验收。


## 热路径与错误传播（2026-10-28）

本轮对照用户提供的 XSCS 审查会话及当前 `eba2a88` 基线，落实现有行为能够直接验证的局部改动。上文“SQL 仍读取完整设备行”的说明对应此前基线；当前授权刷新已使用窄投影。持久化数据、会话归属、撤权与发行来源继续按既有边界验证。

* `access.rs` 的策略投影从设备表全部 18 列减为实际使用的 10 列，避免读取授权码密文、诊断和 capabilities；角色、UUID、地址仍在消费时解析。授权、当前凭据、安装身份与 session 仍在同一读取事务内复核。
* WebSocket Hello 验证过的角色与 backend 由已有不可变表约束保证，在本会话内部复用。心跳先验证诊断及地址，再以 device、installation、session 和未撤销条件执行一次原子 UPDATE，同时写入 last_seen、health、地址及可选诊断。未携带诊断时保留既有快照。普通帧也通过条件 UPDATE 校验归属；周期检查使用 EXISTS。心跳从两次完整行读取、三个独立写事务减为一次条件写入，保持 SQLite FULL 同步和已接受结果先提交再确认的顺序。
* `operations.rs` 的私有 `DeadlineConnection` 保留 2 秒连接获取与 3 秒 SQLite VM 截止时间。完成后移除 progress callback 并 ping 冲刷 worker / rollback，再归还池连接；查询、清理或 ping 期间取消仍通过 Drop 关闭连接，不会把私有过期回调交给下一个请求。查询与清理同时失败时保留原查询错误。
* 维护周期保留阶段上下文与错误来源。仅对错误链中真实 SQLite BUSY / LOCKED 主码进行 50、100、200ms 三次有界退避；成功后恢复原 1 秒周期，停机可中断等待。foundation 已转换为字符串的 Storage 错误不按文本猜测类型，保留诊断并按原 Critical 失败边界退出。outbox sink 与 acknowledgement 同事务、租约恢复的状态 / owner 条件及 unknown-read 事务使部分完成后的重试保持幂等。
* WebSocket 发送、接收与超时保留来源；断线的任务与 presence 清理失败分别记录。管理 API 的对外错误信封继续脱敏。持久化 binding 的 installation UUID 损坏返回内部错误，真正未注册仍返回 Conflict。
* Web 挂载与功能 startup ready 不等待字体下载。字体状态独立为 pending / ready / failed，失败使用系统字体；字体完成由既有字体 / 尺寸观察器更新轮廓。启动失败、非 bfcache 的 pagehide 会取消迟到字体任务，bfcache 保留状态；WASM 下载 / 初始化失败和 30 秒超时仍显示既有失败与重试流程。保留原字体与许可证，未改 G4 曲线。

隔离的 SQL 窄投影微基准采用当前 DDL、32 个授权对端和典型合法 capabilities / diagnostic JSON，SQLite 3.46.1 内存热缓存，15 批、每批 300 次并交替执行旧 / 新查询。按文本 / BLOB 长度与每个整型 8 字节口径统计的返回值大小由 19,392B 减为 6,048B；单次查询及 Python 结果物化的中位耗时为 498µs / 309µs。它不包含 SQLx、HMAC、JSON 序列化、fsync、网络或多客户端并发，不能解释为 Rust / WebSocket 吞吐提升。两种查询计划都保留原设备与 ACL 索引及排序，本轮没有改 DDL 或 schema identity。报告所列的 63KiB 是协议外层上限，当前诊断 DTO 的版本字符串另有较小约束，基准没有使用虚构的大诊断负载。

变更的既有路径与回归面：

| 路径 | 必要性与行为回归 |
| --- | --- |
| `crates/server/src/access.rs` | 窄策略投影、地址上报角色投影、合并心跳；覆盖诊断缺省保留、控制端禁地址、backend 一致、旧 installation / session 与撤销后拒写 |
| `crates/server/src/db.rs` | 共享在线语义、条件 liveness / 当前 session 查询、损坏 binding 与未注册区分；token 沿既有 hex 编码输出 |
| `crates/server/src/http.rs` | 使用原子心跳与条件 presence 路径、保留传输和清理错误；真实回环 WebSocket 通过 SQLite 触发器计数验证一次心跳恰一次设备 UPDATE |
| `crates/server/src/error.rs` | 服务端受保护日志保留错误来源，现有 API 脱敏信封测试保持 |
| `crates/server/src/operations.rs` | deadline 连接正常复用 / 取消关闭，后台严格错误分类与可中断退避；真实 SQLite 连接身份、回滚、worker 执行中取消和锁竞争测试 |
| `crates/server/tests/access.rs` | 在现有持久化夹具验证损坏 UUID 与未注册响应区别 |
| `web/xcss-web/vendor/xcss-web/src/lib.rs`、`startup.js` | 非阻塞字体加载与取消所有权，功能就绪独立于字体状态 |
| `web/index.html`、`web/vendor/xcss/web-fonts/{boot,fonts}.css` | 功能启动时显示页面，字体采用 swap / 系统回退 |
| 两份 Web `SOURCE.json`、xcss-web `README.md` | 沿既有来源与受维护文件合同记录实际修改，保留许可证 |
| 两份 `startup-failure` 浏览器测试 | 真实字体请求延迟 / 503、迟到 pagehide / bfcache 与原 WASM 失败 / 超时恢复验收 |

2026-10-28 本地验证已完成：`cargo fmt --all -- --check`、`cargo test --locked --workspace --all-targets -- --test-threads=2`、`cargo clippy --locked --workspace --all-targets -- -D warnings` 和 `git diff --check` 均通过。工作区共 324 个顶层测试通过、0 失败，3 个既有专用子进程 helper 按既有设置 ignored；父测试启动的 helper 输出不重复计入 324。首轮测试发现 SQLx boxed DatabaseError 的 std source 包装不能直接 downcast 到 SqliteError，现使用公开 `as_database_error().try_downcast_ref()` 读取真实类型，原锁竞争断言通过。

Web 真实 release WASM / dist 构建通过（17 文件，12,543,475 字节），修改包的 WASM Clippy 与 Node 合同 5 项通过；Chromium 和 Firefox 各 7 项启动场景、两引擎完整管理功能与 G4 几何验收均通过。启动专项拦截真实字体请求，覆盖 503、持续延迟、pagehide 迟到取消及 bfcache；同时保留 WASM 503、损坏、30 秒超时与显式键盘重试。完整几何验收先显式等待字体就绪，以核对原字体下的几何。未改应用 Web crate 中 13 项既有 Clippy 问题保留为验证限制，没有宣称整个独立 Web workspace 的 Clippy 全绿；根工作区要求的 Clippy 已全部通过。

以上浏览器证据为隔离夹具；没有执行物理设备、服务器吞吐压测或部署验收。服务、用户数据库、实例配置与凭据均未改动；没有执行提交、推送、部署或发行操作。
