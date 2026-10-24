//! Build-time compressed WASM negotiation around the verified asset responder.
fn quality(value: &str) -> Option<u16> {
    match value.split_once('.') {
        None => match value {
            "0" => Some(0),
            "1" => Some(1000),
            _ => None,
        },
        Some((whole, fraction))
            if fraction.len() <= 3 && fraction.bytes().all(|b| b.is_ascii_digit()) =>
        {
            if whole == "1" {
                return fraction.bytes().all(|b| b == b'0').then_some(1000);
            }
            if whole != "0" {
                return None;
            }
            let mut digits = fraction.to_owned();
            while digits.len() < 3 {
                digits.push('0');
            }
            digits.parse().ok()
        }
        _ => None,
    }
}

fn accepts_gzip(headers: &http::HeaderMap) -> bool {
    let mut gzip = None;
    let mut wildcard = None;
    for header in headers.get_all(http::header::ACCEPT_ENCODING) {
        let Ok(value) = header.to_str() else {
            return false;
        };
        for entry in value.split(',') {
            let mut parts = entry.trim().split(';');
            let encoding = parts.next().unwrap_or_default().trim();
            if !encoding.eq_ignore_ascii_case("gzip") && encoding != "*" {
                continue;
            }
            let mut weight = 1000;
            let mut parameters = parts;
            if let Some(parameter) = parameters.next() {
                let Some((name, value)) = parameter.trim().split_once('=') else {
                    return false;
                };
                if !name.trim().eq_ignore_ascii_case("q") {
                    return false;
                }
                let Some(q) = quality(value.trim()) else {
                    return false;
                };
                weight = q;
                if parameters.next().is_some() {
                    return false;
                }
            }
            let target = if encoding == "*" {
                &mut wildcard
            } else {
                &mut gzip
            };
            // Conflicting duplicates are resolved conservatively; explicit gzip
            // always takes precedence over wildcard, including q=0.
            *target = Some(target.map_or(weight, |previous: u16| previous.min(weight)));
        }
    }
    gzip.or(wildcard).is_some_and(|q| q > 0)
}

