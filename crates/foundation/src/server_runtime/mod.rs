//! Shared server lifecycle and health primitives. Product routers remain outside this crate.

mod http1;
mod listeners;
mod shutdown;
mod tasks;
mod transport;
pub use listeners::BoundListeners;
pub use shutdown::{
    LifecycleParticipant, ProcessSignals, ShutdownFailure, ShutdownLimits, ShutdownPhase,
    ShutdownReport,
};
pub use tasks::{TrackedTasks, WorkClosed, WorkScope};
pub use transport::{Http1Limits, HttpServer};

use async_trait::async_trait;
use axum::{
    Json, Router,
    extract::{Request, State as AxumState},
    http::{HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
};
use futures_util::FutureExt;
use serde::Serialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc, time::Duration};
use tokio::sync::{RwLock, watch};

pub const LIVENESS_PATH: &str = "/healthz";
pub const READINESS_PATH: &str = "/readyz";
/// Exact platform roots/subtrees that filesystem consumers must reserve.
/// This does not reserve unrelated files beneath `/api` or `/api/v1`.
pub const PLATFORM_RESERVED_PATHS: &[&str] = &[
    LIVENESS_PATH,
    READINESS_PATH,
    "/api/v1/auth",
    crate::contracts::ADMINISTRATORS_PATH,
];
pub const DEFAULT_REQUEST_BODY_BYTES: usize = 1024 * 1024;
pub const HEALTH_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
pub const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(2);
pub const SHUTDOWN_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ProductDescriptor {
    pub id: String,
    pub version: String,
    pub common_revision: String,
    pub profile: String,
    pub capabilities: Vec<String>,
}

