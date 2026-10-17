# Leptos 业务数据强类型化

本次实际接入 DTO 的路径：设备列表/详情、创建票据、授权码读取、权限读取与写入、任务历史和诊断提交回执。原来在这些组件之间传递的 `Value` 已换为结构体，UI 直接访问字段。

设备状态采用 `Device`，角色与任务状态采用枚举。时间戳先保存在 `serde_json::Number`，保持既有接口接受的整数/浮点及 null 范围，直到 JS Date 显示时才转换。任务读取成功且回执通过严格合同后才清除 Idempotency-Key。

所有 typed loader/mutation 都先调用原来的 `contracts::validate` / `valid_mutation_response`，再构造 DTO。保留路径与查询参数绑定、revision、页数/条数限制、nullable、未知键策略及整数边界。`models.rs` 不是新增服务端协议。原认证、取消和轮询生命周期沿用原实现；响应不合法继续映射 `platform.invalid_response`。

结构化请求经过 `encode_json_body`，错误显式显示且不发送。分页支持 typed page 的 `PageCursors`（适用时），不再为分页把整页 DTO 转回 JSON。

原生校验命令：

```sh
CARGO_BUILD_JOBS=2 cargo test --manifest-path web/leptos/Cargo.toml --lib --offline
```

新增单测覆盖实际 DTO 与既有严格守卫组合，而不是仅测试 derive 能否反序列化。浏览器回归与发布构建由本轮总体验证记录统一列出；本文件不将原生测试等同于浏览器验收，也不声称存在未测量的性能加速。
