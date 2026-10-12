# xscs

Sunshine 管理服务：管理员认证、设备配对、实例配置、任务与日志，内嵌 Web 管理界面。服务端支持 Linux x86_64 GNU；需要 Rust 1.99.0、Node.js 26.7.0 编译。

## 快速部署

解压发行包后执行以下首次安装步骤，HTTPS 由部署入口提供：

```bash
cp config/config.json.example ./my-config.json
${EDITOR:-vi} ./my-config.json
read -rs -p '管理员密码: ' password; printf '\n'
printf '%s\n' "$password" | sudo ./install.sh setup ./my-config.json admin
unset password
sudo systemctl enable --now xscs
```

## 源码构建

```sh
npm ci --prefix web
web/node_modules/.bin/xcss-build-server --config xcss-web-build.json --mode release --no-install --source-revision "$(git rev-parse HEAD)"
```

构建输出位于 `target/x86_64-unknown-linux-gnu/release/xscs`。完整配置、初始化、编译与运维见[文档](docs/README.md)。
