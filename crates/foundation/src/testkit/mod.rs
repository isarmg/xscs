//! Test-only assertions shared by wire adapters and their consumers.
//! Add this crate as a dev-dependency, never as a production service dependency.

mod management;
pub use management::assert_administrator_management_http_contract;

use crate::contracts::{
    ADMIN_LOGIN_PATH, ADMIN_LOGOUT_PATH, ADMIN_SESSION_PATH, AdministratorSession, ErrorEnvelope,
};
use axum::{
    body::{Body, to_bytes},
    http::{HeaderValue, Method, Request, StatusCode, header},
    response::Response,
};
use std::future::Future;

struct PendingLoginBody;
impl http_body::Body for PendingLoginBody {
    type Data = axum::body::Bytes;
    type Error = std::convert::Infallible;
    fn poll_frame(
        self: std::pin::Pin<&mut Self>,
        _: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
        std::task::Poll::Pending
    }
}

pub const ADMINISTRATOR_USERNAME: &str = "admin";
pub const ADMINISTRATOR_PASSWORD: &str = "correct horse battery";

/// Internal fixtures exercise the production SQLite store; no alternate
/// account format or process-local session implementation is shipped.
#[cfg(test)]
pub(crate) async fn administrator_fixture(
    username: &str,
    password: &str,
) -> crate::admin_sqlite::SqliteAdministratorStore {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::raw_sql(crate::admin_sqlite::ADMIN_PERSISTENT_DDL)
        .execute(&pool)
        .await
        .unwrap();
    initialize_administrator_fixture(&pool, username, password, 1).await;
    crate::admin_sqlite::SqliteAdministratorStore::new(pool)
}

#[cfg(test)]
pub(crate) async fn initialize_administrator_fixture(
    pool: &sqlx::SqlitePool,
    username: &str,
    password: &str,
    now_micros: u64,
) {
    let now = i64::try_from(now_micros).unwrap();
    sqlx::query(crate::admin_sqlite::ADMINISTRATOR_INSERT_SQL)
        .bind("administrator-1")
        .bind(username)
        .bind(crate::admin_auth::hash_password(password).unwrap())
        .bind(1_i64)
        .bind(1_i64)
        .bind(now)
        .bind(now)
        .bind(Option::<i64>::None)
        .execute(pool)
        .await
        .unwrap();
}
const LOGIN_JSON: &str = r#"{"username":"admin","password":"correct horse battery"}"#;
const REQUEST_ID: &str = "testkit-request-1";

fn request(method: Method, path: &str, body: impl Into<Body>) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(path)
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::HOST, "127.0.0.1")
        .header(header::ORIGIN, "http://127.0.0.1")
        .header("sec-fetch-site", "same-origin")
        .header("x-request-id", REQUEST_ID)
        .body(body.into())
        .unwrap()
}

async fn assert_error(response: Response, status: StatusCode, code: &str) {
    assert_eq!(response.status(), status);
    assert_eq!(response.headers()[header::CONTENT_TYPE], "application/json");
    assert!(
        response.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("no-store")
    );
    let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
    let error: ErrorEnvelope = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(error.code.as_str(), code);
    assert!(!error.message.contains(ADMINISTRATOR_PASSWORD));
}

