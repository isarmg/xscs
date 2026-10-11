# xscs

xscs 是自托管的 Sunshine 主机管理服务，与每台主机上的 xscc 代理配合，提供统一的 Web 管理台。

## 项目功能

- 管理多台 Sunshine 主机的配置、应用和 Moonlight PIN
- 查询日志、诊断与维护状态，下发任务并查看结果和审计记录
- 通过客户端控制受支持平台的 Sunshine 服务；媒体流仍由 Sunshine 与 Moonlight 直接传输

## 适用平台

服务端仅支持 Linux x86_64 / AMD64、glibc，生产部署使用 systemd。Web 页面通过浏览器访问，已内嵌到服务端程序中。

## 快速部署

以下用于全新主机。先从 [Release](https://github.com/isarmg/xscs/releases) 下载同版 Linux 归档及 `.sha256`，在下载目录执行；遇到已有目录、账户或配置时停止，不覆盖现有安装。

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

就绪响应应为 `{"ready":true}`。通过 HTTPS 入口转发到配置的后端地址，使用配置的管理员账号登录；跨服务器部署见详细文档中的部署手册。初始化成功后从环境文件移除 `XSCS_BOOTSTRAP_ADMIN_PASSWORD`。客户端须另行安装并配对。

## 编译部署

在 Linux AMD64 GNU 主机准备 Git、Rust `1.99.0`、Node.js `26.7.0`、npm、Python `3.11+` 和 C 编译工具。从干净源码、与版本号一致且精确指向 HEAD 的 annotated tag 构建发行包；输出目录必须已存在、位于仓库外且不含同名制品：

以下命令复现已发布的 v1.0.0，不包含 main 的后续修复。当前源码构建见详细文档中的开发指南；正式打包需要对应版本标签。

```sh
git clone https://github.com/isarmg/xscs.git
cd xscs
git checkout v1.0.0
mkdir -p "$HOME/xscs-output"
python3 scripts/package-release.py "$HOME/xscs-output"
```

脚本构建 Web 和 Rust、生成校验信息并验证发行包。将输出的归档和 `.sha256` 复制到目标主机，按“快速部署”安装。

[详细文档](docs/README.md)
