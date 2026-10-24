use super::*;
use crate::contracts::{ADMIN_ACCOUNT_PATH, ADMINISTRATORS_PATH};

/// Run on a fresh persistent store containing only the standard test account.
pub async fn assert_administrator_management_http_contract<F, Fut>(send: F)
where
    F: Fn(Request<Body>) -> Fut,
    Fut: Future<Output = Response>,
{
    assert_error(
        send(request(Method::POST, ADMIN_ACCOUNT_PATH, "{}")).await,
        StatusCode::UNAUTHORIZED,
        "auth.session_required",
    )
    .await;
    let login = send(request(Method::POST, ADMIN_LOGIN_PATH, LOGIN_JSON)).await;
    assert_eq!(login.status(), StatusCode::OK);
    let cookie = login.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_string();
    let session: AdministratorSession =
        serde_json::from_slice(&to_bytes(login.into_body(), 8192).await.unwrap()).unwrap();
    let authorized = |method: Method, path: &str, body: &str| {
        let mut request = request(method, path, body.to_owned());
        request
            .headers_mut()
            .insert(header::COOKIE, HeaderValue::from_str(&cookie).unwrap());
        request.headers_mut().insert(
            "x-csrf-token",
            HeaderValue::from_str(&session.csrf_token).unwrap(),
        );
        request
    };
    let create = r#"{"username":"secondary","current_password":"another correct password"}"#;
    let mut no_csrf = authorized(Method::POST, ADMIN_ACCOUNT_PATH, create);
    no_csrf.headers_mut().remove("x-csrf-token");
    assert_error(
        send(no_csrf).await,
        StatusCode::FORBIDDEN,
        "auth.csrf_rejected",
    )
    .await;
    for name in [
        header::ORIGIN,
        header::HOST,
        header::COOKIE,
        header::HeaderName::from_static("x-csrf-token"),
    ] {
        let mut duplicate = authorized(Method::POST, ADMIN_ACCOUNT_PATH, create);
        let value = duplicate.headers()[&name].clone();
        duplicate.headers_mut().append(name.clone(), value);
        let (status, code) = if name == header::COOKIE {
            (StatusCode::UNAUTHORIZED, "auth.session_required")
        } else if name.as_str() == "x-csrf-token" {
            (StatusCode::FORBIDDEN, "auth.csrf_rejected")
        } else {
            (StatusCode::FORBIDDEN, "auth.origin_rejected")
        };
        assert_error(send(duplicate).await, status, code).await;
    }
    for body in [
        r#"{"username":"secondary","password":"another correct password","role":"admin"}"#,
        r#"{"username":"secondary","username":"other","password":"another correct password"}"#,
        r#"{"username":"admin","current_password":"correct horse battery","new_password":""}"#,
        r#"{"username":"admin","current_password":"correct horse battery","new_password":null}"#,
    ] {
        assert_error(
            send(authorized(Method::POST, ADMIN_ACCOUNT_PATH, body)).await,
            StatusCode::BAD_REQUEST,
            "admin.invalid_request",
        )
        .await;
    }
    assert_error(
        send(authorized(
            Method::POST,
            ADMIN_ACCOUNT_PATH,
            &"x".repeat(16385),
        ))
        .await,
        StatusCode::PAYLOAD_TOO_LARGE,
        "admin.body_too_large",
    )
    .await;
    let mut wrong_type = authorized(Method::POST, ADMIN_ACCOUNT_PATH, create);
    wrong_type
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static("text/plain"));
    assert_error(
        send(wrong_type).await,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "admin.content_type_required",
    )
    .await;
    for (method, path) in [
        (Method::GET, ADMINISTRATORS_PATH.to_owned()),
        (Method::POST, ADMINISTRATORS_PATH.to_owned()),
        (
            Method::POST,
            format!("{ADMINISTRATORS_PATH}/{}/password", session.user_id),
        ),
        (
            Method::POST,
            format!("{ADMINISTRATORS_PATH}/{}/disable", session.user_id),
        ),
    ] {
        assert_eq!(
            send(authorized(method, &path, create)).await.status(),
            StatusCode::NOT_FOUND
        );
    }
    // The self-service endpoint uses the same transport guards on both adapters.
    let login = send(request(
        Method::POST,
        ADMIN_LOGIN_PATH,
        r#"{"username":"admin","password":"correct horse battery"}"#,
    ))
    .await;
    assert_eq!(login.status(), StatusCode::OK);
    let cookie = login.headers()[header::SET_COOKIE]
        .to_str()
        .unwrap()
        .split(';')
        .next()
        .unwrap()
        .to_owned();
    let current: AdministratorSession =
        serde_json::from_slice(&to_bytes(login.into_body(), 8192).await.unwrap()).unwrap();
    let account_request = |body: &str| {
        let mut value = request(
            Method::POST,
            crate::contracts::ADMIN_ACCOUNT_PATH,
            body.to_owned(),
        );
        value
            .headers_mut()
            .insert(header::COOKIE, HeaderValue::from_str(&cookie).unwrap());
        value.headers_mut().insert(
            "x-csrf-token",
            HeaderValue::from_str(&current.csrf_token).unwrap(),
        );
        value
    };
    let change = r#"{"username":"renamed","current_password":"correct horse battery","new_password":"my updated correct password"}"#;
    let mut no_csrf = account_request(change);
    no_csrf.headers_mut().remove("x-csrf-token");
    assert_error(
        send(no_csrf).await,
        StatusCode::FORBIDDEN,
        "auth.csrf_rejected",
    )
    .await;
    let mut cross_origin = account_request(change);
    cross_origin.headers_mut().insert(
        header::ORIGIN,
        HeaderValue::from_static("https://untrusted.example"),
    );
    assert_error(
        send(cross_origin).await,
        StatusCode::FORBIDDEN,
        "auth.origin_rejected",
    )
    .await;
    assert_error(
        send(account_request(
            r#"{"username":"renamed","current_password":"wrong password"}"#,
        ))
        .await,
        StatusCode::FORBIDDEN,
        "admin.current_password_invalid",
    )
    .await;
    assert_error(send(account_request(r#"{"username":"renamed","current_password":"correct horse battery","administrator_id":"other"}"#)).await,
        StatusCode::BAD_REQUEST, "admin.invalid_request").await;
    let changed = send(account_request(change)).await;
    assert_eq!(changed.status(), StatusCode::NO_CONTENT);
    assert!(
        changed.headers()[header::CACHE_CONTROL]
            .to_str()
            .unwrap()
            .contains("no-store")
    );
    assert_error(
        send(account_request(change)).await,
        StatusCode::UNAUTHORIZED,
        "auth.session_required",
    )
    .await;
    let login = send(request(
        Method::POST,
        ADMIN_LOGIN_PATH,
        r#"{"username":"renamed","password":"my updated correct password"}"#,
    ))
    .await;
    assert_eq!(login.status(), StatusCode::OK);
    let updated: AdministratorSession =
        serde_json::from_slice(&to_bytes(login.into_body(), 8192).await.unwrap()).unwrap();
    assert_eq!(updated.user_id, current.user_id);
    assert_eq!(updated.username, "renamed");
}
