//! Build-bound Web resources and shared HTTP semantics, independent of the server framework.
//!
//! Use [`build::generate`] in `build.rs` and include its generated source at runtime.
//! [`DirectoryAssets`] is an explicitly selected development provider. It never replaces
//! embedded resources automatically, and rereads each requested file for hot reload.

use anyhow::{Context, Result, bail, ensure};
use bytes::Bytes;
use http::{HeaderMap, Method, Response, StatusCode, header};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(not(unix))]
use std::fs;
use std::{
    fs::File,
    io::Read,
    path::{Component, Path, PathBuf},
    sync::Arc,
};

pub mod build;
pub mod wasm;

/// Inventory format identity. Its canonical compact JSON bytes are hashed verbatim.
pub const MANIFEST_FORMAT: &str = "web-assets-v1";
const MAX_ASSET_BYTES: u64 = 64 * 1024 * 1024;
const MAX_TOTAL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_FILES: usize = 16_384;

/// A generated resource; `sha256` binds the exact bytes, not its filename.
#[derive(Clone, Copy, Debug)]
pub struct EmbeddedAsset {
    pub path: &'static str,
    pub content_type: &'static str,
    pub bytes: &'static [u8],
    pub sha256: &'static str,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssetManifest {
    pub format: String,
    pub files: Vec<AssetRecord>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AssetRecord {
    pub path: String,
    pub content_type: String,
    pub size: u64,
    pub sha256: String,
}

/// Verify inventory format, canonical ordering/serialization and its bound digest.
pub fn verify_manifest(manifest: &str, digest: &str) -> Result<()> {
    ensure!(
        manifest.len() <= 8 * 1024 * 1024,
        "web manifest exceeds budget"
    );
    let parsed: AssetManifest = serde_json::from_str(manifest).context("invalid web manifest")?;
    ensure!(
        parsed.format == MANIFEST_FORMAT,
        "unsupported web manifest format"
    );
    ensure!(
        !parsed.files.is_empty() && parsed.files.len() <= MAX_FILES,
        "invalid web inventory size"
    );
    let mut previous: Option<&str> = None;
    let mut total = 0_u64;
    for file in &parsed.files {
        ensure!(safe_relative(&file.path), "unsafe web asset path");
        ensure!(
            previous.is_none_or(|path| path < file.path.as_str()),
            "web inventory is not strictly sorted"
        );
        previous = Some(&file.path);
        ensure!(
            file.content_type == content_type(&file.path),
            "web asset content type differs"
        );
        ensure!(valid_digest(&file.sha256), "invalid web asset digest");
        ensure!(file.size <= MAX_ASSET_BYTES, "web asset exceeds budget");
        total = total
            .checked_add(file.size)
            .context("web inventory byte overflow")?;
        ensure!(total <= MAX_TOTAL_BYTES, "web inventory exceeds budget");
    }
    ensure!(
        serde_json::to_string(&parsed)? == manifest,
        "web manifest is not canonical JSON"
    );
    ensure!(
        valid_digest(digest) && sha256(manifest.as_bytes()) == digest,
        "web manifest digest differs"
    );
    Ok(())
}

/// Verify a generated registry against the manifest and every embedded byte.
/// Intended for build acceptance and release validation, not every HTTP request.
pub fn verify_embedded(assets: &[EmbeddedAsset], manifest: &str, digest: &str) -> Result<()> {
    verify_manifest(manifest, digest)?;
    let parsed: AssetManifest = serde_json::from_str(manifest)?;
    ensure!(
        parsed.files.len() == assets.len(),
        "embedded web inventory length differs"
    );
    for (record, asset) in parsed.files.iter().zip(assets) {
        ensure!(
            record.path == asset.path && record.content_type == asset.content_type,
            "embedded web metadata differs"
        );
        ensure!(
            record.size == asset.bytes.len() as u64
                && record.sha256 == asset.sha256
                && record.sha256 == sha256(asset.bytes),
            "embedded web bytes differ"
        );
    }
    Ok(())
}

/// Serve exact embedded resources. Empty path and `/` select `index.html`.
/// Unknown paths are 404, unsupported methods are 405; no SPA fallback is implicit.
/// HTML is never stored. Other resources revalidate through a strong SHA-256 ETag.
pub fn response(
    assets: &'static [EmbeddedAsset],
    path: &str,
    method: &Method,
    headers: &HeaderMap,
) -> Response<Bytes> {
    if method != Method::GET && method != Method::HEAD {
        return error_response(StatusCode::METHOD_NOT_ALLOWED, method);
    }
    let Some(path) = request_path(path) else {
        return error_response(StatusCode::NOT_FOUND, method);
    };
    let Some(asset) = assets.iter().find(|asset| asset.path == path) else {
        return error_response(StatusCode::NOT_FOUND, method);
    };
    asset_response(
        Bytes::from_static(asset.bytes),
        asset.content_type,
        asset.sha256,
        method,
        headers,
    )
}

/// Descriptor-bound development root, with no-follow reads on Unix.
/// File changes are visible on the next request. Symlinks, hardlinks, special files,
/// unsafe paths, and oversized reads are rejected instead of being served.
#[derive(Clone, Debug)]
pub struct DirectoryAssets {
    root: PathBuf,
    #[cfg(unix)]
    directory: Arc<File>,
    #[cfg(not(unix))]
    directory: Arc<()>,
}

impl DirectoryAssets {
    /// Open an absolute development directory. No production mode is selected here.
    pub fn new(root: impl AsRef<Path>) -> Result<Self> {
        let root = root.as_ref();
        ensure!(root.is_absolute(), "web directory must be absolute");
        ensure!(
            root.components().all(|component| matches!(
                component,
                Component::RootDir | Component::Prefix(_) | Component::Normal(_)
            )),
            "unsafe web directory"
        );
        #[cfg(unix)]
        let directory = Arc::new(open_directory(root)?);
        #[cfg(not(unix))]
        let directory = {
            let mut current = PathBuf::new();
            for component in root.components() {
                current.push(component);
                let metadata = fs::symlink_metadata(&current)?;
                ensure!(
                    metadata.is_dir() && !metadata.file_type().is_symlink(),
                    "unsafe web directory"
                );
            }
            Arc::new(())
        };
        // Fail configuration early for unsafe initial trees; later reads still
        // validate descriptors because the development tree can change.
        build::collect_paths(root, false)?;
        Ok(Self {
            root: root.to_path_buf(),
            directory,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn response(&self, path: &str, method: &Method, headers: &HeaderMap) -> Response<Bytes> {
        if method != Method::GET && method != Method::HEAD {
            return error_response(StatusCode::METHOD_NOT_ALLOWED, method);
        }
        let Some(path) = request_path(path) else {
            return error_response(StatusCode::NOT_FOUND, method);
        };
        match self.read(path) {
            Ok(bytes) => {
                let digest = sha256(&bytes);
                asset_response(
                    Bytes::from(bytes),
                    content_type(path),
                    &digest,
                    method,
                    headers,
                )
            }
            Err(_) => error_response(StatusCode::NOT_FOUND, method),
        }
    }

    pub(crate) fn read(&self, path: &str) -> Result<Vec<u8>> {
        ensure!(safe_relative(path), "unsafe web asset path");
        #[cfg(unix)]
        {
            use rustix::fs::{FileType, Mode, OFlags, fstat, openat};
            let mut parent = self.directory.try_clone()?;
            let mut components = Path::new(path).components().peekable();
            while let Some(Component::Normal(name)) = components.next() {
                if components.peek().is_some() {
                    parent = File::from(openat(&parent, name, directory_flags(), Mode::empty())?);
                } else {
                    let file = File::from(openat(
                        &parent,
                        name,
                        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC,
                        Mode::empty(),
                    )?);
                    let metadata = fstat(&file)?;
                    ensure!(
                        FileType::from_raw_mode(metadata.st_mode) == FileType::RegularFile
                            && metadata.st_nlink == 1,
                        "unsafe web asset file"
                    );
                    return read_bounded(file);
                }
            }
            bail!("empty web asset path");
        }
        #[cfg(not(unix))]
        {
            let _ = &self.directory;
            let mut current = self.root.clone();
            for component in Path::new(path).components() {
                current.push(component);
                let metadata = fs::symlink_metadata(&current)?;
                ensure!(!metadata.file_type().is_symlink(), "unsafe web asset link");
            }
            let metadata = fs::symlink_metadata(&current)?;
            ensure!(metadata.is_file(), "unsafe web asset file");
            read_bounded(File::open(current)?)
        }
    }
}

fn read_bounded(file: File) -> Result<Vec<u8>> {
    ensure!(
        file.metadata()?.len() <= MAX_ASSET_BYTES,
        "web asset exceeds budget"
    );
    let mut bytes = Vec::new();
    file.take(MAX_ASSET_BYTES + 1).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() as u64 <= MAX_ASSET_BYTES,
        "web asset exceeds budget"
    );
    Ok(bytes)
}

#[cfg(unix)]
fn directory_flags() -> rustix::fs::OFlags {
    use rustix::fs::OFlags;
    OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC
}

#[cfg(unix)]
fn open_directory(path: &Path) -> Result<File> {
    use rustix::fs::{Mode, open, openat};
    let mut fd = open("/", directory_flags(), Mode::empty())?;
    for component in path.components() {
        match component {
            Component::RootDir => (),
            Component::Normal(name) => fd = openat(&fd, name, directory_flags(), Mode::empty())?,
            _ => bail!("unsafe web directory"),
        }
    }
    Ok(File::from(fd))
}

fn request_path(path: &str) -> Option<&str> {
    if path.is_empty() || path == "/" {
        Some("index.html")
    } else {
        let path = path.strip_prefix('/').unwrap_or(path);
        safe_relative(path).then_some(path)
    }
}

pub(crate) fn safe_relative(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 4096
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part.len() <= 255
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
}

fn valid_digest(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Explicit MIME mappings; unknown outputs remain binary and cannot be sniffed.
pub fn content_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "json" | "map" => "application/json",
        "txt" | "md" => "text/plain; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "mp4" => "video/mp4",
        "ico" => "image/x-icon",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        "ttf" => "font/ttf",
        "otf" => "font/otf",
        "wasm" => "application/wasm",
        "webmanifest" => "application/manifest+json",
        "xml" => "application/xml",
        _ => "application/octet-stream",
    }
}

fn base_response(
    status: StatusCode,
    content_type: &str,
    size: usize,
    method: &Method,
) -> Response<Bytes> {
    let mut builder = Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::REFERRER_POLICY, "same-origin");
    if status != StatusCode::NOT_MODIFIED {
        builder = builder.header(header::CONTENT_LENGTH, size.to_string());
    }
    let body = if method == Method::HEAD {
        Bytes::new()
    } else {
        Bytes::from_static(b"")
    };
    builder.body(body).expect("constant HTTP headers are valid")
}

fn asset_response(
    bytes: Bytes,
    content_type: &str,
    digest: &str,
    method: &Method,
    headers: &HeaderMap,
) -> Response<Bytes> {
    let html = content_type.starts_with("text/html");
    let etag = format!("\"{digest}\"");
    // If-None-Match uses weak comparison for GET/HEAD, including lists and wildcard.
    let unchanged = !html
        && headers.get_all(header::IF_NONE_MATCH).iter().any(|value| {
            value.to_str().is_ok_and(|value| {
                value.split(',').any(|candidate| {
                    let candidate = candidate.trim();
                    candidate == "*" || candidate.strip_prefix("W/").unwrap_or(candidate) == etag
                })
            })
        });
    let mut result = base_response(
        if unchanged {
            StatusCode::NOT_MODIFIED
        } else {
            StatusCode::OK
        },
        content_type,
        bytes.len(),
        method,
    );
    result.headers_mut().insert(
        header::CACHE_CONTROL,
        if html {
            "no-store, max-age=0"
        } else {
            "public, no-cache"
        }
        .parse()
        .expect("constant cache header"),
    );
    if !html {
        result
            .headers_mut()
            .insert(header::ETAG, etag.parse().expect("hexadecimal ETag"));
    }
    if method != Method::HEAD && !unchanged {
        *result.body_mut() = bytes;
    }
    result
}

fn error_response(status: StatusCode, method: &Method) -> Response<Bytes> {
    let message: &'static [u8] = if status == StatusCode::METHOD_NOT_ALLOWED {
        b"Method not allowed\n"
    } else {
        b"Not found\n"
    };
    let mut result = base_response(status, "text/plain; charset=utf-8", message.len(), method);
    result.headers_mut().insert(
        header::CACHE_CONTROL,
        "no-store".parse().expect("constant cache header"),
    );
    if status == StatusCode::METHOD_NOT_ALLOWED {
        result.headers_mut().insert(
            header::ALLOW,
            "GET, HEAD".parse().expect("constant allow header"),
        );
    }
    if method != Method::HEAD {
        *result.body_mut() = Bytes::from_static(message);
    }
    result
}

#[cfg(test)]
mod tests;
