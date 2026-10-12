# 部署与运维

Linux x86_64 GNU 服务端提供普通 HTTP，Web 已内嵌。生产入口应提供 HTTPS/WSS，转发 Host、Origin 与 WebSocket Upgrade；不要求任何代理品牌或 X-Forwarded-Proto。后端仅向受控网络开放。

## 安装与升级

安装需要 Bash、Python 3、systemd 及普通账户管理工具。在解压的发行包根目录选择自己的配置，密码只通过stdin：

```bash
cp config/config.json.example ./my-config.json
${EDITOR:-vi} ./my-config.json
read -rs -p '管理员密码: ' password; printf '\n'
printf '%s\n' "$password" | sudo ./install.sh setup ./my-config.json admin
unset password
sudo systemctl enable --now xscs
sudo -u isarmg-xscs /opt/isarmg/xscs/bin/xscs status --json
```

唯一安装入口为 `install.sh setup CONFIG_JSON [USERNAME]` 或 `install.sh upgrade`。setup创建不可登录的isarmg-xscs普通系统用户与同名组，安装文件、复制所选配置、从stdin初始化并check；不自动启动。用户名默认admin。调用者环境变量不会覆盖安装所选JSON。

升级从新包运行 `sudo ./install.sh upgrade`：读取现有配置，原服务active时先停服，替换程序/unit后check，只在此前active时重启；不初始化、不修改配置/密钥/数据库。校验失败保持停服并报告错误，不用init覆盖状态。

布局：

- 程序 `/opt/isarmg/xscs/bin/xscs`
- 配置 `/etc/isarmg/xscs/config.json`，isarmg-xscs所有、0600；直接父目录0700
- data_dir默认 `/var/lib/isarmg/xscs`，也可选其他绝对目录；isarmg-xscs所有、0700
- 临时运行目录 `/run/isarmg/xscs`，仅systemd管理
- 数据库 `data_dir/xscs.sqlite3`；密钥 `data_dir/secrets.json`；日志 `data_dir/logs`

安装器根据所选data_dir生成systemd ReadWritePaths白名单，正确转义路径。父目录必须允许服务用户遍历；check以服务UID执行，安装器绝不递归chown业务树。ProtectSystem=strict保持，ProtectHome=read-only，不隔离临时目录中的显式自定义状态。

## 初始化与密钥

安装器调用一次 `init --username admin`，只接受新建或空私有数据目录。init自动生成32字节随机业务密钥，固定primary标识，原子写入0600 secrets.json，建立数据库与管理员。重复init拒绝已有数据。

run只加载现有密钥，缺失、损坏、权限错误或状态不完整均拒绝启动，绝不补新密钥。secrets.json为私有版本1文档，字段version、credential_key_id、credential_key；密钥采用规范Base64，标识必须primary。备份整个data_dir，不单独编辑或替换密钥文件。

## 配置

默认 `/etc/isarmg/xscs/config.json`。仅隐式默认文件不存在时使用环境和默认值；显式 --config/XSCS_CONFIG 缺失、权限或格式错误都报错。CLI > XSCS_ 环境 > JSON > defaults，每层均验证。

- bind：SocketAddr，默认127.0.0.1:18104；生产支持内网、0.0.0.0与[::]
- data_dir：唯一内部数据根，默认/var/lib/isarmg/xscs；绝对路径
- development：默认false；true仅允许回环HTTP，生产保持Secure Cookie与Origin/Host检查
- trusted_proxies：默认空的IP/CIDR字符串数组。仅明确可信连接对端的X-Forwarded-For参与解析，从右向左取首个不可信地址。未信任对端的代理头被忽略；不解析X-Real-IP，亦不用于决定HTTPS。
- log_level：info（默认）、trace、debug、warn、error、off
- development_web_dir：仅unbound开发构建可用的显式Web目录

对应环境是 XSCS_BIND、XSCS_DATA_DIR、XSCS_DEVELOPMENT、XSCS_LOG_LEVEL、XSCS_TRUSTED_PROXIES（JSON数组）、XSCS_DEV_WEB_DIR。全局 --bind、--data-dir、--development true/false、--log-level 可覆盖。密钥与数据库路径不是配置入口。

## 检查、日志与维护

config validate 只验证配置，初始化前可运行。check 只读检查已有数据库、密钥、资源和目录。status 查询实际live readiness，不把进程存在当成功。run 仅运行已初始化实例，不隐式创建账号或密钥。

前台和systemd使用同一UTC结构日志，保留UI日志语义。服务写 data_dir/logs 下滚动日志，默认每文件8MiB、4个归档；log_level统一控制，不读取RUST_LOG。systemd提供120秒停止窗口，维护前等待systemctl stop完成。

重置管理员密码使用 `xscs admin reset-password --username admin`，密码从stdin读取；执行前停服，成功后重启并重新登录。普通改名改密仍可用Web账户设置。