/// Runs the same administrator wire contract against either adapter.
///
/// The supplied application must contain the test administrator above and use
/// loopback-development origin mode. The closure supplies the actual socket
/// peer through its adapter; network identity is not used for admission.
pub async fn assert_administrator_http_contract<F, Fut>(send: F, product_id: &str)
where
    F: Fn(Request<Body>) -> Fut,
    Fut: Future<Output = Response>,
{
    let cookie = format!("admin-{product_id}-session={}", "A".repeat(43));
    for separate_lines in [false, true] {
        let mut ambiguous = request(Method::POST, ADMIN_LOGIN_PATH, LOGIN_JSON);
        if separate_lines {
            ambiguous
                .headers_mut()
                .append(header::COOKIE, HeaderValue::from_str(&cookie).unwrap());
            ambiguous
                .headers_mut()
                .append(header::COOKIE, HeaderValue::from_str(&cookie).unwrap());
        } else {
            ambiguous.headers_mut().insert(
                header::COOKIE,
                HeaderValue::from_str(&format!("{cookie}; {cookie}")).unwrap(),
            );
        }
        let response = send(ambiguous).await;
        assert!(!response.headers().contains_key(header::SET_COOKIE));
        assert_error(response, StatusCode::BAD_REQUEST, "auth.invalid_cookie").await;
    }
    for name in [
        header::HOST,
        header::ORIGIN,
        header::HeaderName::from_static("sec-fetch-site"),
    ] {
        let mut duplicate = request(Method::POST, ADMIN_LOGIN_PATH, LOGIN_JSON);
        let value = duplicate.headers()[&name].clone();
        duplicate.headers_mut().append(name, value);
        assert_error(
            send(duplicate).await,
            StatusCode::FORBIDDEN,
            "auth.origin_rejected",
        )
        .await;
    }
    let mut cross_origin = request(Method::POST, ADMIN_LOGIN_PATH, LOGIN_JSON);
    cross_origin.headers_mut().insert(
        header::ORIGIN,
        HeaderValue::from_static("http://example.invalid"),
    );
    assert_error(
        send(cross_origin).await,
        StatusCode::FORBIDDEN,
        "auth.origin_rejected",
    )
    .await;

    let mut ambiguous = request(Method::POST, ADMIN_LOGIN_PATH, LOGIN_JSON);
    *ambiguous.uri_mut() = format!("http://127.0.0.1{ADMIN_LOGIN_PATH}")
        .parse()
        .unwrap();
    assert_error(
        send(ambiguous).await,
        StatusCode::FORBIDDEN,
        "auth.origin_rejected",
    )
    .await;

    let unknown = request(
        Method::POST,
        ADMIN_LOGIN_PATH,
        r#"{"username":"admin","password":"correct horse battery","extra":true}"#,
    );
    assert_error(
        send(unknown).await,
        StatusCode::BAD_REQUEST,
        "auth.invalid_request",
    )
    .await;
    let oversized = request(Method::POST, ADMIN_LOGIN_PATH, vec![b' '; 16 * 1024 + 1]);
    assert_error(
        send(oversized).await,
        StatusCode::PAYLOAD_TOO_LARGE,
        "auth.body_too_large",
    )
    .await;

    // Content-Type is a singleton parsed field, not a prefix before a semicolon.
    for value in [
        None,
        Some("text/plain"),
        Some("application/json; broken"),
        Some("application/json; charset=\"unfinished"),
    ] {
        let mut invalid = request(Method::POST, ADMIN_LOGIN_PATH, LOGIN_JSON);
        invalid.headers_mut().remove(header::CONTENT_TYPE);
        if let Some(value) = value {
            invalid
                .headers_mut()
                .insert(header::CONTENT_TYPE, HeaderValue::from_str(value).unwrap());
        }
        assert_error(
            send(invalid).await,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "auth.content_type_required",
        )
        .await;
    }
    let mut duplicate_type = request(Method::POST, ADMIN_LOGIN_PATH, LOGIN_JSON);
    duplicate_type.headers_mut().append(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    assert_error(
        send(duplicate_type).await,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "auth.content_type_required",
    )
    .await;

    // Hold all 32 unfinished readers through the real adapter, not a test-only limiter.
    let mut readers = Vec::new();
    for _ in 0..32 {
        let mut pending = Box::pin(send(request(
            Method::POST,
            ADMIN_LOGIN_PATH,
            Body::new(PendingLoginBody),
        )));
        std::future::poll_fn(|cx| {
            assert!(pending.as_mut().poll(cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        readers.push(pending);
    }
    let mut overflow = request(Method::POST, ADMIN_LOGIN_PATH, LOGIN_JSON);
    overflow
        .headers_mut()
        .insert("x-forwarded-for", HeaderValue::from_static("192.0.2.77"));
    let limited = send(overflow).await;
    assert_eq!(limited.headers()[header::RETRY_AFTER], "1");
    assert_error(limited, StatusCode::TOO_MANY_REQUESTS, "auth.body_capacity").await;
    drop(readers);
    let mut released = request(Method::POST, ADMIN_LOGIN_PATH, LOGIN_JSON);
    released.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("Application/JSON; charset=utf-8"),
    );
    assert_eq!(send(released).await.status(), StatusCode::OK);

    let mut duplicate_id = request(Method::POST, ADMIN_LOGIN_PATH, LOGIN_JSON);
    duplicate_id
        .headers_mut()
        .append("x-request-id", HeaderValue::from_static(REQUEST_ID));
    assert_error(
        send(duplicate_id).await,
        StatusCode::BAD_REQUEST,
        "request.duplicate_id",
    )
    .await;

    let bad_login = request(
        Method::POST,
        ADMIN_LOGIN_PATH,
        r#"{"username":"admin","password":"wrong"}"#,
    );
    let response = send(bad_login).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
    let error: ErrorEnvelope = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(error.code.as_str(), "auth.invalid_credentials");
    assert_eq!(error.request_id.unwrap().as_str(), REQUEST_ID);

    // Absolute-form requests supply authority when no Host field is present.
    let mut login = request(Method::POST, ADMIN_LOGIN_PATH, LOGIN_JSON);
    login.headers_mut().remove(header::HOST);
    *login.uri_mut() = format!("http://127.0.0.1{ADMIN_LOGIN_PATH}")
        .parse()
        .unwrap();
    let response = send(login).await;
    assert_eq!(response.status(), StatusCode::OK);
    let cookie = response.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .to_owned();
    assert!(cookie.starts_with(&format!("admin-{product_id}-session=")));
    assert!(cookie.ends_with("; Path=/; HttpOnly; SameSite=Strict"));
    let cookie = cookie.split(';').next().unwrap();
    let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
    let login_session: AdministratorSession = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(login_session.username, ADMINISTRATOR_USERNAME);

    for duplicate_field_lines in [false, true] {
        let mut duplicate = request(Method::GET, ADMIN_SESSION_PATH, Body::empty());
        if duplicate_field_lines {
            duplicate
                .headers_mut()
                .append(header::COOKIE, HeaderValue::from_str(cookie).unwrap());
            duplicate
                .headers_mut()
                .append(header::COOKIE, HeaderValue::from_str(cookie).unwrap());
        } else {
            duplicate.headers_mut().insert(
                header::COOKIE,
                HeaderValue::from_str(&format!("{cookie}; {cookie}")).unwrap(),
            );
        }
        assert_error(
            send(duplicate).await,
            StatusCode::UNAUTHORIZED,
            "auth.session_required",
        )
        .await;
    }

    let mut restore = request(Method::GET, ADMIN_SESSION_PATH, Body::empty());
    restore
        .headers_mut()
        .insert(header::COOKIE, HeaderValue::from_str(cookie).unwrap());
    let response = send(restore).await;
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
    let session: AdministratorSession = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(session.csrf_token, login_session.csrf_token);
    let restore_request = || {
        let mut value = request(Method::GET, ADMIN_SESSION_PATH, Body::empty());
        value
            .headers_mut()
            .insert(header::COOKIE, HeaderValue::from_str(cookie).unwrap());
        value
    };
    let (left, right) = tokio::join!(send(restore_request()), send(restore_request()));
    for response in [left, right] {
        assert_eq!(response.status(), StatusCode::OK);
        let restored: AdministratorSession =
            serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap()).unwrap();
        assert_eq!(restored.csrf_token, session.csrf_token);
    }
    let mut logout = request(Method::POST, ADMIN_LOGOUT_PATH, Body::empty());
    logout
        .headers_mut()
        .insert(header::COOKIE, HeaderValue::from_str(cookie).unwrap());
    assert_error(
        send(logout).await,
        StatusCode::FORBIDDEN,
        "auth.csrf_rejected",
    )
    .await;

    let mut logout = request(Method::POST, ADMIN_LOGOUT_PATH, Body::empty());
    logout
        .headers_mut()
        .insert(header::COOKIE, HeaderValue::from_str(cookie).unwrap());
    logout.headers_mut().insert(
        "x-csrf-token",
        HeaderValue::from_str(&session.csrf_token).unwrap(),
    );
    let response = send(logout).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert!(
        response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .contains("Max-Age=0")
    );
    let mut revoked = request(Method::GET, ADMIN_SESSION_PATH, Body::empty());
    revoked
        .headers_mut()
        .insert(header::COOKIE, HeaderValue::from_str(cookie).unwrap());
    assert_error(
        send(revoked).await,
        StatusCode::UNAUTHORIZED,
        "auth.session_required",
    )
    .await;
}
