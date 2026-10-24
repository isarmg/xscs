//! Hyper integration for the exact administrator semantics implemented by the
//! xcss Axum router. The wrapper adapts only body types;
//! it intentionally owns no second authentication handler implementation.

use crate::admin_auth::AdministratorOriginMode;
pub use crate::admin_axum::{VerifiedAdministrator, XcssErrorResponse, authenticate_request};
use crate::admin_core::{AdministratorService, AdministratorStore};
use axum::{
    Router,
    body::Body,
    http::{Request, Response},
};
use bytes::Bytes;
use http_body::Body as HttpBody;
use std::sync::Arc;
use tower::ServiceExt;

#[derive(Clone, Debug)]
pub struct HyperAdministratorRouter {
    router: Router,
}

impl HyperAdministratorRouter {
    pub fn new<Store>(
        product_id: impl Into<String>,
        mode: AdministratorOriginMode,
        service: Arc<AdministratorService<Store>>,
    ) -> Result<Self, crate::admin_core::Error>
    where
        Store: AdministratorStore + 'static,
    {
        Ok(Self {
            router: crate::admin_axum::administrator_router(product_id, mode, service)?,
        })
    }

    pub fn owns_path(path: &str) -> bool {
        path == crate::contracts::ADMINISTRATORS_PATH
            || path
                .strip_prefix(crate::contracts::ADMINISTRATORS_PATH)
                .is_some_and(|tail| tail.starts_with('/'))
            || path == "/api/v1/auth"
            || path.starts_with("/api/v1/auth/")
    }

    pub async fn handle_incoming(&self, request: Request<hyper::body::Incoming>) -> Response<Body> {
        self.handle(request).await
    }

    pub async fn handle<InputBody>(&self, request: Request<InputBody>) -> Response<Body>
    where
        InputBody: HttpBody<Data = Bytes> + Send + 'static,
        InputBody::Error: Into<axum::BoxError>,
    {
        let request = request.map(Body::new);
        match self.router.clone().oneshot(request).await {
            Ok(response) => response,
            Err(error) => match error {},
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{Method, StatusCode, header};

    #[tokio::test]
    async fn shared_persistent_management_contract() {
        let directory = tempfile::tempdir().unwrap();
        let pool = crate::sqlite::create_if_missing(
            directory.path().join("admin.sqlite3"),
            crate::sqlite::PoolOptions::new(2),
        )
        .await
        .unwrap();
        sqlx::raw_sql(crate::admin_sqlite::ADMIN_PERSISTENT_DDL)
            .execute(&pool)
            .await
            .unwrap();
        let service = Arc::new(AdministratorService::new(
            crate::admin_sqlite::SqliteAdministratorStore::new(pool),
        ));
        crate::testkit::initialize_administrator_fixture(
            service.store().pool(),
            "admin",
            crate::testkit::ADMINISTRATOR_PASSWORD,
            1,
        )
        .await;
        let router = HyperAdministratorRouter::new(
            "example-product",
            AdministratorOriginMode::LoopbackDevelopmentHttp,
            service,
        )
        .unwrap();
        crate::testkit::assert_administrator_management_http_contract(|request| {
            let router = router.clone();
            assert!(HyperAdministratorRouter::owns_path(request.uri().path()));
            async move { router.handle(request).await }
        })
        .await;
    }

    #[tokio::test]
    async fn shared_administrator_wire_contract() {
        let store = crate::testkit::administrator_fixture(
            crate::testkit::ADMINISTRATOR_USERNAME,
            crate::testkit::ADMINISTRATOR_PASSWORD,
        )
        .await;
        let router = HyperAdministratorRouter::new(
            "example-product",
            AdministratorOriginMode::LoopbackDevelopmentHttp,
            Arc::new(AdministratorService::new(store)),
        )
        .unwrap();
        crate::testkit::assert_administrator_http_contract(
            |request| {
                let router = router.clone();
                async move { router.handle(request).await }
            },
            "example-product",
        )
        .await;
    }

    #[tokio::test]
    async fn hyper_body_path_uses_the_shared_router() {
        let store = crate::testkit::administrator_fixture("admin", "correct horse battery").await;
        let router = HyperAdministratorRouter::new(
            "example-product",
            AdministratorOriginMode::LoopbackDevelopmentHttp,
            Arc::new(AdministratorService::new(store)),
        )
        .unwrap();
        let request = Request::builder()
            .method(Method::POST)
            .uri(crate::contracts::ADMIN_LOGIN_PATH)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, "http://127.0.0.1")
            .header(header::HOST, "127.0.0.1")
            .header("sec-fetch-site", "same-origin")
            .body(Body::from(
                r#"{"username":"admin","password":"correct horse battery"}"#,
            ))
            .unwrap();
        let response = router.handle(request).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            response
                .headers()
                .get(header::SET_COOKIE)
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("admin-example-product-session=")
        );
        assert!(HyperAdministratorRouter::owns_path(
            crate::contracts::ADMIN_LOGOUT_PATH
        ));
        assert!(!HyperAdministratorRouter::owns_path("/api/v1/files"));
    }
}
