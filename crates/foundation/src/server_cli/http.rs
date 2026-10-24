//! Axum extraction failures at the shared HTTP error boundary. Product Serde
//! definitions retain ownership of their current field and semantic contract.

use crate::error::{ErrorEnvelope, RequestId};
use axum::{
    Json,
    extract::{FromRequest, FromRequestParts, Path, Query, Request, rejection::JsonRejection},
    http::{Extensions, StatusCode, header, request::Parts},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::de::DeserializeOwned;

/// Apply the same bounded correlation and error policy as xcss's native
/// transport. Install this outside product input/authentication middleware.
pub async fn request_context_middleware(request: Request, next: Next) -> Response {
    use tower::ServiceExt;
    match crate::server_runtime::request_service(next)
        .oneshot(request)
        .await
    {
        Ok(response) => response,
        Err(never) => match never {},
    }
}

fn json_data_reason(error: &dyn std::error::Error) -> &'static str {
    let mut source = error.source();
    while let Some(error) = source {
        let json_error = error.downcast_ref::<serde_json::Error>().or_else(|| {
            error
                .downcast_ref::<serde_path_to_error::Error<serde_json::Error>>()
                .map(|error| error.inner())
        });
        if let Some(error) = json_error {
            // Inspect only the serializer's stable category prefixes. Neither
            // the error text nor submitted field names/values leave this scope.
            let message = error.to_string();
            return if message.starts_with("missing field ") {
                "MISSING_FIELD"
            } else if message.starts_with("unknown field ") {
                "UNKNOWN_FIELD"
            } else if message.starts_with("invalid type: ")
                && !message.contains(", expected struct ")
            {
                "TYPE_MISMATCH"
            } else {
                "JSON_STRUCTURE"
            };
        }
        source = error.source();
    }
    "JSON_STRUCTURE"
}

#[derive(Debug, Clone, Copy)]
pub struct ContractJson<T>(pub T);
#[derive(Debug, Clone, Copy)]
pub struct ContractQuery<T>(pub T);
#[derive(Debug, Clone, Copy)]
pub struct ContractPath<T>(pub T);

fn rejected(status: StatusCode, reason: &'static str, extensions: &Extensions) -> Response {
    let (code, message) = if status == StatusCode::PAYLOAD_TOO_LARGE {
        (
            "payload_too_large",
            "The request body exceeds its size limit.",
        )
    } else if status.is_server_error() {
        (
            "internal_error",
            "The request endpoint could not process its input.",
        )
    } else {
        (
            "contract_violation",
            "The request does not satisfy the current input contract.",
        )
    };
    let mut envelope = super::failure(code, message);
    envelope.details.insert("reason".into(), reason.into());
    envelope.request_id = extensions.get::<RequestId>().cloned().or_else(|| {
        extensions
            .get::<String>()
            .and_then(|id| RequestId::new(id).ok())
    });
    error_response(status, envelope)
}

fn error_response(status: StatusCode, error: ErrorEnvelope) -> Response {
    let mut response = (status, Json(error)).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        "no-store".parse().expect("static header"),
    );
    response
}

impl<T, S> FromRequest<S> for ContractJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        let extensions = request.extensions().clone();
        Json::<T>::from_request(request, state)
            .await
            .map(|Json(value)| Self(value))
            .map_err(|error| {
                let (status, reason) = match &error {
                    JsonRejection::JsonDataError(error) => {
                        (StatusCode::BAD_REQUEST, json_data_reason(error))
                    }
                    JsonRejection::JsonSyntaxError(_) => (StatusCode::BAD_REQUEST, "INVALID_JSON"),
                    JsonRejection::MissingJsonContentType(_) => {
                        (StatusCode::UNSUPPORTED_MEDIA_TYPE, "CONTENT_TYPE_REQUIRED")
                    }
                    _ if error.status() == StatusCode::PAYLOAD_TOO_LARGE => {
                        (StatusCode::PAYLOAD_TOO_LARGE, "BODY_LIMIT")
                    }
                    _ => (StatusCode::BAD_REQUEST, "INVALID_JSON"),
                };
                rejected(status, reason, &extensions)
            })
    }
}

