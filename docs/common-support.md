# xscs 使用的公共库

xscs 在构建时依赖 Rust 包 xcss 和管理 Web 包 @xcss/web。它们提供通用机制，由产品组合自己的协议、页面、数据与资源限制。

## 职责与来源

| 输入 | 提供的能力 | 固定位置 |
|---|---|---|
| `xcss` | 配置、CLI、日志、认证、SQLite 和服务生命周期 | Cargo.toml 的完整 Git rev 与 Cargo.lock |
| `@xcss/web` | 管理员会话、共享界面、字体和 Web 构建工具 | web/package.json 与 package-lock.json 的发行 URL / integrity |
| `xscs-protocol` | 产品业务类型与校验 | crates/protocol，由 Client 锁定源码消费 |

Server 和 xcss 的运行目标为 Linux AMD64 GNU。独立 Client 使用 xcsc；共享业务协议保持数据类型层的依赖。

## 检查构建输入

从仓库根目录执行：

```sh
cargo metadata --locked --no-deps --format-version 1
cargo tree --locked -e normal
npm --prefix web ci
npm --prefix web run check:xcss
```

metadata 查看包、目标和直接依赖，tree 查看完整依赖图，npm ci 按锁文件安装并核验摘要。模块或完整性检查失败时，核对清单、锁文件和当前协议源码是否匹配。

更新公共库时一起更新调用代码、完整 revision、锁文件及对应测试。完整检查见[开发指南](development.md)；公共库作为构建依赖随产品交付，部署时使用产品自己的服务、配置和数据目录。

Rust 输入固定为 xcss 1.0.0 / `b34b3861e15edd2b1348e7d03b9c9a93e97d6521`；Web 输入为 `https://github.com/isarmg/xcss/releases/download/v1.0.0/xcss-web-1.0.0.tgz`，锁文件 SHA512 integrity 固定为 `sha512-QR/zmdzyQkLnFzTTp+EHYnOL9XEe5N95XVUasnDlKqdNWw/nVoY4evq083QTxMED/ElRRYeegEItnTxCUcb7TQ==`。正式发行不使用本地路径补丁。
