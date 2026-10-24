# xcss::web_assets

生产资源编译进服务端可执行文件。资源清单及其 SHA-256 摘要绑定文件名、MIME 类型、大小和每一个内嵌字节。生产模式不会隐式选择磁盘资源目录。

在 `[dependencies]` 和 `[build-dependencies]` 中都声明唯一的 `xcss` crate。整个 crate 要求 Linux x86_64 GNU。前端构建完成后，在 `build.rs` 中调用：

```rust,ignore
fn main() -> anyhow::Result<()> {
    xcss::web_assets::build::generate("web/dist")
}
```

运行时使用：

```rust,ignore
mod web {
    include!(concat!(env!("OUT_DIR"), "/xcss-web-assets.rs"));
}
let response = xcss::web_assets::response(web::ASSETS, uri.path(), &method, &headers);
// Axum: response.map(axum::body::Body::from)
// Hyper: response.map(http_body_util::Full::new)
```

生成的模块导出 `ASSETS`、`MANIFEST` 和 `DIGEST`。`MANIFEST` 是紧凑 JSON，结构严格为 `{ "format": "web-assets-v1", "files": [{ "path": "index.html", "content_type": "text/html; charset=utf-8", "size": 123, "sha256": "..." }] }`，没有外围空白或末尾换行。文件按路径排序。`DIGEST` 是清单原始字节的 SHA-256。`verify_manifest` 验证规范格式和摘要；`verify_embedded` 还核对每一个内嵌字节。发行工具可以读取可执行文件输出的清单，核验实际提供的资源，无需另行发行一份重复的 Web 目录。

`build::generate_to(root, filename)` 支持生成多个独立模块。构建工具可用 `build::generate_at(root, output, filename)` 指定输出目录，无需修改 `OUT_DIR`。生成器先把输入字节快照写入输出目录，再使用 `include_bytes!`，因此编译期间修改输入不会悄然使编译清单失效。所有普通文件都会被纳入，包括许可证。当资源包含有非运行时文件时，应在构建暂存目录中选择实际需要的输入。

开发时显式选择 `DirectoryAssets::new(absolute_root)`，并调用其 `response(path, method, headers)` 方法。该提供器在每次请求时重新读取资源。在 Unix 上，已持有的目录描述符及带 `NOFOLLOW` 的 `openat` 防止通过被替换的目录或链接遍历。可移植提供器逐一检查路径分量中的符号链接；正式支持目标仍为 Linux AMD64 GNU。生成清单时会拒绝符号链接、Unix 上的硬链接、特殊文件、不安全名称以及超出预算的清单或读取。

两种提供器共享以下 HTTP 行为：

- 空路径和 `/` 选择 `index.html`；其他路径必须精确匹配资源。产品在调用处理器前移除挂载前缀。
- 支持 GET 和 HEAD，返回精确的 MIME 类型、`nosniff` 和 `Referrer-Policy: same-origin`；其他方法返回 405。
- HTML 使用 `Cache-Control: no-store, max-age=0`。其他资源使用 `public, no-cache` 和基于 SHA-256 的强 ETag；条件 GET/HEAD 支持弱验证器、验证器列表及 `*`。
- 未知或不安全路径返回 404，不自动回退到单页应用入口或磁盘资源。
- 产品继续控制自身的 CSP 和其他应用响应头。仅凭文件名不能证明内容身份，因此本库不会根据猜测的文件命名规则启用不可变缓存。
