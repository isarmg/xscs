//! Bounded readers for untrusted administrator HTTP bodies, shared by both adapters.
use super::{RequestId, error};
use crate::admin_core::{ADMIN_BODY_MAX_BYTES, ADMIN_BODY_READERS_GLOBAL, ADMIN_BODY_TIMEOUT};
use axum::{
    body::{Body, to_bytes},
    extract::Request,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::Response,
};
use bytes::Bytes;
use std::{
    error::Error,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Readers {
    total: usize,
}

#[derive(Clone, Default)]
pub(super) struct BodyAdmission(Arc<Mutex<Readers>>);

struct Permit {
    admission: BodyAdmission,
}

impl Drop for Permit {
    fn drop(&mut self) {
        // A poisoned budget remains closed; never resume with uncertain counters.
        if let Ok(mut readers) = self.admission.0.lock() {
            readers.total -= 1;
        }
    }
}

#[derive(Clone, Copy)]
pub(super) enum Scope {
    Login,
    Management,
}

impl Scope {
    fn content_type(self) -> &'static str {
        match self {
            Self::Login => "auth.content_type_required",
            Self::Management => "admin.content_type_required",
        }
    }
    fn too_large(self) -> &'static str {
        match self {
            Self::Login => "auth.body_too_large",
            Self::Management => "admin.body_too_large",
        }
    }
    fn invalid(self) -> &'static str {
        match self {
            Self::Login => "auth.invalid_request",
            Self::Management => "admin.invalid_request",
        }
    }
}

impl BodyAdmission {
    fn acquire(&self, id: Option<&RequestId>) -> Result<Permit, Box<Response>> {
        let mut readers = self.0.lock().map_err(|_| {
            Box::new(error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "platform.internal",
                false,
                id,
            ))
        })?;
        if readers.total >= ADMIN_BODY_READERS_GLOBAL {
            let mut response = error(
                StatusCode::TOO_MANY_REQUESTS,
                "auth.body_capacity",
                true,
                id,
            );
            response
                .headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
            return Err(Box::new(response));
        }
        readers.total += 1;
        Ok(Permit {
            admission: self.clone(),
        })
    }

    pub(super) async fn read(
        &self,
        request: Request,
        scope: Scope,
        json: bool,
        id: Option<&RequestId>,
    ) -> Result<Bytes, Box<Response>> {
        if json {
            require_json_content_type(request.headers(), scope, id)?;
        }
        let _permit = self.acquire(id)?;
        read_bounded(request.into_body(), scope, id).await
    }
}

fn require_json_content_type(
    headers: &HeaderMap,
    scope: Scope,
    id: Option<&RequestId>,
) -> Result<(), Box<Response>> {
    let mut values = headers.get_all(header::CONTENT_TYPE).iter();
    let valid = match (values.next(), values.next()) {
        (Some(value), None) => value
            .to_str()
            .ok()
            .and_then(|value| value.parse::<mime::Mime>().ok())
            .is_some_and(|value| value.essence_str().eq_ignore_ascii_case("application/json")),
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(Box::new(error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            scope.content_type(),
            false,
            id,
        )))
    }
}

