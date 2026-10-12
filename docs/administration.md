# 管理服务

使用 `systemctl status xscs` 查看服务状态，`journalctl -u xscs` 查看启动前诊断。运行日志以UTC结构JSONL写入data_dir/logs，亦可从管理UI查看。修改JSON配置后先执行 `xscs config validate`，再 `systemctl restart xscs` 和 `xscs status --json`。

维护前 `systemctl stop xscs` 并等待退出。`xscs check` 只读验证完整状态，不创建数据库或密钥。备份整个data_dir，包括secrets.json；不要只替换数据库或重新生成密钥。

```bash
sudo systemctl stop xscs
read -rs -p '新密码: ' password; printf '\n'
printf '%s\n' "$password" | sudo -u isarmg-xscs /opt/isarmg/xscs/bin/xscs admin reset-password --username admin
unset password
sudo systemctl start xscs
```

正常改名改密使用Web账号设置。详细安装和路径见[运维](operations.md)。
