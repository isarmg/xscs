# 配置与生效

`xscs --root ROOT` 读取 `ROOT/config/config.json`。ROOT 只能显式传入绝对路径，安装工具的默认位置为 `/opt/isarmg/xscs`。JSON 拒绝重复键、未知字段及不符合当前契约的类型与业务值。

[配置样例](../config/config.json.example)及 `crates/server/src/config_definition.rs` 是产品字段、默认值和校验的来源。`data_dir` 默认 ROOT/state；明确设置物理绝对路径后，私有数据库和配套密钥可以整体外置。ROOT/logs、ROOT/run 仍在安装根内；状态不得与程序、配置、日志、运行或维护目录重叠。

配置由管理员控制：直属目录 root 所有、0700 或 0750，文件 root 所有、0600 或 0640、单硬链接且与父目录同组；不得有额外访问或默认 ACL。服务账号按实际组权限只读。运行程序不会为了加载文件而改属主、权限或创建文件；管理员修改后需重启服务。

`bind` 选择监听地址，`log_level` 选择结构化日志等级。`development` 用于显式回环 HTTP 开发模式；生产管理会话要求浏览器 HTTPS 来源。只有未绑定发行源码的 debug 构建允许使用开发目录 Web 资源和开发用户所有的私有配置。

产品前缀环境变量的明确映射见 `crates/server/src/config.rs`，优先于 JSON；命令行只选择 ROOT。包内工具的只读维护核对当前材料和实际有效配置，诊断不输出凭据。业务程序没有代理或访客 IP 配置。
