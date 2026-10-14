# Server / Client 边界

本仓库只构建 Linux x86_64 Server，管理 Web 位于 `web/`。产品协议唯一源码位于 `crates/protocol/`。
客户端独立仓库为 https://github.com/isarmg/xscc ，通过完整 Git 提交固定协议依赖。
客户端不需要检出或构建 Server 可执行程序；Server 的常规构建也不需要客户端源码。

当前 Server 开发版本为 1.0.1，Schema revision 为 1。设备通道及业务字段统一使用 Client 命名，
Server 校验当前接口与数据库身份，不自动转换不匹配状态。发行标签保持不可变。

跨仓库端到端测试保留在 `web/tests/client-end-to-end.mjs`。先在独立 Client 仓库构建所需精确提交，
然后设置 `SUNSHINE_TEST_CLIENT_BINARY` 为该可执行文件的绝对路径；在本仓库执行 `npm --prefix web ci --ignore-scripts`，
再执行 `web/node_modules/.bin/xcss-build-server --config xcss-web-build.json --mode development --no-install`，
进入 `web` 执行 `node tests/client-end-to-end.mjs`。
使用自定义 Cargo 输出目录时，另以 `SUNSHINE_TEST_SERVER_BINARY` 指定 Server 可执行文件绝对路径。
该测试使用临时数据库、临时证书和本机 Sunshine HTTPS 测试替身，不操作已安装 Sunshine。
真实 Sunshine 验收必须另外记录源码提交、版本、平台及受控重启授权。

版本验收记录：Server `d9103beec4d11d97f63fcfc866b709fad9057007`、
Client `2c2b291`，使用本页说明的显式输出目录选项。覆盖网页提交、持久任务、严格 WSS、
配置合并保存、审计、离线投递、完整修订冲突、不确定重启与重连、凭据撤销。
这是 Sunshine HTTPS 替身验收，不代替真实 Sunshine 双平台运行验收。
