# 实现边界

xscs 管理 Web、设备绑定、凭据撤销、持久任务、审计与 SQLite。公共认证、运行目录、SQLite 和服务生命周期继续使用固定版本的 xcss。

xscc 是嵌入 RustDesk 的 Rust 库。RustDesk 提供已有 Tokio runtime、身份与受保护凭据、NativeHost、NativeJournal 和 shutdown；不存在独立客户端服务或本机 HTTPS/IPC 适配层。媒体和对等连接认证由 RustDesk 自己负责。

协议唯一源码为 `crates/protocol`，当前 `xscs-management/2`、`xscs-task/2`、WSS 子协议 `xscs-management.v2`。仅支持 RustDesk、embedded 执行方式和 ReadDiagnostics。未接入时明确报告 awaiting_integration；测试替身不代表可用的原生集成。

数据库 schema revision 2，设备保存结构化诊断而非远程配置快照。旧状态需要保全，当前分支不提供自动迁移。产品 DDL 是 `schema/product.sql`，生成文件由固定 xcss schema-compose 工具生成，指纹采用 xcss 对 sqlite_schema 的规范算法。
