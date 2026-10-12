# xscs 使用文档

xscs 为管理员提供自托管 Web 管理台，服务端运行于 Linux AMD64 GNU。每台受管主机安装 [xscc](https://github.com/isarmg/xscc/blob/main/docs/README.md)。本文档对应当前 1.0.3。

## 第一次使用

1. [安装服务端](operations.md)：下载发行包、配置、初始化并启动。
2. [连接一台主机](instance-management.md)：创建实例，安装并配对客户端。
3. [使用管理台](usage.md)：查看状态并完成日常操作。

## 按任务查找

- [配置](configuration.md)：环境变量、默认值与修改方法
- [日常运维](administration.md)：服务、日志、容量和账号维护
- [排查问题](troubleshooting.md)：根据现象选择检查
- [命令参考](cli.md)：初始化、校验、状态和诊断
- [本地运行](local-service.md)与[开发指南](development.md)：构建、测试、打包
- [实现概览](architecture.md)与[学习路线](beginner-guide/README.md)：理解协议和持久化
- [功能范围](feature-inventory-and-tradeoffs.md)、[公共库](common-support.md)、[仓库关系](repository-boundary.md)与[原生接口审查](unsafe-audit.md)
- [1.0.3 发行说明](releases/1.0.3.md)、[1.0.2 历史发行记录](releases/1.0.2.md)、[1.0.1 历史发行记录](releases/1.0.1.md)

管理接口见[协议参考](client-management-v1.md)，任务结果见[通信可靠性](communication-reliability.md)，原生验证见[验证范围](client-release-acceptance.md)。
