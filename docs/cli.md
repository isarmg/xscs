# 运行入口

业务程序仅负责加载既有安装并运行，入口为 `xscs --root ROOT`；ROOT 必须是显式、规范化的物理绝对路径。`--help`、`--version` 不访问安装状态。省略 ROOT 明确失败；运行程序不提供安装、初始化、升级、离线检查或密码重置命令。

默认安装位置 `/opt/isarmg/xscs` 由独立 [XSSI 安装维护工具](https://github.com/isarmg/xssi)选择，业务程序没有默认目录。程序读取 `ROOT/config/config.json`，关键配置、数据库或安装密钥缺失时停止，不猜测首次安装。

正常启动与停止由 `systemctl start xscs.service`、`systemctl stop xscs.service` 管理，安装后需由管理员手动启动。只读检查和离线密码恢复使用 XSSI 的 `maintenance.sh`；升级使用 `upgrade.sh` 调用 XSSC。其完整参数、互斥和恢复步骤由工具仓库维护，见[运维](operations.md)。
