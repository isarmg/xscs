# xscs 使用文档

所有日期、时间输入、显示与日志日界统一使用 UTC，页面明确标注 UTC；一天固定为 24 小时，不随服务器或浏览器时区、夏令时改变。无时区的 SQLite 时间及日期输入也按 UTC 解释。

xscs 为管理员提供自托管 Web 管理台，服务端运行于 Linux AMD64 GNU。每台受管主机安装 [xscc](https://github.com/isarmg/xscc/blob/main/docs/README.md)。本文档对应当前 1.0.0。

## 第一次使用

1. [安装服务端](operations.md)：核验发行包、通过 XSSI 安装，再手动启动。
2. [连接一台主机](instance-management.md)：创建实例，安装并配对客户端。
3. [使用管理台](usage.md)：查看状态并完成日常操作。

## 按任务查找

- [配置](configuration.md)：环境变量、默认值与修改方法
- [日常运维](administration.md)：服务、日志、容量和账号维护
- [排查问题](troubleshooting.md)：根据现象选择检查
- [命令参考](cli.md)：唯一运行入口及工具职责
- [部署运维](operations.md)与[开发指南](development.md)：构建、测试、打包
- [实现概览](architecture.md)与[学习路线](beginner-guide/README.md)：理解协议和持久化
- [功能范围](feature-inventory-and-tradeoffs.md)、[公共库](common-support.md)、[仓库关系](repository-boundary.md)与[原生接口审查](unsafe-audit.md)
- [当前 1.0.0 发行说明](releases/1.0.0.md)

管理接口见[协议参考](client-management-v1.md)，任务结果见[通信可靠性](communication-reliability.md)，原生验证见[验证范围](client-release-acceptance.md)。