impl ProductDescriptor {
    pub fn validate(&self) -> Result<(), Error> {
        for (label, value) in [
            ("id", &self.id),
            ("version", &self.version),
            ("profile", &self.profile),
        ] {
            if value.is_empty()
                || value.len() > crate::log::MAX_IDENTIFIER_BYTES
                || !value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
            {
                return Err(Error::InvalidDescriptor(label));
            }
        }
        let bound = self.common_revision.len() == 40
            && self
                .common_revision
                .bytes()
                .all(|b| b.is_ascii_digit() || matches!(b, b'a'..=b'f'));
        let development = cfg!(debug_assertions) && self.common_revision == "unbound";
        if !bound && !development {
            return Err(Error::InvalidDescriptor("common_revision"));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskCriticality {
    Critical,
    Degrading,
    BestEffort,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Running,
    Stopped,
    Completed,
    Failed,
    Panicked,
    Aborted,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct TaskSnapshot {
    pub criticality: TaskCriticality,
    pub state: TaskState,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct HealthSnapshot {
    pub live: bool,
    pub ready: bool,
    pub degraded: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Diagnostics {
    pub request_id: Option<String>,
    pub schema_identity: Option<crate::schema_identity::SchemaIdentity>,
    pub metrics: BTreeMap<DiagnosticMetric, Option<u64>>,
    pub product: ProductDescriptor,
    pub health: HealthSnapshot,
    pub checks: BTreeMap<String, bool>,
    pub tasks: BTreeMap<String, TaskSnapshot>,
}

/// Closed, numeric diagnostic fields: no arbitrary path, secret or error text
/// can be supplied by a product probe. Null means unsupported or unavailable.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagnosticMetric {
    AuditBacklog,
    OperationBacklog,
    SpoolPendingBytes,
    SpoolPendingRecords,
}

fn empty_metrics() -> BTreeMap<DiagnosticMetric, Option<u64>> {
    [
        DiagnosticMetric::AuditBacklog,
        DiagnosticMetric::OperationBacklog,
        DiagnosticMetric::SpoolPendingBytes,
        DiagnosticMetric::SpoolPendingRecords,
    ]
    .into_iter()
    .map(|metric| (metric, None))
    .collect()
}

#[async_trait]
trait MetricProbe: Send + Sync {
    async fn read(&self) -> Option<u64>;
}

struct ClosureMetricProbe<F>(F);

#[async_trait]
impl<F, Fut> MetricProbe for ClosureMetricProbe<F>
where
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = Option<u64>> + Send,
{
    async fn read(&self) -> Option<u64> {
        (self.0)().await
    }
}

#[async_trait]
pub trait HealthCheck: Send + Sync {
    async fn check(&self) -> bool;
}

struct ClosureHealthCheck<F>(F);

#[async_trait]
impl<F, Fut> HealthCheck for ClosureHealthCheck<F>
where
    F: Fn() -> Fut + Send + Sync,
    Fut: Future<Output = bool> + Send,
{
    async fn check(&self) -> bool {
        (self.0)().await
    }
}

pub fn health_check<F, Fut>(check: F) -> Arc<dyn HealthCheck>
where
    F: Fn() -> Fut + Send + Sync + 'static,
    Fut: Future<Output = bool> + Send + 'static,
{
    Arc::new(ClosureHealthCheck(check))
}

#[derive(Clone)]
pub struct RuntimeHandle {
    state: Arc<RwLock<RuntimeState>>,
    shutdown: watch::Sender<bool>,
}

impl RuntimeHandle {
    pub fn shutdown(&self) {
        // Retain the request even before the runtime or HTTP listener subscribes.
        self.shutdown.send_replace(true);
    }
    pub fn shutdown_signal(&self) -> watch::Receiver<bool> {
        self.shutdown.subscribe()
    }
    pub async fn health(&self) -> HealthSnapshot {
        let mut health = self.state.read().await.health();
        health.ready &= !*self.shutdown.borrow();
        health
    }
    pub async fn diagnostics(&self) -> Diagnostics {
        self.state.read().await.diagnostics()
    }
}

struct RuntimeState {
    product: ProductDescriptor,
    schema_identity: Option<crate::schema_identity::SchemaIdentity>,
    metrics: BTreeMap<DiagnosticMetric, Option<u64>>,
    checks: BTreeMap<String, bool>,
    tasks: BTreeMap<String, TaskSnapshot>,
    shutting_down: bool,
}

impl RuntimeState {
    fn health(&self) -> HealthSnapshot {
        let critical_failed = self.tasks.values().any(|t| {
            t.criticality == TaskCriticality::Critical
                && !matches!(t.state, TaskState::Running | TaskState::Stopped)
        });
        let degraded = self.tasks.values().any(|t| {
            t.criticality == TaskCriticality::Degrading
                && !matches!(t.state, TaskState::Running | TaskState::Stopped)
        });
        HealthSnapshot {
            live: !critical_failed,
            ready: !self.shutting_down
                && !critical_failed
                && self.checks.values().all(|ready| *ready),
            degraded,
        }
    }
    fn diagnostics(&self) -> Diagnostics {
        Diagnostics {
            request_id: None,
            schema_identity: self.schema_identity.clone(),
            metrics: self.metrics.clone(),
            product: self.product.clone(),
            health: self.health(),
            checks: self.checks.clone(),
            tasks: self.tasks.clone(),
        }
    }
}

pub struct ServerRuntime {
    handle: RuntimeHandle,
    checks: Vec<(String, Arc<dyn HealthCheck>)>,
    metrics: Vec<(DiagnosticMetric, Arc<dyn MetricProbe>)>,
    tasks: Vec<TaskRegistration>,
}
struct TaskRegistration {
    name: String,
    criticality: TaskCriticality,
    task: Box<SupervisedTask>,
}
type SupervisedTask = dyn FnOnce(watch::Receiver<bool>) -> Pin<Box<dyn Future<Output = Result<(), String>> + Send>>
    + Send;

pub struct ServerRuntimeBuilder {
    product: ProductDescriptor,
    schema_identity: Option<crate::schema_identity::SchemaIdentity>,
    metrics: Vec<(DiagnosticMetric, Arc<dyn MetricProbe>)>,
    checks: Vec<(String, Arc<dyn HealthCheck>)>,
    tasks: Vec<TaskRegistration>,
}

impl ServerRuntime {
    pub fn builder(product: ProductDescriptor) -> ServerRuntimeBuilder {
        ServerRuntimeBuilder {
            product,
            schema_identity: None,
            metrics: vec![],
            checks: vec![],
            tasks: vec![],
        }
    }
    pub fn handle(&self) -> RuntimeHandle {
        self.handle.clone()
    }

    pub async fn run_until_shutdown(mut self) {
        let mut runtime_shutdown = self.handle.shutdown_signal();
        let joins = tokio_util::task::TaskTracker::new();
        for registration in self.tasks.drain(..) {
            let state = self.handle.state.clone();
            let shutdown = self.handle.shutdown_signal();
            let shutdown_requested = shutdown.clone();
            let task_handle = self.handle.clone();
            joins.spawn(async move {
                let outcome =
                    std::panic::AssertUnwindSafe(
                        async move { (registration.task)(shutdown).await },
                    )
                    .catch_unwind()
                    .await;
                let task_state = match outcome {
                    Ok(Ok(())) if *shutdown_requested.borrow() => TaskState::Stopped,
                    Ok(Ok(())) => TaskState::Completed,
                    Ok(Err(_)) => TaskState::Failed,
                    Err(_) => TaskState::Panicked,
                };
                let mut guard = state.write().await;
                if let Some(snapshot) = guard.tasks.get_mut(&registration.name) {
                    snapshot.state = task_state;
                }
                if !guard.health().live {
                    task_handle.shutdown();
                }
            });
        }
        self.refresh_health().await;
        let mut health_interval = tokio::time::interval(HEALTH_REFRESH_INTERVAL);
        health_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            if *runtime_shutdown.borrow() {
                break;
            }
            tokio::select! {
                _ = wait_for_shutdown(&mut runtime_shutdown) => break,
                _ = health_interval.tick() => self.refresh_health().await,
            }
        }
        self.handle.state.write().await.shutting_down = true;
        self.handle.shutdown();
        // The process HTTP owner enforces the hard deadline. Dropping its
        // waiter must not abort tasks that still hold persistent obligations.
        joins.close();
        joins.wait().await;
    }

    pub fn refresh_health(&self) -> impl Future<Output = ()> + Send + use<> {
        let checks = self.checks.clone();
        let metrics = self.metrics.clone();
        let handle = self.handle.clone();
        async move {
            let checks = checks.into_iter().map(|(name, check)| async move {
                let ready = tokio::time::timeout(
                    HEALTH_CHECK_TIMEOUT,
                    std::panic::AssertUnwindSafe(async { check.check().await }).catch_unwind(),
                )
                .await
                .is_ok_and(|result| matches!(result, Ok(true)));
                (name, ready)
            });
            let metrics = metrics.into_iter().map(|(name, probe)| async move {
                let value = tokio::time::timeout(
                    HEALTH_CHECK_TIMEOUT,
                    std::panic::AssertUnwindSafe(async { probe.read().await }).catch_unwind(),
                )
                .await
                .ok()
                .and_then(Result::ok)
                .flatten();
                (name, value)
            });
            let (checks, metrics) = tokio::join!(
                futures_util::future::join_all(checks),
                futures_util::future::join_all(metrics),
            );
            let mut state = handle.state.write().await;
            for (name, ready) in checks {
                state.checks.insert(name, ready);
            }
            for (name, value) in metrics {
                state.metrics.insert(name, value);
            }
        }
    }
}

/// Observe both a retained shutdown request and a later notification. A closed
/// channel also stops the consumer; no task may outlive its runtime owner.
pub async fn wait_for_shutdown(shutdown: &mut watch::Receiver<bool>) {
    loop {
        if *shutdown.borrow_and_update() || shutdown.changed().await.is_err() {
            return;
        }
    }
}

/// Create a route-only runtime handle. Servers that already own their socket
/// lifecycle can adopt the xcss HTTP surface before moving workers into
/// [`ServerRuntime`]. No background task is left unsupervised by this helper.
pub fn platform_handle(product: ProductDescriptor) -> Result<RuntimeHandle, Error> {
    product.validate()?;
    let (shutdown, _) = watch::channel(false);
    Ok(RuntimeHandle {
        state: Arc::new(RwLock::new(RuntimeState {
            product,
            schema_identity: None,
            metrics: empty_metrics(),
            checks: BTreeMap::new(),
            tasks: BTreeMap::new(),
            shutting_down: false,
        })),
        shutdown,
    })
}

impl ServerRuntimeBuilder {
    pub fn with_schema_identity(
        mut self,
        identity: crate::schema_identity::SchemaIdentity,
    ) -> Self {
        self.schema_identity = Some(identity);
        self
    }

    pub fn register_metric<F, Fut>(mut self, metric: DiagnosticMetric, read: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Option<u64>> + Send + 'static,
    {
        self.metrics
            .push((metric, Arc::new(ClosureMetricProbe(read))));
        self
    }

    pub fn register_health_check(
        mut self,
        name: impl Into<String>,
        check: Arc<dyn HealthCheck>,
    ) -> Self {
        self.checks.push((name.into(), check));
        self
    }
    pub fn register_background_task<F, Fut>(
        mut self,
        name: impl Into<String>,
        criticality: TaskCriticality,
        task: F,
    ) -> Self
    where
        F: FnOnce(watch::Receiver<bool>) -> Fut + Send + 'static,
        Fut: Future<Output = Result<(), String>> + Send + 'static,
    {
        self.tasks.push(TaskRegistration {
            name: name.into(),
            criticality,
            task: Box::new(move |shutdown| Box::pin(task(shutdown))),
        });
        self
    }
    pub async fn build(self) -> Result<ServerRuntime, Error> {
        self.product.validate()?;
        if let Some(schema) = &self.schema_identity
            && (schema.validate().is_err() || schema.application != self.product.id)
        {
            return Err(Error::InvalidDescriptor("schema_identity"));
        }
        let mut seen_metrics = std::collections::BTreeSet::new();
        for (metric, _) in &self.metrics {
            if !seen_metrics.insert(*metric) {
                return Err(Error::DuplicateName(format!("{metric:?}")));
            }
        }
        let mut task_states = BTreeMap::new();
        for task in &self.tasks {
            if task_states
                .insert(
                    task.name.clone(),
                    TaskSnapshot {
                        criticality: task.criticality,
                        state: TaskState::Running,
                    },
                )
                .is_some()
            {
                return Err(Error::DuplicateName(task.name.clone()));
            }
        }
        let mut check_states = BTreeMap::new();
        for (name, _) in &self.checks {
            if check_states.insert(name.clone(), false).is_some() {
                return Err(Error::DuplicateName(name.clone()));
            }
        }
        let (shutdown, _) = watch::channel(false);
        Ok(ServerRuntime {
            handle: RuntimeHandle {
                state: Arc::new(RwLock::new(RuntimeState {
                    product: self.product,
                    schema_identity: self.schema_identity,
                    metrics: empty_metrics(),
                    checks: check_states,
                    tasks: task_states,
                    shutting_down: false,
                })),
                shutdown,
            },
            checks: self.checks,
            metrics: self.metrics,
            tasks: self.tasks,
        })
    }
}

pub fn new_request_id() -> String {
    static FALLBACK_COUNTER: AtomicU64 = AtomicU64::new(1);
    let mut bytes = [0_u8; 16];
    if getrandom::fill(&mut bytes).is_err() {
        let counter = FALLBACK_COUNTER.fetch_add(1, Ordering::Relaxed);
        let elapsed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        bytes[..8].copy_from_slice(&counter.to_be_bytes());
        bytes[8..].copy_from_slice(&(elapsed as u64).to_be_bytes());
    }
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[derive(Clone)]
struct PlatformState {
    handle: RuntimeHandle,
}

/// Compose the xcss-owned auth, health and readiness routes.
/// Request IDs are installed once by `serve`, outside product URI validation.
/// No administrator diagnostics endpoint is provided. Product routes must be merged separately.
pub fn platform_router<Store>(
    handle: RuntimeHandle,
    product_id: impl Into<String>,
    mode: crate::admin_auth::AdministratorOriginMode,
    administrator: Arc<crate::admin_core::AdministratorService<Store>>,
) -> Result<Router, crate::admin_core::Error>
where
    Store: crate::admin_core::AdministratorStore + 'static,
{
    let product_id = product_id.into();
    let auth = crate::admin_axum::administrator_router(
        product_id.clone(),
        mode,
        Arc::clone(&administrator),
    )?;
    let state = PlatformState { handle };
    let runtime = Router::new()
        .route(LIVENESS_PATH, get(liveness))
        .route(READINESS_PATH, get(readiness))
        .with_state(state);
    Ok(Router::new().merge(auth).merge(runtime))
}

async fn liveness(AxumState(state): AxumState<PlatformState>) -> Response {
    let status = if state.handle.health().await.live {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (status, [(axum::http::header::CACHE_CONTROL, "no-store")]).into_response()
}

async fn readiness(AxumState(state): AxumState<PlatformState>) -> Response {
    let ready = state.handle.health().await.ready;
    (
        if ready {
            StatusCode::OK
        } else {
            StatusCode::SERVICE_UNAVAILABLE
        },
        [(axum::http::header::CACHE_CONTROL, "no-store")],
        Json(serde_json::json!({"ready": ready})),
    )
        .into_response()
}

fn request_error(status: StatusCode, code: &'static str, request_id: &str) -> Response {
    let mut response = (
        status,
        Json(serde_json::json!({
            "code": code,
            "message": code.replace('.', " "),
            "request_id": request_id,
            "retryable": false,
            "details": {},
        })),
    )
        .into_response();
    if let Ok(value) = HeaderValue::from_str(request_id) {
        response.headers_mut().insert("x-request-id", value);
    }
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("no-store, private, max-age=0"),
    );
    response
        .extensions_mut()
        .insert(crate::admin_axum::XcssErrorResponse);
    response
}

pub fn request_service<S>(
    inner: S,
) -> tower::util::BoxCloneSyncService<Request, Response, std::convert::Infallible>
where
    S: tower::Service<Request, Response = Response, Error = std::convert::Infallible>
        + Clone
        + Send
        + Sync
        + 'static,
    S::Future: Send,
{
    tower::util::BoxCloneSyncService::new(tower::service_fn(move |request| {
        let inner = inner.clone();
        async move { Ok::<_, std::convert::Infallible>(request_id_boundary(request, inner).await) }
    }))
}

async fn request_id_boundary<S>(mut request: Request, inner: S) -> Response
where
    S: tower::Service<Request, Response = Response, Error = std::convert::Infallible>,
{
    use tower::ServiceExt;
    static HEADER: axum::http::HeaderName = axum::http::HeaderName::from_static("x-request-id");
    let request_id = match request
        .headers()
        .get_all(&HEADER)
        .iter()
        .collect::<Vec<_>>()
        .as_slice()
    {
        [] => new_request_id(),
        [value] => match value.to_str() {
            Ok(value) if crate::contracts::RequestId::new(value).is_ok() => value.to_owned(),
            _ => {
                return request_error(
                    StatusCode::BAD_REQUEST,
                    "request.invalid_id",
                    &new_request_id(),
                );
            }
        },
        _ => {
            return request_error(
                StatusCode::BAD_REQUEST,
                "request.duplicate_id",
                &new_request_id(),
            );
        }
    };
    request.extensions_mut().insert(request_id.clone());
    request.extensions_mut().insert(
        crate::contracts::RequestId::new(request_id.clone()).expect("validated request ID"),
    );
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        request.headers_mut().insert(HEADER.clone(), value);
    }
    let mut response = match std::panic::AssertUnwindSafe(inner.oneshot(request))
        .catch_unwind()
        .await
    {
        Ok(Ok(response)) => response,
        Ok(Err(never)) => match never {},
        Err(_) => request_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "platform.internal",
            &request_id,
        ),
    };
    response = correlate_error_response(response, &request_id).await;
    if let Ok(value) = HeaderValue::from_str(&request_id) {
        response.headers_mut().insert(HEADER.clone(), value);
    }
    response
}

/// Correlation of flat JSON errors is bounded even when a product accidentally
/// returns a streaming error. Success/download bodies never enter this path.
async fn correlate_error_response(response: Response, request_id: &str) -> Response {
    use axum::http::header;
    const MAX_ERROR_BYTES: usize = 16 * 1024;
    if !(response.status().is_client_error() || response.status().is_server_error())
        || response.headers().contains_key(header::CONTENT_ENCODING)
        || !response
            .headers()
            .get(header::CONTENT_TYPE)
            .is_some_and(|value| {
                value.to_str().is_ok_and(|value| {
                    value
                        .split(';')
                        .next()
                        .is_some_and(|mime| mime.trim() == "application/json")
                })
            })
    {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let bytes = match tokio::time::timeout(
        Duration::from_secs(1),
        axum::body::to_bytes(body, MAX_ERROR_BYTES),
    )
    .await
    {
        Ok(Ok(bytes)) => bytes,
        _ => {
            return request_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "platform.internal",
                request_id,
            );
        }
    };
    let Ok(mut error) = serde_json::from_slice::<crate::contracts::ErrorEnvelope>(&bytes) else {
        return Response::from_parts(parts, axum::body::Body::from(bytes));
    };
    error.request_id =
        Some(crate::contracts::RequestId::new(request_id).expect("validated request ID"));
    let Ok(bytes) = serde_json::to_vec(&error) else {
        return request_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "platform.internal",
            request_id,
        );
    };
    if bytes.len() > MAX_ERROR_BYTES {
        return request_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "platform.internal",
            request_id,
        );
    }
    parts.headers.remove(header::CONTENT_LENGTH);
    parts
        .headers
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Response::from_parts(parts, axum::body::Body::from(bytes))
}

pub fn install_panic_hook() {
    // Panic payloads may include credentials, file names, or request contents.
    std::panic::set_hook(Box::new(|_| eprintln!("runtime panic; task terminated")));
}

pub async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut terminate) => {
                tokio::select! { _ = tokio::signal::ctrl_c() => {}, _ = terminate.recv() => {} }
            }
            Err(_) => {
                let _ = tokio::signal::ctrl_c().await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid product descriptor field: {0}")]
    InvalidDescriptor(&'static str),
    #[error("duplicate runtime registration: {0}")]
    DuplicateName(String),
    #[error("HTTP server failed")]
    Http(#[from] std::io::Error),
    #[error("critical background task stopped unexpectedly")]
    CriticalTaskStopped,
    #[error("all HTTP listeners stopped unexpectedly")]
    ListenersStopped,
    #[error("product state could not close cleanly")]
    StateCloseFailed,
    #[error(transparent)]
    ShutdownIncomplete(#[from] ShutdownFailure),
}

#[cfg(test)]
mod tests {
    use super::*;
    use tower::ServiceExt;
    struct Ready;
    #[async_trait]
    impl HealthCheck for Ready {
        async fn check(&self) -> bool {
            true
        }
    }
    fn descriptor() -> ProductDescriptor {
        ProductDescriptor {
            id: "example".into(),
            version: "1.0.0".into(),
            common_revision: "0123456789abcdef0123456789abcdef01234567".into(),
            profile: "server-control-plane".into(),
            capabilities: vec!["server-runtime".into()],
        }
    }

    #[tokio::test]
    async fn readiness_requires_checks_and_critical_exit_stops_runtime() {
        let runtime = ServerRuntime::builder(descriptor())
            .register_health_check("database", Arc::new(Ready))
            .register_background_task("worker", TaskCriticality::Critical, |_shutdown| async {
                Err("stopped".into())
            })
            .build()
            .await
            .unwrap();
        runtime.refresh_health().await;
        assert!(runtime.handle().health().await.ready);
        let handle = runtime.handle();
        runtime.run_until_shutdown().await;
        let health = handle.health().await;
        assert!(!health.live && !health.ready);
    }

    #[test]
    fn request_ids_are_bounded_random_hex() {
        let id = new_request_id();
        assert_eq!(id.len(), 32);
        assert!(id.bytes().all(|b| b.is_ascii_hexdigit()));
    }

    #[tokio::test]
    async fn request_policy_injects_one_id_and_redacts_handler_panics() {
        let app = Router::new()
            .route(
                "/echo",
                get(|headers: axum::http::HeaderMap| async move {
                    headers["x-request-id"].to_str().unwrap().to_owned()
                }),
            )
            .route(
                "/panic",
                get(|| async {
                    panic!("password=secret /private/database.sqlite");
                    #[allow(unreachable_code)]
                    StatusCode::OK
                }),
            );
        let app = request_service(app);
        let response = app
            .clone()
            .oneshot(
                Request::get("/echo")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let id = response.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let body = axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap();
        assert_eq!(body.as_ref(), id.as_bytes());
        let response = app
            .oneshot(
                Request::get("/panic")
                    .header("x-request-id", "trace-123")
                    .body(axum::body::Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(response.headers()["x-request-id"], "trace-123");
        let body = axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap();
        let error: crate::contracts::ErrorEnvelope = serde_json::from_slice(&body).unwrap();
        assert_eq!(error.code.as_str(), "platform.internal");
        assert_eq!(error.request_id.unwrap().as_str(), "trace-123");
        assert!(!String::from_utf8_lossy(&body).contains("secret"));
    }

    #[tokio::test]
    async fn invalid_and_duplicate_ids_use_strict_error_envelopes() {
        let app = Router::new().route("/", get(|| async { StatusCode::OK }));
        let app = request_service(app);
        for duplicate in [false, true] {
            let mut request = Request::get("/").body(axum::body::Body::empty()).unwrap();
            request.headers_mut().insert(
                "x-request-id",
                HeaderValue::from_static(if duplicate {
                    "valid-123"
                } else {
                    "contains spaces"
                }),
            );
            if duplicate {
                request
                    .headers_mut()
                    .append("x-request-id", HeaderValue::from_static("valid-123"));
            }
            let response = app.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            let id = response.headers()["x-request-id"]
                .to_str()
                .unwrap()
                .to_owned();
            let body = axum::body::to_bytes(response.into_body(), 8192)
                .await
                .unwrap();
            let error: crate::contracts::ErrorEnvelope = serde_json::from_slice(&body).unwrap();
            assert_eq!(error.request_id.unwrap().as_str(), id);
            assert_eq!(
                error.code.as_str(),
                if duplicate {
                    "request.duplicate_id"
                } else {
                    "request.invalid_id"
                }
            );
        }
    }

    #[tokio::test(start_paused = true)]
    async fn product_error_correlation_is_consistent_and_bounded() {
        let app = request_service(
            Router::new()
                .route(
                    "/error",
                    get(|request: Request| async move {
                        assert_eq!(request.extensions().get::<String>().unwrap(), "trace-123");
                        assert_eq!(
                            request
                                .extensions()
                                .get::<crate::contracts::RequestId>()
                                .unwrap()
                                .as_str(),
                            "trace-123"
                        );
                        (
                            StatusCode::CONFLICT,
                            Json(serde_json::json!({
                                "code": "conflict", "message": "The operation conflicts.",
                                "retryable": false, "request_id": "stale-id", "details": {}
                            })),
                        )
                            .into_response()
                    }),
                )
                .route(
                    "/large",
                    get(|| async {
                        (
                            StatusCode::BAD_REQUEST,
                            [("content-type", "application/json")],
                            "x".repeat(16 * 1024 + 1),
                        )
                            .into_response()
                    }),
                )
                .route(
                    "/stalled",
                    get(|| async {
                        let body = axum::body::Body::from_stream(futures_util::stream::pending::<
                            Result<Vec<u8>, std::convert::Infallible>,
                        >());
                        (
                            StatusCode::BAD_REQUEST,
                            [("content-type", "application/json")],
                            body,
                        )
                            .into_response()
                    }),
                )
                .route(
                    "/other",
                    get(|| async {
                        (
                            StatusCode::BAD_REQUEST,
                            Json(serde_json::json!({"business": "value"})),
                        )
                            .into_response()
                    }),
                ),
        );
        for path in ["/error", "/large", "/stalled", "/other"] {
            let response = app
                .clone()
                .oneshot(
                    Request::get(path)
                        .header("x-request-id", "trace-123")
                        .body(axum::body::Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.headers()["x-request-id"], "trace-123");
            let status = response.status();
            if path != "/other" {
                assert!(
                    response.headers()["cache-control"]
                        .to_str()
                        .unwrap()
                        .split(',')
                        .any(|directive| directive.trim() == "no-store")
                );
            }
            let bytes = axum::body::to_bytes(response.into_body(), 16 * 1024)
                .await
                .unwrap();
            if path == "/other" {
                assert_eq!(status, StatusCode::BAD_REQUEST);
                assert_eq!(
                    serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
                    serde_json::json!({"business": "value"})
                );
            } else {
                let error: crate::contracts::ErrorEnvelope =
                    serde_json::from_slice(&bytes).unwrap();
                assert_eq!(error.request_id.unwrap().as_str(), "trace-123");
                assert_eq!(
                    status,
                    if path == "/error" {
                        StatusCode::CONFLICT
                    } else {
                        StatusCode::INTERNAL_SERVER_ERROR
                    }
                );
            }
        }
    }

    #[test]
    fn revision_requires_a_full_immutable_commit() {
        let mut product = descriptor();
        assert!(product.validate().is_ok());
        product.common_revision.truncate(7);
        assert!(product.validate().is_err());
        product.common_revision = "A".repeat(40);
        assert!(product.validate().is_err());
        product = descriptor();
        product.id = "p".repeat(crate::log::MAX_IDENTIFIER_BYTES + 1);
        assert!(product.validate().is_err());
    }

    #[tokio::test]
    async fn shutdown_before_subscribing_is_retained() {
        let runtime = ServerRuntime::builder(descriptor()).build().await.unwrap();
        let handle = runtime.handle();
        handle.shutdown();
        let mut signal = handle.shutdown_signal();
        tokio::time::timeout(Duration::from_secs(1), wait_for_shutdown(&mut signal))
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), runtime.run_until_shutdown())
            .await
            .unwrap();
        assert!(!handle.health().await.ready);
    }

    #[tokio::test(start_paused = true)]
    async fn uncooperative_worker_is_reported_without_aborting_at_the_deadline() {
        let runtime = ServerRuntime::builder(descriptor())
            .register_background_task("stalled", TaskCriticality::Critical, |_| {
                std::future::pending()
            })
            .build()
            .await
            .unwrap();
        let handle = runtime.handle();
        handle.shutdown();
        let listeners = BoundListeners::bind(["127.0.0.1:0".parse().unwrap()]).unwrap();
        let transport = HttpServer::new(listeners, ProcessSignals::install().unwrap());
        let result = runtime.serve(transport, Router::new()).await;
        let Err(Error::ShutdownIncomplete(failure)) = result else {
            panic!("hard deadline must fail")
        };
        assert!(!failure.report.clean);
        assert_eq!(failure.report.unfinished_ordinary_work, 1);
        assert_eq!(
            handle.diagnostics().await.tasks["stalled"].state,
            TaskState::Running
        );
        assert!(!handle.health().await.ready);
    }

    #[tokio::test]
    async fn http_lifecycle_observes_shutdown_requested_before_start() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let runtime = ServerRuntime::builder(descriptor()).build().await.unwrap();
        runtime.handle().shutdown();
        tokio::time::timeout(
            Duration::from_secs(2),
            runtime.serve(
                HttpServer::new(
                    BoundListeners::from_listener(listener).unwrap(),
                    ProcessSignals::install().unwrap(),
                ),
                Router::new(),
            ),
        )
        .await
        .unwrap()
        .unwrap();
    }

    #[tokio::test]
    async fn http_lifecycle_propagates_critical_worker_failure() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let runtime = ServerRuntime::builder(descriptor())
            .register_background_task("failed", TaskCriticality::Critical, |_| async {
                Err("failure".into())
            })
            .build()
            .await
            .unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(2),
            runtime.serve(
                HttpServer::new(
                    BoundListeners::from_listener(listener).unwrap(),
                    ProcessSignals::install().unwrap(),
                ),
                Router::new(),
            ),
        )
        .await
        .unwrap();
        assert!(matches!(result, Err(Error::CriticalTaskStopped)));
    }

    #[tokio::test]
    async fn panicking_task_factory_is_recorded_and_stops_critical_runtime() {
        let runtime = ServerRuntime::builder(descriptor())
            .register_background_task("factory", TaskCriticality::Critical, |_| {
                panic!("factory panic");
                #[allow(unreachable_code)]
                std::future::ready(Ok(()))
            })
            .build()
            .await
            .unwrap();
        let handle = runtime.handle();
        tokio::time::timeout(Duration::from_secs(1), runtime.run_until_shutdown())
            .await
            .unwrap();
        assert_eq!(
            handle.diagnostics().await.tasks["factory"].state,
            TaskState::Panicked
        );
        assert!(!handle.health().await.live);
    }

    #[tokio::test]
    async fn degrading_and_best_effort_exits_do_not_stop_the_service() {
        let runtime = ServerRuntime::builder(descriptor())
            .register_background_task("maintenance", TaskCriticality::Degrading, |_| async {
                Err("failure".into())
            })
            .register_background_task("optional", TaskCriticality::BestEffort, |_| async {
                Err("failure".into())
            })
            .build()
            .await
            .unwrap();
        let handle = runtime.handle();
        let task = tokio::spawn(runtime.run_until_shutdown());
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let diagnostic = handle.diagnostics().await;
                if diagnostic
                    .tasks
                    .values()
                    .all(|task| task.state == TaskState::Failed)
                {
                    assert!(
                        diagnostic.health.ready
                            && diagnostic.health.live
                            && diagnostic.health.degraded
                    );
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(!task.is_finished());
        handle.shutdown();
        task.await.unwrap();
    }

    #[tokio::test]
    async fn slow_or_panicking_checks_fail_closed_without_locking_diagnostics() {
        let runtime = ServerRuntime::builder(descriptor())
            .register_health_check("stalled", health_check(std::future::pending))
            .register_health_check("panic", health_check(|| async { panic!("check panic") }))
            .build()
            .await
            .unwrap();
        let handle = runtime.handle();
        let refresh = runtime.refresh_health();
        tokio::pin!(refresh);
        tokio::select! {
            () = &mut refresh => panic!("stalled check completed without its deadline"),
            () = tokio::time::sleep(Duration::from_millis(10)) => {}
        }
        let health = tokio::time::timeout(Duration::from_millis(100), handle.health())
            .await
            .unwrap();
        assert!(!health.ready);
        tokio::time::timeout(HEALTH_CHECK_TIMEOUT + Duration::from_secs(1), refresh)
            .await
            .unwrap();
        assert!(
            handle
                .diagnostics()
                .await
                .checks
                .values()
                .all(|ready| !ready)
        );
    }

    #[tokio::test(start_paused = true)]
    async fn diagnostic_metrics_are_bounded_and_never_retain_stale_values() {
        let readings = Arc::new(AtomicU64::new(0));
        let probe_readings = readings.clone();
        let runtime = ServerRuntime::builder(descriptor())
            .with_schema_identity(
                crate::schema_identity::SchemaIdentity::new("example", "1.0.0", 1, "a".repeat(64))
                    .unwrap(),
            )
            .register_metric(DiagnosticMetric::AuditBacklog, move || {
                let readings = probe_readings.clone();
                async move {
                    if readings.fetch_add(1, Ordering::Relaxed) == 0 {
                        Some(7)
                    } else {
                        std::future::pending().await
                    }
                }
            })
            .register_metric(DiagnosticMetric::OperationBacklog, || async {
                panic!("secret diagnostic")
            })
            .build()
            .await
            .unwrap();
        runtime.refresh_health().await;
        let first = runtime.handle().diagnostics().await;
        assert_eq!(first.schema_identity.unwrap().schema_revision, 1);
        assert_eq!(first.metrics[&DiagnosticMetric::AuditBacklog], Some(7));
        assert_eq!(first.metrics[&DiagnosticMetric::OperationBacklog], None);
        assert_eq!(first.metrics[&DiagnosticMetric::SpoolPendingBytes], None);
        runtime.refresh_health().await;
        assert_eq!(
            runtime.handle().diagnostics().await.metrics[&DiagnosticMetric::AuditBacklog],
            None
        );
    }

    #[tokio::test]
    async fn patch_release_reports_its_version_while_retaining_the_data_format_identity() {
        let mut product = descriptor();
        product.version = "1.0.1".into();
        let schema =
            crate::schema_identity::SchemaIdentity::new("example", "1.0.0", 1, "a".repeat(64))
                .unwrap();
        let runtime = ServerRuntime::builder(product)
            .with_schema_identity(schema.clone())
            .build()
            .await
            .unwrap();
        let diagnostics = runtime.handle().diagnostics().await;
        assert_eq!(diagnostics.product.version, "1.0.1");
        assert_eq!(diagnostics.schema_identity, Some(schema));
    }

    #[tokio::test]
    async fn schema_must_match_product_and_diagnostic_probes_are_unique() {
        let invalid = ServerRuntime::builder(descriptor())
            .with_schema_identity(
                crate::schema_identity::SchemaIdentity::new(
                    "different",
                    "1.0.0",
                    1,
                    "a".repeat(64),
                )
                .unwrap(),
            )
            .build()
            .await;
        assert!(matches!(
            invalid,
            Err(Error::InvalidDescriptor("schema_identity"))
        ));
        let duplicate = ServerRuntime::builder(descriptor())
            .register_metric(DiagnosticMetric::AuditBacklog, || async { Some(0) })
            .register_metric(DiagnosticMetric::AuditBacklog, || async { Some(0) })
            .build()
            .await;
        assert!(matches!(duplicate, Err(Error::DuplicateName(_))));
    }

    #[tokio::test]
    async fn diagnostics_are_absent_for_both_anonymous_and_authenticated_requests() {
        use crate::admin_core::AdministratorService;
        use axum::{body::Body, http::header};
        let store = crate::testkit::administrator_fixture("admin", "correct horse battery").await;
        let runtime = ServerRuntime::builder(descriptor())
            .with_schema_identity(
                crate::schema_identity::SchemaIdentity::new("example", "1.0.0", 3, "a".repeat(64))
                    .unwrap(),
            )
            .register_health_check("database", health_check(|| async { true }))
            .register_metric(DiagnosticMetric::AuditBacklog, || async { Some(5) })
            .build()
            .await
            .unwrap();
        runtime.refresh_health().await;
        let app = platform_router(
            runtime.handle(),
            "example",
            crate::admin_auth::AdministratorOriginMode::LoopbackDevelopmentHttp,
            Arc::new(AdministratorService::new(store)),
        )
        .unwrap();
        let live = app
            .clone()
            .oneshot(Request::get(LIVENESS_PATH).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(live.status(), StatusCode::NO_CONTENT);
        assert_eq!(
            live.headers()[axum::http::header::CACHE_CONTROL],
            "no-store"
        );
        assert!(
            axum::body::to_bytes(live.into_body(), 8192)
                .await
                .unwrap()
                .is_empty()
        );
        let ready = app
            .clone()
            .oneshot(Request::get(READINESS_PATH).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(
            ready.headers()[axum::http::header::CACHE_CONTROL],
            "no-store"
        );
        let bytes = axum::body::to_bytes(ready.into_body(), 8192).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap(),
            serde_json::json!({"ready": true})
        );
        let anonymous = app
            .clone()
            .oneshot(
                Request::get("/api/v1/platform/diagnostics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(anonymous.status(), StatusCode::NOT_FOUND);
        let login = Request::post(crate::contracts::ADMIN_LOGIN_PATH)
            .header(header::HOST, "127.0.0.1")
            .header(header::ORIGIN, "http://127.0.0.1")
            .header("sec-fetch-site", "same-origin")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                r#"{"username":"admin","password":"correct horse battery"}"#,
            ))
            .unwrap();
        let response = app.clone().oneshot(login).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let cookie = response.headers()[header::SET_COOKIE]
            .to_str()
            .unwrap()
            .split(';')
            .next()
            .unwrap();
        let diagnostic = app
            .oneshot(
                Request::get("/api/v1/platform/diagnostics")
                    .header(header::COOKIE, cookie)
                    .header("x-request-id", "diag-123")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(diagnostic.status(), StatusCode::NOT_FOUND);
    }
}
