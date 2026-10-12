# 使用 Sunshine 管理台

先完成[服务端安装](operations.md)、[实例创建](instance-management.md)和 Client 配对，再用浏览器登录 HTTPS 管理入口。

## 确认管理链路

1. 选择实例，查看 Client 在线状态。
2. 读取 Sunshine 状态或配置，等待任务结果。
3. 确认 Sunshine 可访问。Client 在线与 Sunshine 可访问是两项独立状态。

读取任务成功后，即可编辑配置、应用、查看 Sunshine 日志与诊断。流媒体仍由 Sunshine 与 Moonlight 直接传输。

## 修改配置

1. 读取最新配置，编辑当前平台可管理的字段。
2. 检查保存预览，确认设置项和恢复默认的删除项。
3. 保存后查看任务结果；显示「待重启」时，在方便中断串流的时间重启 Sunshine。
4. 重新读取配置和服务状态；运行时效果仍需按该设置验证。

管理台保留未开放管理的本机设置。日常统一从管理台修改受管字段，可减少与本机 Sunshine Web UI 同时编辑的冲突。更多保存与修订语义见[远端管理参考](remote-management.md)。

## 管理应用和 Moonlight 配对

应用页可新建、编辑应用及上传不超过 30 KiB 的 PNG 封面。应用中的启动、准备和分离命令会在 Sunshine 主机执行，保存前检查其内容与执行影响。

在 Moonlight 选择 Sunshine 主机取得 PIN，再在该实例的 Moonlight 配对功能提交 PIN，完成后回到 Moonlight 验证连接。这里的串流配对与 xscc 连接 xscs 使用的实例授权码是两个步骤。上游操作说明见 [Moonlight Setup Guide](https://github.com/moonlight-stream/moonlight-docs/wiki/Setup-Guide)。

## 查看任务结果

「日志」页按UTC的创建日期筛选，可选择包含起止当天的范围，每页最多 50 条。任务被接受只表示已保存，等待 succeeded/failed 等最终状态。

unknown 表示效果未确认。先在设备上核对实际结果，再在管理台记录确认结论；刷新页面或退出不会撤销已接受的任务。所有任务（包括读取）从创建起 15 分钟内可派发，过期后重新读取资源再决定是否提交。

Windows 和 Ubuntu 可以控制已适配的 Sunshine 服务；macOS 提供配置与管理 API 能力，服务启停在本机处理。账号和实例密码维护见[账号设置](account-settings.md)与[实例管理](instance-management.md)。