async fn read_bounded(
    body: Body,
    scope: Scope,
    id: Option<&RequestId>,
) -> Result<Bytes, Box<Response>> {
    match tokio::time::timeout(ADMIN_BODY_TIMEOUT, to_bytes(body, ADMIN_BODY_MAX_BYTES)).await {
        Ok(Ok(bytes)) => Ok(bytes),
        Ok(Err(failure)) => {
            let oversized = failure
                .source()
                .is_some_and(|source| source.is::<http_body_util::LengthLimitError>());
            Err(Box::new(error(
                if oversized {
                    StatusCode::PAYLOAD_TOO_LARGE
                } else {
                    StatusCode::BAD_REQUEST
                },
                if oversized {
                    scope.too_large()
                } else {
                    scope.invalid()
                },
                false,
                id,
            )))
        }
        Err(_) => Err(Box::new(error(
            StatusCode::REQUEST_TIMEOUT,
            "auth.body_timeout",
            true,
            id,
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body::Frame;
    use std::{
        convert::Infallible,
        pin::Pin,
        sync::atomic::{AtomicBool, Ordering},
        task::{Context, Poll},
    };

    struct PendingBody(Arc<AtomicBool>);
    impl http_body::Body for PendingBody {
        type Data = Bytes;
        type Error = Infallible;
        fn poll_frame(
            self: Pin<&mut Self>,
            _: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
            Poll::Pending
        }
    }
    impl Drop for PendingBody {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    fn request(body: Body) -> Request {
        Request::builder()
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .unwrap()
    }

    #[test]
    fn global_budget_releases_permits_without_source_identity() {
        let admission = BodyAdmission::default();
        let mut permits = Vec::new();
        for _ in 0..ADMIN_BODY_READERS_GLOBAL {
            permits.push(admission.acquire(None).unwrap());
        }
        assert_eq!(
            admission.acquire(None).err().unwrap().status(),
            StatusCode::TOO_MANY_REQUESTS
        );
        permits.pop();
        permits.push(admission.acquire(None).unwrap());
        drop(permits);
        assert_eq!(admission.0.lock().unwrap().total, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn slow_body_deadline_and_cancellation_drop_body_and_permit() {
        let admission = BodyAdmission::default();
        let dropped = Arc::new(AtomicBool::new(false));
        let id = RequestId::new("slow-body-request-1").unwrap();
        let response = admission
            .read(
                request(Body::new(PendingBody(dropped.clone()))),
                Scope::Login,
                true,
                Some(&id),
            )
            .await
            .unwrap_err();
        assert_eq!(response.status(), StatusCode::REQUEST_TIMEOUT);
        let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
        let envelope: crate::contracts::ErrorEnvelope = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(envelope.request_id.as_ref(), Some(&id));
        assert!(envelope.retryable);
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(admission.0.lock().unwrap().total, 0);
        let dropped = Arc::new(AtomicBool::new(false));
        let mut read = Box::pin(admission.read(
            request(Body::new(PendingBody(dropped.clone()))),
            Scope::Login,
            true,
            None,
        ));
        std::future::poll_fn(|cx| {
            assert!(read.as_mut().poll(cx).is_pending());
            Poll::Ready(())
        })
        .await;
        assert_eq!(admission.0.lock().unwrap().total, 1);
        drop(read);
        assert!(dropped.load(Ordering::SeqCst));
        assert_eq!(admission.0.lock().unwrap().total, 0);
    }

    #[tokio::test]
    async fn exact_size_boundary_and_malformed_transport_are_distinct_safe_errors() {
        assert_eq!(
            read_bounded(
                Body::from(vec![b' '; ADMIN_BODY_MAX_BYTES]),
                Scope::Login,
                None
            )
            .await
            .unwrap()
            .len(),
            ADMIN_BODY_MAX_BYTES
        );
        assert_eq!(
            read_bounded(
                Body::from(vec![b' '; ADMIN_BODY_MAX_BYTES + 1]),
                Scope::Login,
                None
            )
            .await
            .unwrap_err()
            .status(),
            StatusCode::PAYLOAD_TOO_LARGE
        );
        struct BrokenBody;
        impl http_body::Body for BrokenBody {
            type Data = Bytes;
            type Error = std::io::Error;
            fn poll_frame(
                self: Pin<&mut Self>,
                _: &mut Context<'_>,
            ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
                Poll::Ready(Some(Err(std::io::Error::other("SECRET transport details"))))
            }
        }
        let response = read_bounded(Body::new(BrokenBody), Scope::Login, None)
            .await
            .unwrap_err();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let bytes = to_bytes(response.into_body(), 8192).await.unwrap();
        assert!(!std::str::from_utf8(&bytes).unwrap().contains("SECRET"));
    }
}
