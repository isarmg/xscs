//! The browser client and its immutable inventory are compiled into this binary.
#[path = "leptos_assets.rs"]
mod leptos_assets;

include!(concat!(env!("OUT_DIR"), "/xcss-web-assets.rs"));

pub fn response(
    directory: Option<&xcss::web_assets::DirectoryAssets>,
    path: &str,
    method: &axum::http::Method,
    headers: &axum::http::HeaderMap,
) -> axum::response::Response {
    match directory {
        Some(directory) => directory.response(path, method, headers),
        None => leptos_assets::response(path, method, headers, |path, method, headers| {
            xcss::web_assets::response(ASSETS, path, method, headers)
        }),
    }
    .map(axum::body::Body::from)
}

/// Check the final executable bytes against its compiled inventory.
pub fn verify() -> anyhow::Result<()> {
    xcss::web_assets::verify_embedded(ASSETS, MANIFEST, DIGEST)
}

#[cfg(test)]
mod leptos_response_tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};

    #[tokio::test]
    async fn embedded_wasm_negotiates_verified_gzip_head_and_304() {
        verify().unwrap();
        let raw = ASSETS
            .iter()
            .find(|asset| asset.path.ends_with(".wasm"))
            .unwrap();
        let variant = format!("{}.gz", raw.path);
        let gzip = ASSETS.iter().find(|asset| asset.path == variant).unwrap();
        assert!(gzip.bytes.starts_with(&[0x1f, 0x8b]));
        assert!(gzip.bytes.len() < raw.bytes.len());
        let mut headers = HeaderMap::new();
        headers.insert(header::ACCEPT_ENCODING, HeaderValue::from_static("gzip"));
        let get = response(None, raw.path, &Method::GET, &headers);
        assert_eq!(get.status(), StatusCode::OK);
        assert_eq!(get.headers()[header::CONTENT_TYPE], "application/wasm");
        assert_eq!(get.headers()[header::CONTENT_ENCODING], "gzip");
        assert_eq!(get.headers()[header::VARY], "Accept-Encoding");
        let etag = get.headers()[header::ETAG].clone();
        let body = axum::body::to_bytes(get.into_body(), gzip.bytes.len())
            .await
            .unwrap();
        assert_eq!(body.as_ref(), gzip.bytes);
        let head = response(None, raw.path, &Method::HEAD, &headers);
        assert_eq!(
            head.headers()[header::CONTENT_LENGTH].to_str().unwrap(),
            gzip.bytes.len().to_string()
        );
        assert!(
            axum::body::to_bytes(head.into_body(), 0)
                .await
                .unwrap()
                .is_empty()
        );
        headers.insert(header::IF_NONE_MATCH, etag);
        let unchanged = response(None, raw.path, &Method::GET, &headers);
        assert_eq!(unchanged.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(unchanged.headers()[header::CONTENT_ENCODING], "gzip");
        assert!(
            axum::body::to_bytes(unchanged.into_body(), 0)
                .await
                .unwrap()
                .is_empty()
        );
        headers.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("gzip;q=0, *;q=1"),
        );
        let identity = response(None, raw.path, &Method::GET, &headers);
        assert_eq!(identity.status(), StatusCode::OK);
        assert!(!identity.headers().contains_key(header::CONTENT_ENCODING));
        assert_eq!(identity.headers()[header::VARY], "Accept-Encoding");
        assert_eq!(
            axum::body::to_bytes(identity.into_body(), raw.bytes.len())
                .await
                .unwrap()
                .as_ref(),
            raw.bytes
        );
    }
}