impl<T, S> FromRequestParts<S> for ContractQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Query::<T>::from_request_parts(parts, state)
            .await
            .map(|Query(value)| Self(value))
            .map_err(|_| rejected(StatusCode::BAD_REQUEST, "INVALID_QUERY", &parts.extensions))
    }
}

impl<T, S> FromRequestParts<S> for ContractPath<T>
where
    T: DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = Response;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Path::<T>::from_request_parts(parts, state)
            .await
            .map(|Path(value)| Self(value))
            .map_err(|error| rejected(error.status(), "INVALID_PATH", &parts.extensions))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Router,
        body::{Body, to_bytes},
        extract::DefaultBodyLimit,
        routing::{get, post},
    };
    use serde::Deserialize;
    use tower::ServiceExt;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Input {
        number: u32,
    }

    async fn json(ContractJson(input): ContractJson<Input>) -> String {
        input.number.to_string()
    }
    async fn query(ContractQuery(input): ContractQuery<Input>) -> String {
        input.number.to_string()
    }
    async fn path(ContractPath(number): ContractPath<u32>) -> String {
        number.to_string()
    }

    #[tokio::test]
    async fn extraction_errors_are_flat_safe_bounded_and_uncacheable() {
        let router = Router::new()
            .route("/json", post(json))
            .route("/query", get(query))
            .route("/path/{number}", get(path))
            .layer(DefaultBodyLimit::max(64));
        for (method, uri, content_type, body, status, code, reason) in [
            (
                "POST",
                "/json",
                Some("application/json"),
                r#"{"number":"SECRET"}"#,
                400,
                "contract_violation",
                "TYPE_MISMATCH",
            ),
            (
                "POST",
                "/json",
                Some("application/json"),
                r#"{"number":2,"SECRET":2}"#,
                400,
                "contract_violation",
                "UNKNOWN_FIELD",
            ),
            (
                "POST",
                "/json",
                Some("application/json"),
                "{}",
                400,
                "contract_violation",
                "MISSING_FIELD",
            ),
            (
                "POST",
                "/json",
                Some("application/json"),
                "[]",
                400,
                "contract_violation",
                "JSON_STRUCTURE",
            ),
            (
                "POST",
                "/json",
                Some("application/json"),
                r#"{"number":SECRET}"#,
                400,
                "contract_violation",
                "INVALID_JSON",
            ),
            (
                "POST",
                "/json",
                None,
                "SECRET",
                415,
                "contract_violation",
                "CONTENT_TYPE_REQUIRED",
            ),
            (
                "POST",
                "/json",
                Some("application/json"),
                "SECRETSECRETSECRETSECRETSECRETSECRETSECRETSECRETSECRETSECRETSECRET",
                413,
                "payload_too_large",
                "BODY_LIMIT",
            ),
            (
                "GET",
                "/query?number=SECRET",
                None,
                "",
                400,
                "contract_violation",
                "INVALID_QUERY",
            ),
            (
                "GET",
                "/path/SECRET",
                None,
                "",
                400,
                "contract_violation",
                "INVALID_PATH",
            ),
        ] {
            let mut builder = Request::builder().method(method).uri(uri);
            if let Some(content_type) = content_type {
                builder = builder.header(header::CONTENT_TYPE, content_type);
            }
            let mut request = builder.body(Body::from(body)).unwrap();
            request.extensions_mut().insert("request-1".to_string());
            let response = router.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status().as_u16(), status);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
            assert!(!std::str::from_utf8(&bytes).unwrap().contains("SECRET"));
            let error: ErrorEnvelope = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(error.code.as_str(), code);
            assert_eq!(error.details["reason"], reason);
            assert_eq!(error.request_id.unwrap().as_str(), "request-1");
        }
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/json")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"number":7}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn router_middleware_correlates_extractor_and_product_errors() {
        let router = Router::new()
            .route("/json", post(json))
            .layer(axum::middleware::from_fn(request_context_middleware));
        let response = router
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/json")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header("x-request-id", "trace-123")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.headers()["x-request-id"], "trace-123");
        let bytes = to_bytes(response.into_body(), 4096).await.unwrap();
        let error: ErrorEnvelope = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(error.request_id.unwrap().as_str(), "trace-123");
        assert_eq!(error.details["reason"], "MISSING_FIELD");
    }
}
