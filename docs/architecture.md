# xscs 实现概览

xscs 将业务服务和管理 Web 放在一个 Linux AMD64 GNU 可执行程序中，以单个 SQLite 数据库保存状态。xscc 在受管主机独立运行，协议类型由本仓库维护。

## 从哪里读源码

| 目录 | 职责 |
|---|---|
| `crates/protocol/` | 通信、任务、配置字段与结果类型 |
| `crates/server/src/` | HTTP、配置、操作调度、持久化和秘密上下文 |
| `crates/server/tests/` | 路由、存储、认证、并发与发行行为测试 |
| `schema/` | 当前 DDL 与生成的结构定义 |
| `web/src/` | 产品页面、API 调用与响应校验 |
| `scripts/` | 本地启动、构建、检查和打包 |
| `deploy/` | systemd及设备反向代理示例 |

根 Cargo.toml 统一 workspace 依赖和 lint，Cargo.lock 锁定依赖图。公共配置、认证、日志、SQLite 和生命周期使用 xcss；管理页面使用单个 @xcss/web 包。产品继续维护自己的业务模型和持久条件，见[公共库](common-support.md)。

## 一条请求经过什么

浏览器通过 Session/CSRF 提交任务，Server 持久化后派发给 Client。Client 在副作用前保存执行意图、完成后保存结果；Server 收存结果再确认。WSS 断开后双方以持久事实继续，而非从页面状态推断结果。

通信身份为 xscs-management/1，WebSocket 子协议为 xscs-management.v1；持久业务任务身份为 xscs-task/1。任务指纹涵盖协议、实例绑定、权限和命令，通信能力由 Client Hello 声明。详见[通信可靠性](communication-reliability.md)。

## 状态与资源

XSSI 脚本处理主机安装与升级，独立签名 setup 通过 `offline.rs` 创建首个管理员和数据库。业务 binary 只接受 `--root ROOT` 运行，验证当前结构和已有密钥后启动；配置与数据检查均只读，不兼容旧配置/结构，也不自动初始化。发行签名通过 XSSI 绑定同次构建的源码、server、setup、资源和统一服务模板。

构建、测试、发行步骤及验证范围见[开发指南](development.md)。Rust 原生接口结论见[unsafe 审查](unsafe-audit.md)。

服务启动先检查已有目录、密钥和数据库文件，再取得实例与业务数据锁，最后只捕获一次位于 ROOT/run 的私有 SQLite 快照。在同一数据库 generation 上验证当前结构、完整性、外键、管理员和实例授权，校验完成后才打开生产数据库。只读检查使用同一流程，结束后清理快照，不写源数据。

启动校验逐行读取设备，在同一行上检查配置观测、能力、授权密文与对应摘要，不把全部设备视图或全部密文同时装入内存；操作记录按固定批次读取标识并逐条检查。
