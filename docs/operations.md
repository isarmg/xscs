# xscs 1.0.2 发行包部署手册

## 安装与初始化


在一台尚未安装本服务的 Linux x86_64 / glibc 主机上，以可使用 sudo 的管理员执行。需要 systemd、GNU tar、sha256sum、curl、OpenSSL 以及账户管理工具。

先从 [Release](https://github.com/isarmg/xscs/releases) 下载同版 Linux 归档及 `.sha256`，在下载目录执行；以下检查用于确认安装位置为空；已有服务的管理见本文后面的运行检查。

```sh
set -eu
sha256sum --check --strict xscs-1.0.2-x86_64-unknown-linux-gnu.tar.gz.sha256
sudo test ! -e /opt/isarmg/xscs
sudo test ! -e /etc/isarmg/xscs.env
sudo test ! -e /etc/systemd/system/xscs.service
sudo test ! -e /var/lib/isarmg/xscs
sudo install -d -m 0755 -o root -g root /opt/isarmg /opt/isarmg/xscs /opt/isarmg/xscs/releases
sudo tar -xzf xscs-1.0.2-x86_64-unknown-linux-gnu.tar.gz \
  -C /opt/isarmg/xscs/releases --same-permissions --delay-directory-restore
sudo chown -R root:root /opt/isarmg/xscs/releases/1.0.2
sudo /opt/isarmg/xscs/releases/1.0.2/bin/xscs \
  verify-release --root /opt/isarmg/xscs/releases/1.0.2
sudo groupadd --system xscs
sudo useradd --system --gid xscs --home-dir /var/lib/isarmg/xscs \
  --no-create-home --shell /usr/sbin/nologin xscs
sudo install -d -m 0700 -o xscs -g xscs /var/lib/isarmg/xscs/db
sudo install -d -m 0755 -o root -g root /etc/isarmg
sudo sh -c 'umask 077; set -C; : > /etc/isarmg/xscs.env'
openssl rand -base64 32
sudoedit /etc/isarmg/xscs.env
```

填写下列配置：将密码替换为至少 12 字节的独立强密码，密钥替换为上一步生成的 Base64 值；密钥需长期保存。

```dotenv
XSCS_DATABASE_URL=sqlite:///var/lib/isarmg/xscs/db/xscs.sqlite3
XSCS_BIND=127.0.0.1:18104
XSCS_PRODUCTION=true
XSCS_BOOTSTRAP_ADMIN_USERNAME=admin
XSCS_BOOTSTRAP_ADMIN_PASSWORD=REPLACE_WITH_A_UNIQUE_LONG_PASSWORD
XSCS_CREDENTIAL_KEY=REPLACE_WITH_BASE64_ENCODED_32_RANDOM_BYTES
XSCS_CREDENTIAL_KEY_ID=primary
```

先初始化，再启动服务：

```sh
sudo ln -sT /opt/isarmg/xscs/releases/1.0.2 /opt/isarmg/xscs/current
sudo systemd-run --wait --collect -p User=xscs -p Group=xscs \
  -p EnvironmentFile=/etc/isarmg/xscs.env \
  /opt/isarmg/xscs/releases/1.0.2/bin/xscs init
sudo install -m 0644 -o root -g root \
  /opt/isarmg/xscs/current/systemd/xscs.service /etc/systemd/system/xscs.service
sudo systemctl daemon-reload
sudo systemctl enable --now xscs.service
curl --fail http://127.0.0.1:18104/readyz
```

就绪响应应为 `{"ready":true}`。通过 HTTPS 入口转发到配置的后端地址，使用配置的管理员账号登录；初始化成功后从环境文件移除 `XSCS_BOOTSTRAP_ADMIN_PASSWORD`。客户端须另行安装并配对。


## HTTPS 入口

每个 xscs 项目实例在一台服务器上运行，程序、SQLite 数据库与状态文件留在该机。其他服务端项目可以部署在其他服务器，由统一入口按各自独立三级域名转发；不拆分 xscs 组件，也不为同一实例配置多机副本。

HTTPS/WSS 和系统信任的证书由路由或反向代理入口保证。xscs 提供普通 HTTP 后端，不识别代理品牌，也不要求 `X-Forwarded-Proto`。同机入口可使用默认的 `127.0.0.1:18104`；入口在另一台服务器时，将 `XSCS_BIND` 改为 xscs 主机的内网地址，例如 `10.20.0.12:18104`，保留 `XSCS_PRODUCTION=true`，再按配置章节重启服务。后端端口只向受控入口网络开放；跨越不可信网络的回源链路需由部署环境保护。

将一个域名的所有路径原样转发到该后端，保留原始 Host、Origin、Sec-Fetch-Site、Authorization 与响应头、JSON 错误及状态码，支持 `/xscc/v1/` 的 WebSocket Upgrade。不要添加或剥离 URL 路径前缀。浏览器和 Client 均使用这个公网 HTTPS 域名。

例如在统一入口上使用 Caddy，将域名和内网地址替换为实际值：

```caddyfile
sunshine.example.com {
    reverse_proxy 10.20.0.12:18104
}
```

其他项目各用自己的域名块和后端地址。Caddy 的普通 HTTP 回源保留 Host，并自动处理 WebSocket Upgrade，见[官方 reverse_proxy 文档](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy)。完整示例位于 [deploy/Caddyfile](https://github.com/isarmg/xscs/blob/main/deploy/Caddyfile)；其他入口遵守上述 HTTP 转发约定即可，[Nginx 设备通道示例](https://github.com/isarmg/xscs/blob/main/deploy/client-ingress.nginx.conf)也支持配置内网后端。Windows Client 的系统信任链来自 LocalSystem 的计算机证书存储。

## 运行检查

```sh
sudo systemctl status xscs.service --no-pager
sudo journalctl -u xscs.service -n 100 --no-pager
curl --fail http://127.0.0.1:18104/readyz
```

以上 curl 使用默认回环地址；如果修改了 `XSCS_BIND`，请改为配置的内网地址，并从入口主机确认也能访问。预期服务为 active，`/readyz` 返回 `{"ready":true}`；`/healthz` 的正常状态为 HTTP 204。然后在浏览器打开 HTTPS 地址并登录，创建一个实例，按 [xscc 安装说明](https://github.com/isarmg/xscc/blob/main/docs/platform-setup.md)完成配对。管理台应显示设备在线；读取一次 Sunshine 配置并确认任务返回结果。

修改环境文件后执行 `sudo systemctl restart xscs.service`，再检查就绪状态。`XSCS_CREDENTIAL_KEY` 与数据库内密文配套，日常修改密码或实例授权码时保持它不变；替换该密钥会使已有密文不可读。

## 安装布局

| 路径 | 用途 |
|---|---|
| `/opt/isarmg/xscs/releases/1.0.2/` | root 所有的只读发行目录 |
| `/opt/isarmg/xscs/current` | 指向该版本的绝对链接 |
| `/etc/isarmg/xscs.env` | root 0600 配置，由 systemd 加载 |
| `/var/lib/isarmg/xscs/db/xscs.sqlite3` | 服务账号所有的当前数据库 |
| `/var/lib/isarmg/xscs/db/logs/` | 私有轮转运行日志 |

归档的顶层是 `1.0.2/`，包含 `bin/xscs`、`systemd/xscs.service`、`web-assets.json`、`README.md` 和 `RELEASE-MANIFEST.json`。管理页面已内嵌到二进制。修改运行参数使用环境文件；发行目录保留下载时的内容与权限，供启动校验使用。

`identity` 输出产品、版本、源码提交和结构身份；`verify-release` 核对完整文件树。SHA-256 用于核对下载完整性，仍需从可信发布页取得归档及校验文件。

| 身份字段 | 当前值 |
|---|---|
| `application` | `xscs` |
| `version` | `1.0.2` |
| `schema_revision` | `1` |
| `target` | `x86_64-unknown-linux-gnu` |

## 遇到问题

- 校验失败：重新核对下载来源、文件名及完整性；保留原始错误。
- `init` 失败：检查环境文件中的密码、Base64 密钥、数据目录所有者与 0700 权限。
- 服务无法启动：先读 journal，确认已初始化，发行树校验通过且配置和数据库配套。
- 本机 ready 正常但无法登录：检查 HTTPS 代理、浏览器地址、Host/Origin 和主机时间。
- 客户端离线：检查客户端日志、HTTPS 信任和配对；再确认 WSS Upgrade 与本机 Sunshine 访问。

[配置参考](https://github.com/isarmg/xscs/blob/main/docs/configuration.md) · [日常运维](https://github.com/isarmg/xscs/blob/main/docs/administration.md) · [故障排查](https://github.com/isarmg/xscs/blob/main/docs/troubleshooting.md)