/// Serve a precompressed WASM variant when the client accepts gzip.
///
/// `serve` remains responsible for safe asset lookup, representation ETags and
/// GET/HEAD/conditional responses. Other files and methods pass through intact.
pub fn response<B>(
    path: &str,
    method: &http::Method,
    headers: &http::HeaderMap,
    serve: impl Fn(&str, &http::Method, &http::HeaderMap) -> http::Response<B>,
) -> http::Response<B> {
    if !path.ends_with(".wasm") || (method != http::Method::GET && method != http::Method::HEAD) {
        return serve(path, method, headers);
    }
    let mut result = if accepts_gzip(headers) {
        let compressed = serve(&format!("{path}.gz"), method, headers);
        if compressed.status() == http::StatusCode::OK
            || compressed.status() == http::StatusCode::NOT_MODIFIED
        {
            let mut compressed = compressed;
            compressed.headers_mut().insert(
                http::header::CONTENT_ENCODING,
                http::HeaderValue::from_static("gzip"),
            );
            compressed.headers_mut().insert(
                http::header::CONTENT_TYPE,
                http::HeaderValue::from_static("application/wasm"),
            );
            compressed
        } else {
            serve(path, method, headers)
        }
    } else {
        serve(path, method, headers)
    };
    if result.status() == http::StatusCode::OK || result.status() == http::StatusCode::NOT_MODIFIED
    {
        let already_varies = result
            .headers()
            .get_all(http::header::VARY)
            .iter()
            .any(|value| {
                value.to_str().is_ok_and(|v| {
                    v.split(',').any(|s| {
                        s.trim() == "*" || s.trim().eq_ignore_ascii_case("accept-encoding")
                    })
                })
            });
        if !already_varies {
            result.headers_mut().append(
                http::header::VARY,
                http::HeaderValue::from_static("Accept-Encoding"),
            );
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn gzip_quality_and_explicit_exclusion() {
        for (value, accepted) in [
            ("gzip", true),
            ("br, GZip;q=0.5", true),
            ("*;q=1, gzip;q=0", false),
            ("gzip;q=0, gzip;q=1", false),
            ("identity", false),
            ("gzip;q=1.001", false),
            ("gzip;q=NaN", false),
            ("gzip;q=0.001", true),
            ("gzip;q=0.000", false),
            ("*;q=0.8", true),
        ] {
            let mut headers = http::HeaderMap::new();
            headers.insert(http::header::ACCEPT_ENCODING, value.parse().unwrap());
            assert_eq!(accepts_gzip(&headers), accepted, "{value}");
        }
        assert!(!accepts_gzip(&http::HeaderMap::new()));
    }
    #[test]
    fn gzip_variant_keeps_representation_etag_head_and_304() {
        let serve = |path: &str, method: &http::Method, headers: &http::HeaderMap| {
            let compressed = path.ends_with(".gz");
            let etag = if compressed { "\"gzip\"" } else { "\"raw\"" };
            let unchanged = headers
                .get(http::header::IF_NONE_MATCH)
                .is_some_and(|v| v == etag);
            http::Response::builder()
                .status(if unchanged { 304 } else { 200 })
                .header(http::header::ETAG, etag)
                .header(http::header::CONTENT_TYPE, "application/octet-stream")
                .header(
                    http::header::CONTENT_LENGTH,
                    if compressed { "4" } else { "3" },
                )
                .body(if unchanged || method == http::Method::HEAD {
                    ""
                } else if compressed {
                    "gzip"
                } else {
                    "raw"
                })
                .unwrap()
        };
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::ACCEPT_ENCODING,
            http::HeaderValue::from_static("gzip"),
        );
        let get = response("/assets/app.wasm", &http::Method::GET, &headers, serve);
        assert_eq!(get.body(), &"gzip");
        assert_eq!(
            get.headers()[http::header::CONTENT_TYPE],
            "application/wasm"
        );
        assert_eq!(get.headers()[http::header::CONTENT_ENCODING], "gzip");
        assert_eq!(get.headers()[http::header::VARY], "Accept-Encoding");
        let head = response("/assets/app.wasm", &http::Method::HEAD, &headers, serve);
        assert!(head.body().is_empty());
        assert_eq!(head.headers()[http::header::CONTENT_LENGTH], "4");
        headers.insert(
            http::header::IF_NONE_MATCH,
            get.headers()[http::header::ETAG].clone(),
        );
        let cached = response("/assets/app.wasm", &http::Method::GET, &headers, serve);
        assert_eq!(cached.status(), http::StatusCode::NOT_MODIFIED);
        assert!(cached.body().is_empty());
        assert_eq!(cached.headers()[http::header::CONTENT_ENCODING], "gzip");
        headers.remove(http::header::ACCEPT_ENCODING);
        let raw = response("/assets/app.wasm", &http::Method::GET, &headers, serve);
        assert_eq!(raw.status(), http::StatusCode::OK);
        assert_eq!(raw.body(), &"raw");
        assert!(!raw.headers().contains_key(http::header::CONTENT_ENCODING));
        assert_ne!(
            raw.headers()[http::header::ETAG],
            get.headers()[http::header::ETAG]
        );
    }
    #[test]
    fn absent_compressed_asset_falls_back_and_errors_stay_errors() {
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::ACCEPT_ENCODING,
            http::HeaderValue::from_static("gzip"),
        );
        let serve = |path: &str, _: &http::Method, _: &http::HeaderMap| {
            http::Response::builder()
                .status(if path.ends_with(".gz") { 404 } else { 200 })
                .body("raw")
                .unwrap()
        };
        let raw = response("/app.wasm", &http::Method::GET, &headers, serve);
        assert_eq!(raw.status(), http::StatusCode::OK);
        assert!(!raw.headers().contains_key(http::header::CONTENT_ENCODING));
        let missing = response("/missing.wasm", &http::Method::GET, &headers, |_, _, _| {
            http::Response::builder().status(404).body("").unwrap()
        });
        assert_eq!(missing.status(), http::StatusCode::NOT_FOUND);
        let is_encoded = missing
            .headers()
            .contains_key(http::header::CONTENT_ENCODING);
        assert!(!is_encoded);
    }
}
