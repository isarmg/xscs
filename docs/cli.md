# 命令行

全局参数：--config、--data-dir、--bind、--development true/false、--log-level、--json。

- init --username admin：一次初始化，密码stdin，自动建立私有密钥
- run：运行已有完整状态，绝不隐式初始化
- config validate：纯配置校验，不要求DB或密钥存在
- check：只读检查状态、密钥、数据库和资源
- status：查询live readiness
- admin reset-password --username admin：停服后从stdin重置密码
- identity、web-assets：输出二进制和内嵌资源身份
- verify-release --root PATH：显式验包，不是run前提

运行及部署细节见[运维](operations.md)。
