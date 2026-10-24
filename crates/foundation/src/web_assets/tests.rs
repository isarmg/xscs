use super::*;
use http::HeaderValue;
use std::fs;
use tempfile::TempDir;

fn fixture() -> (&'static [EmbeddedAsset], String, String) {
    let mut assets = Vec::new();
    let mut records = Vec::new();
    for (path, bytes) in [
        ("assets/app.js", b"console.log('hello');".as_slice()),
        ("index.html", b"<h1>Hello</h1>".as_slice()),
    ] {
        let digest: &'static str = Box::leak(sha256(bytes).into_boxed_str());
        assets.push(EmbeddedAsset {
            path,
            bytes,
            content_type: content_type(path),
            sha256: digest,
        });
        records.push(AssetRecord {
            path: path.to_owned(),
            content_type: content_type(path).to_owned(),
            size: bytes.len() as u64,
            sha256: digest.to_owned(),
        });
    }
    let manifest = serde_json::to_string(&AssetManifest {
        format: MANIFEST_FORMAT.to_owned(),
        files: records,
    })
    .unwrap();
    let digest = sha256(manifest.as_bytes());
    (Box::leak(assets.into_boxed_slice()), manifest, digest)
}

fn directory() -> TempDir {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("assets")).unwrap();
    fs::write(temp.path().join("index.html"), b"<h1>Hello</h1>").unwrap();
    fs::write(temp.path().join("assets/app.js"), b"console.log('hello');").unwrap();
    temp
}

#[test]
fn embedded_get_head_and_exact_types() {
    let (assets, _, _) = fixture();
    let response = super::response(assets, "/assets/app.js", &Method::GET, &HeaderMap::new());
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.body().as_ref(), b"console.log('hello');");
    assert_eq!(
        response.headers()[header::CONTENT_TYPE],
        "text/javascript; charset=utf-8"
    );
    assert_eq!(
        response.headers()[header::X_CONTENT_TYPE_OPTIONS],
        "nosniff"
    );
    let head = super::response(assets, "/assets/app.js", &Method::HEAD, &HeaderMap::new());
    assert!(head.body().is_empty());
    assert_eq!(head.headers(), response.headers());
}

