# 环境与首次运行

Server 只支持 Linux x86_64 GNU，业务程序和管理 Web 同包交付。安装必须使用对应发行包和独立 [XSSI](https://github.com/isarmg/xssi)，执行前核验 SHA-256；不得从源码目录直接调用旧初始化命令。

准备产品[配置](../configuration.md)，工具默认根为 `/opt/isarmg/xscs`。安装建立当前管理员、数据库、密钥和 systemd 配置，不自动启动或启用服务；管理员确认有效配置后手动启动 `xscs.service`，以 `/readyz` 和管理登录核对结果。访问生产管理页面需浏览器 HTTPS 来源。

业务运行入口和权限前提见[运行入口](../cli.md)及[运维](../operations.md)，当前数据缺失或损坏明确失败。开发者构建与隔离测试见[开发指南](../development.md)，不能把开发构建视作正式发行。
