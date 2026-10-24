# 运行入口

业务程序仅负责加载既有安装并运行，入口为 `xscs --root ROOT`；ROOT 必须是显式、规范化的物理绝对路径。`--help`、`--version` 不访问安装状态。省略 ROOT 明确失败；运行程序不提供安装、初始化、升级、离线检查或密码重置命令。

安装工具位于本仓库的 [`tools/installer`](../tools/installer/README.md)，发行包同时提供相同入口。文档示例使用 `/opt/isarmg/xscs`；安装和业务程序均要求显式 ROOT。程序读取 `ROOT/config/config.json`，关键配置、数据库或安装密钥缺失时停止，不猜测首次安装。

正常启动与停止由 `systemctl start xscs.service`、`systemctl stop xscs.service` 管理，安装后需由管理员手动启动。只读检查和离线密码恢复使用包内 `maintenance.sh`；升级使用包内 `upgrade.sh`。完整参数可由对应脚本的 `--help` 查看，互斥和恢复约定见[运维](operations.md)。