#[test]
fn embedded_public_video_retains_media_type_and_head_contract() {
    let bytes = b"public MP4 payload fixture";
    let asset = EmbeddedAsset {
        path: "legacy/backgroundVideo.mp4",
        bytes,
        content_type: content_type("legacy/backgroundVideo.mp4"),
        sha256: Box::leak(sha256(bytes).into_boxed_str()),
    };
    let assets = Box::leak(vec![asset].into_boxed_slice());
    let get = response(
        assets,
        "/legacy/backgroundVideo.mp4",
        &Method::GET,
        &HeaderMap::new(),
    );
    assert_eq!(get.status(), StatusCode::OK);
    assert_eq!(get.headers()[header::CONTENT_TYPE], "video/mp4");
    assert_eq!(get.headers()[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
    assert_eq!(get.body().as_ref(), bytes);
    let head = response(
        assets,
        "/legacy/backgroundVideo.mp4",
        &Method::HEAD,
        &HeaderMap::new(),
    );
    assert!(head.body().is_empty());
    assert_eq!(head.headers(), get.headers());
    assert_eq!(
        content_type("unrecognized.payload"),
        "application/octet-stream"
    );
}

#[test]
fn conditional_requests_accept_weak_lists_and_wildcard() {
    let (assets, _, _) = fixture();
    let get = response(assets, "/assets/app.js", &Method::GET, &HeaderMap::new());
    let etag = get.headers()[header::ETAG].to_str().unwrap();
    for validator in [
        etag.to_owned(),
        format!("W/{etag}"),
        format!("\"different\", W/{etag}"),
        "*".to_owned(),
    ] {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::IF_NONE_MATCH,
            HeaderValue::from_str(&validator).unwrap(),
        );
        for method in [&Method::GET, &Method::HEAD] {
            let not_modified = response(assets, "/assets/app.js", method, &headers);
            assert_eq!(not_modified.status(), StatusCode::NOT_MODIFIED);
            assert!(not_modified.body().is_empty());
            assert!(!not_modified.headers().contains_key(header::CONTENT_LENGTH));
            assert_eq!(not_modified.headers()[header::ETAG], etag);
        }
    }
}

#[test]
fn html_always_fresh_and_never_stored() {
    let (assets, _, _) = fixture();
    let mut headers = HeaderMap::new();
    headers.insert(header::IF_NONE_MATCH, HeaderValue::from_static("*"));
    for path in ["", "/", "/index.html"] {
        let html = response(assets, path, &Method::GET, &headers);
        assert_eq!(html.status(), StatusCode::OK);
        assert_eq!(html.headers()[header::CACHE_CONTROL], "no-store, max-age=0");
        assert!(!html.headers().contains_key(header::ETAG));
    }
}

#[test]
fn unsafe_unknown_and_non_get_requests_are_rejected() {
    let (assets, _, _) = fixture();
    for path in [
        "/missing",
        "//index.html",
        "/../index.html",
        "/assets/../index.html",
        "/%2e%2e/index.html",
        "/assets\\app.js",
        "/index.html?query",
        "/assets/",
    ] {
        let response = response(assets, path, &Method::GET, &HeaderMap::new());
        assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
    }
    let response = response(assets, "/", &Method::POST, &HeaderMap::new());
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(response.headers()[header::ALLOW], "GET, HEAD");
}

#[test]
fn manifests_validate_bytes_order_and_canonical_json() {
    let (assets, manifest, digest) = fixture();
    verify_embedded(assets, &manifest, &digest).unwrap();
    assert!(verify_manifest(&format!("{manifest}\n"), &digest).is_err());
    assert!(verify_manifest(&manifest, &"0".repeat(64)).is_err());
    let mut parsed: AssetManifest = serde_json::from_str(&manifest).unwrap();
    parsed.files.reverse();
    let unsorted = serde_json::to_string(&parsed).unwrap();
    assert!(verify_manifest(&unsorted, &sha256(unsorted.as_bytes())).is_err());
    parsed.files.reverse();
    parsed.files[0].sha256 = "0".repeat(64);
    let tampered = serde_json::to_string(&parsed).unwrap();
    assert!(verify_embedded(assets, &tampered, &sha256(tampered.as_bytes())).is_err());
}

#[test]
fn generated_inventory_is_deterministic_and_snapshots_exact_bytes() {
    let input = directory();
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    build::generate_at(input.path(), first.path(), "web.rs").unwrap();
    build::generate_at(input.path(), second.path(), "web.rs").unwrap();
    let manifest = fs::read_to_string(first.path().join("web.rs.manifest.json")).unwrap();
    assert_eq!(
        manifest,
        fs::read_to_string(second.path().join("web.rs.manifest.json")).unwrap()
    );
    verify_manifest(&manifest, &sha256(manifest.as_bytes())).unwrap();
    let parsed: AssetManifest = serde_json::from_str(&manifest).unwrap();
    assert_eq!(
        parsed
            .files
            .iter()
            .map(|record| record.path.as_str())
            .collect::<Vec<_>>(),
        ["assets/app.js", "index.html"]
    );
    fs::write(
        input.path().join("assets/app.js"),
        b"changed after generation",
    )
    .unwrap();
    let snapshots = fs::read_dir(first.path().join("web.rs.assets")).unwrap();
    let mut snapshot_hashes = snapshots
        .map(|entry| sha256(&fs::read(entry.unwrap().path()).unwrap()))
        .collect::<Vec<_>>();
    snapshot_hashes.sort();
    let mut expected = parsed
        .files
        .iter()
        .map(|record| record.sha256.clone())
        .collect::<Vec<_>>();
    expected.sort();
    assert_eq!(snapshot_hashes, expected);
    let generated = fs::read_to_string(first.path().join("web.rs")).unwrap();
    assert!(generated.contains("pub static ASSETS:"));
    assert!(generated.contains("pub const MANIFEST:"));
    assert!(generated.contains("pub const DIGEST:"));
}

#[test]
fn development_provider_refreshes_bytes_and_validators() {
    let input = directory();
    let directory = DirectoryAssets::new(input.path()).unwrap();
    let before = directory.response("/assets/app.js", &Method::GET, &HeaderMap::new());
    fs::write(input.path().join("assets/app.js"), b"updated").unwrap();
    let after = directory.response("/assets/app.js", &Method::GET, &HeaderMap::new());
    assert_eq!(after.body().as_ref(), b"updated");
    assert_ne!(
        before.headers()[header::ETAG],
        after.headers()[header::ETAG]
    );
    let head = directory.response("/assets/app.js", &Method::HEAD, &HeaderMap::new());
    assert!(head.body().is_empty());
    assert_eq!(head.headers(), after.headers());
}

#[test]
fn generation_rejects_unsafe_names_and_empty_trees() {
    let empty = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    assert!(build::generate_at(empty.path(), output.path(), "web.rs").is_err());
    fs::write(empty.path().join("with space.txt"), b"no").unwrap();
    assert!(build::generate_at(empty.path(), output.path(), "web.rs").is_err());
    assert!(build::generate_at(empty.path(), output.path(), "../web.rs").is_err());
}

#[cfg(unix)]
#[test]
fn links_are_rejected_during_generation_and_hot_reload() {
    use std::os::unix::fs::symlink;
    let input = directory();
    let output = tempfile::tempdir().unwrap();
    let directory = DirectoryAssets::new(input.path()).unwrap();
    fs::hard_link(
        input.path().join("index.html"),
        input.path().join("hard.html"),
    )
    .unwrap();
    assert!(build::generate_at(input.path(), output.path(), "web.rs").is_err());
    assert_eq!(
        directory
            .response("/hard.html", &Method::GET, &HeaderMap::new())
            .status(),
        StatusCode::NOT_FOUND
    );
    fs::remove_file(input.path().join("hard.html")).unwrap();
    fs::remove_file(input.path().join("assets/app.js")).unwrap();
    symlink(
        input.path().join("index.html"),
        input.path().join("assets/app.js"),
    )
    .unwrap();
    assert!(build::generate_at(input.path(), output.path(), "web.rs").is_err());
    assert_eq!(
        directory
            .response("/assets/app.js", &Method::GET, &HeaderMap::new())
            .status(),
        StatusCode::NOT_FOUND
    );
    let aliased_root = input.path().with_extension("alias");
    symlink(input.path(), &aliased_root).unwrap();
    assert!(DirectoryAssets::new(&aliased_root).is_err());
    fs::remove_file(aliased_root).unwrap();
}

#[cfg(unix)]
#[test]
fn replaced_directory_cannot_redirect_dev_reads() {
    use std::os::unix::fs::symlink;
    let input = directory();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("app.js"), b"must not serve").unwrap();
    let directory = DirectoryAssets::new(input.path()).unwrap();
    fs::rename(
        input.path().join("assets"),
        input.path().join("original-assets"),
    )
    .unwrap();
    symlink(outside.path(), input.path().join("assets")).unwrap();
    assert_eq!(
        directory
            .response("/assets/app.js", &Method::GET, &HeaderMap::new())
            .status(),
        StatusCode::NOT_FOUND
    );
}
