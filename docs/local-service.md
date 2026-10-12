# 前台开发

构建内嵌Web和Server后，以当前用户准备私有JSON配置，指定绝对data_dir、回环bind和development=true。JSON文件0600、直接父目录0700。

```bash
web/node_modules/.bin/xcss-build-server --config xcss-web-build.json --mode development --no-install
binary="$PWD/target/x86_64-unknown-linux-gnu/debug/xscs"
config="$HOME/.config/xscs/config.json"
"$binary" --config "$config" config validate
read -rs -p '管理员密码: ' password; printf '\n'
printf '%s\n' "$password" | "$binary" --config "$config" init --username admin
unset password
"$binary" --config "$config" check
"$binary" --config "$config" run
```

init只运行一次，密钥自动写入data_dir/secrets.json。run保持前台，Ctrl+C优雅停止；需要后台运行使用标准systemd，不提供自实现PID/nohup启动器。

开发前端可运行 `npm run dev --prefix web`；显式development_web_dir只允许未绑定源码的开发构建。默认使用内嵌Web资源。
