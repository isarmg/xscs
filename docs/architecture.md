# 实现边界

xscs 管理 Web、控制端/被控端实例、定向控制授权、被控地址、设备绑定、凭据撤销、持久任务、审计与 SQLite。公共认证、运行目录、SQLite 和服务生命周期继续使用固定版本的 xcss。

xscc 是嵌入 RustDesk 的 Rust 库。RustDesk 提供已有 Tokio runtime、身份与受保护凭据、NativeHost、NativeJournal 和 shutdown；不存在独立客户端服务或本机 HTTPS/IPC 适配层。媒体和对等连接认证由 RustDesk 自己负责。One-KVM 是另一种被控端实现，设备和浏览器控制端分别绑定实例，原生 XSCC 控制端可按同一授权关系访问 KVM；边界见 [One-KVM 接入](one-kvm.md)。

协议唯一源码为 `crates/protocol`，当前 `xscs-management/2`、`xscs-task/2`、WSS 子协议 `xscs-management.v2`。支持 RustDesk / One-KVM backend、embedded 执行方式和 ReadDiagnostics。未接入时明确报告 awaiting_integration；测试替身不代表可用的原生集成。

硬件、编码、画质、帧率等具体设置由 RustDesk 主控端在原生会话内选择并与被控端协商，服务端不保存、修改或下发这些设置。控制授权列表不能代替原生会话的加密身份验证。

数据库 schema revision 4（application_version=v4），设备保存不可变的角色与 backend、结构化诊断和被控地址，device_access 保存唯一一份控制端到被控端的定向关系。运行时不自动迁移旧状态；指纹与 metadata 精确匹配的 v3 数据库可使用[显式离线迁移工具](one-kvm.md#数据格式与升级)备份并升级，未知旧结构拒绝启动和迁移。产品 DDL 是 `schema/product.sql`，生成文件由固定 xcss schema-compose 工具生成，指纹采用 xcss 对 sqlite_schema 的规范算法。
