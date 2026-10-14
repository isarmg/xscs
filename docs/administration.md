# 管理员与凭据维护

Web 菜单右侧人物图标提供当前管理员账号设置，可更改用户名和密码。身份、会话、审计及 Argon2id 密码哈希位于主 SQLite 数据库的公共管理表；安装密钥与实例授权属于产品私有状态，不由改密码自动重建。

忘记密码时使用独立 [XSSI](https://github.com/isarmg/xssi)离线维护入口，在全部写入者已退出且启动受控后检查当前契约并重置。重置撤销先前管理员会话，保留实例身份、`data_dir/secrets.json` 及加密授权。

运行状态、UTC 日志和完整环境备份见[运维](operations.md)，目录权限见[配置](configuration.md)，正常账号修改见[账号设置](account-settings.md)。业务程序不包含安装或离线维护命令。
