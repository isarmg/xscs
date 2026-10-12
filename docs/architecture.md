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

`init` 创建私有数据库与首个管理员；`run` 验证当前结构后运行。config validate仅验证配置；check使用独立只读快照验证状态。密码维护在停服窗口取得排他维护锁。正式发行通过 manifest 绑定源码、二进制、资源和权限。

构建、测试、发行步骤及验证范围见[开发指南](development.md)。Rust 原生接口结论见[unsafe 审查](unsafe-audit.md)。
