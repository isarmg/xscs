use crate::{
    crypto::SecretBox,
    db,
    error::{AppError, AppResult},
    model::{
        ClientAuthorization, CreateDeviceRequest, DeviceName, DeviceView,
        OperationResolutionRequest, UpdateClientAuthorization,
    },
    operations::{OperationManager, OperationSummary, OperationView},
    release_contract::{API_NAMESPACE, API_VERSION_PREFIX},
};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{
        DefaultBodyLimit, Extension, Request, State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, patch, post},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore},
    time::{Instant, timeout},
};
use xcss::admin_auth::AdministratorOriginMode;
use xcss::admin_core::AdministratorService;
use xcss::admin_sqlite::SqliteAdministratorStore;
use xcss::server_cli::{ContractJson, ContractPath};
use xscs_protocol::{
    Binding, ClientMessage, Command, DeliveryMode, MAX_MESSAGE_BYTES, ManagerMessage,
    WEBSOCKET_SUBPROTOCOL,
};

#[derive(Clone)]
pub struct WorkerState {
    pub pool: sqlx::SqlitePool,
    pub secrets: SecretBox,
    administrator_service: Arc<AdministratorService<SqliteAdministratorStore>>,
    administrator_origin_mode: AdministratorOriginMode,
    operations: OperationManager,
    web_directory: Option<Arc<xcss::web_assets::DirectoryAssets>>,
    client_slots: HistoryReads,
    history_reads: HistoryReads,
    task_writes: HistoryReads,
}
#[derive(Clone, Debug, Eq, PartialEq)]
struct InternalIdentity {
    subject: String,
}
impl WorkerState {
    pub fn new(
        pool: sqlx::SqlitePool,
        secrets: SecretBox,
        production: bool,
        static_dir: impl Into<Option<PathBuf>>,
    ) -> anyhow::Result<Self> {
        Ok(Self {
            administrator_service: Arc::new(AdministratorService::new(
                SqliteAdministratorStore::new(pool.clone()),
            )),
            administrator_origin_mode: if production {
                AdministratorOriginMode::ProductionHttps
            } else {
                AdministratorOriginMode::LoopbackDevelopmentHttp
            },
            operations: OperationManager::new(pool.clone(), secrets.clone()),
            pool,
            secrets,
            web_directory: static_dir
                .into()
                .map(xcss::web_assets::DirectoryAssets::new)
                .transpose()?
                .map(Arc::new),
            client_slots: HistoryReads::with_capacity(256),
            history_reads: HistoryReads::new(),
            task_writes: HistoryReads::new(),
        })
    }
    pub fn operation_manager(&self) -> &OperationManager {
        &self.operations
    }
}
pub fn router(
    state: WorkerState,
    runtime: xcss::server_runtime::RuntimeHandle,
) -> anyhow::Result<Router> {
    let protected = Router::new()
        .route("/rustdesk/devices", get(devices).post(create_device))
        .route(
            "/rustdesk/devices/{id}",
            patch(rename_device).delete(delete_device),
        )
        .route(
            "/rustdesk/devices/{id}/authorization",
            get(device_authorization).put(update_device_authorization),
        )
        .route(
            "/rustdesk/devices/{id}/pairing",
            axum::routing::delete(cancel_pairing),
        )
        .route("/rustdesk/devices/{id}/revoke", post(revoke_device))
        .route(
            "/rustdesk/devices/{id}/tasks",
            get(device_tasks).post(submit_task),
        )
        .route(
            "/rustdesk/devices/{id}/tasks/summary",
            get(device_task_summary),
        )
        .route(
            "/rustdesk/devices/{id}/tasks/calendar",
            get(device_task_calendar),
        )
        .route("/rustdesk/operations/{id}", get(operation_get))
        .route("/rustdesk/operations/{id}/resolve", post(operation_resolve))
        .layer(DefaultBodyLimit::max(MAX_MESSAGE_BYTES))
        .layer(middleware::from_fn_with_state(state.clone(), authenticate));
    let api = Router::new()
        .nest(API_VERSION_PREFIX, protected)
        .fallback(|| async { AppError::NotFound("Not Found".into()) });
    let platform = xcss::server_runtime::platform_router(
        runtime,
        "xscs",
        state.administrator_origin_mode,
        Arc::clone(&state.administrator_service),
    )?;
    let client = Router::new()
        .route(xscs_protocol::ENROLL_PATH, post(enroll))
        .route(xscs_protocol::PAIRING_PATH, post(resolve_pairing))
        .route(xscs_protocol::IDENTITY_PATH, get(client_identity))
        .route(xscs_protocol::CONNECT_PATH, get(client_connect));
    Ok(Router::new()
        .nest(API_NAMESPACE, api)
        .merge(client)
        .layer(DefaultBodyLimit::max(16 * 1024))
        .fallback(
            |State(state): State<WorkerState>, request: Request| async move {
                let path = request.uri().path();
                if path == xscs_protocol::CLIENT_NAMESPACE
                    || path
                        .strip_prefix(xscs_protocol::CLIENT_NAMESPACE)
                        .is_some_and(|suffix| suffix.starts_with('/'))
                {
                    return AppError::NotFound("Not Found".into()).into_response();
                }
                crate::web_assets::response(
                    state.web_directory.as_deref(),
                    request.uri().path(),
                    request.method(),
                    request.headers(),
                )
            },
        )
        .with_state(state)
        .merge(platform)
        .method_not_allowed_fallback(|| async { AppError::MethodNotAllowed })
        .layer(axum::middleware::from_fn(log_request))
        .layer(axum::middleware::from_fn(
            xcss::server_cli::request_context_middleware,
        )))
}
async fn authenticate(
    State(state): State<WorkerState>,
    mut request: Request,
    next: Next,
) -> Response {
    let identity = match xcss::admin_axum::authenticate_request(
        &state.administrator_service,
        request.headers(),
        request.uri(),
        request.method(),
        "xscs",
        state.administrator_origin_mode,
    )
    .await
    {
        Ok(identity) => identity,
        Err(response) => return *response,
    };
    request.extensions_mut().insert(InternalIdentity {
        subject: identity.administrator_id.to_string(),
    });
    next.run(request).await
}
async fn devices(State(state): State<WorkerState>) -> AppResult<Json<Vec<DeviceView>>> {
    Ok(Json(db::list_devices(&state.pool).await?))
}
async fn create_device(
    State(state): State<WorkerState>,
    Extension(actor): Extension<InternalIdentity>,
    ContractJson(value): ContractJson<CreateDeviceRequest>,
) -> AppResult<Response> {
    let name = value.name.as_deref().unwrap_or("新实例");
    let ticket = db::create_device(&state.pool, &state.secrets, name, &actor.subject).await?;
    Ok((
        StatusCode::CREATED,
        [("cache-control", "no-store")],
        Json(ticket),
    )
        .into_response())
}
async fn device_authorization(
    State(state): State<WorkerState>,
    ContractPath(id): ContractPath<String>,
) -> AppResult<Response> {
    Ok((
        [("cache-control", "no-store")],
        Json(ClientAuthorization {
            manager_id: db::manager_id(&state.pool).await?.to_string(),
            device_id: id.clone(),
            authorization_code: db::get_authorization(&state.pool, &state.secrets, &id).await?,
        }),
    )
        .into_response())
}
async fn update_device_authorization(
    State(state): State<WorkerState>,
    Extension(actor): Extension<InternalIdentity>,
    ContractPath(id): ContractPath<String>,
    ContractJson(value): ContractJson<UpdateClientAuthorization>,
) -> AppResult<Response> {
    Ok((
        [("cache-control", "no-store")],
        Json(ClientAuthorization {
            manager_id: db::manager_id(&state.pool).await?.to_string(),
            device_id: id.clone(),
            authorization_code: db::rotate_authorization(
                &state.pool,
                &state.secrets,
                &id,
                &value.authorization_code,
                &actor.subject,
            )
            .await?,
        }),
    )
        .into_response())
}
async fn cancel_pairing(
    State(state): State<WorkerState>,
    Extension(actor): Extension<InternalIdentity>,
    ContractPath(id): ContractPath<String>,
) -> AppResult<StatusCode> {
    db::cancel_pairing(&state.pool, &id, &actor.subject).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn rename_device(
    State(state): State<WorkerState>,
    Extension(actor): Extension<InternalIdentity>,
    ContractPath(id): ContractPath<String>,
    ContractJson(value): ContractJson<DeviceName>,
) -> AppResult<Json<DeviceView>> {
    Ok(Json(
        db::rename(&state.pool, &id, &value.name, &actor.subject).await?,
    ))
}
async fn revoke_device(
    State(state): State<WorkerState>,
    Extension(actor): Extension<InternalIdentity>,
    ContractPath(id): ContractPath<String>,
) -> AppResult<StatusCode> {
    db::revoke(&state.pool, &id, &actor.subject).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn delete_device(
    State(state): State<WorkerState>,
    Extension(actor): Extension<InternalIdentity>,
    ContractPath(id): ContractPath<String>,
) -> AppResult<StatusCode> {
    db::delete_device(&state.pool, &id, &actor.subject).await?;
    Ok(StatusCode::NO_CONTENT)
}
// Do not queue readers behind a busy device. The permits follow the database task and
// response body, so a timed-out query or a slow reader cannot allocate another page.
#[derive(Clone)]
struct HistoryReads {
    global: Arc<Semaphore>,
    devices: Arc<Mutex<HashMap<String, Weak<Semaphore>>>>,
}
struct HistoryReadPermit {
    _global: OwnedSemaphorePermit,
    _device: OwnedSemaphorePermit,
}
impl HistoryReads {
    fn new() -> Self {
        Self::with_capacity(4)
    }
    fn with_capacity(capacity: usize) -> Self {
        Self {
            global: Arc::new(Semaphore::new(capacity)),
            devices: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    fn acquire(&self, device: &str) -> AppResult<HistoryReadPermit> {
        let busy = || AppError::TooManyRequests { retry_after: 1 };
        let global = self
            .global
            .clone()
            .try_acquire_owned()
            .map_err(|_| busy())?;
        let semaphore = {
            let mut devices = self
                .devices
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            devices.retain(|_, value| value.strong_count() > 0);
            if let Some(value) = devices.get(device).and_then(Weak::upgrade) {
                value
            } else {
                let value = Arc::new(Semaphore::new(1));
                devices.insert(device.to_owned(), Arc::downgrade(&value));
                value
            }
        };
        let device = semaphore.try_acquire_owned().map_err(|_| busy())?;
        Ok(HistoryReadPermit {
            _global: global,
            _device: device,
        })
    }
}
fn task_page_response(bytes: Vec<u8>, permit: HistoryReadPermit) -> Response {
    let stream = futures_util::stream::unfold(
        (bytes, 0usize, permit),
        |(bytes, offset, permit)| async move {
            if offset == bytes.len() {
                return None;
            }
            let end = (offset + 16 * 1024).min(bytes.len());
            let chunk = Bytes::copy_from_slice(&bytes[offset..end]);
            Some((
                Ok::<_, std::convert::Infallible>(chunk),
                (bytes, end, permit),
            ))
        },
    );
    let mut response = Body::from_stream(stream).into_response();
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store, private, max-age=0"),
    );
    response
}
async fn device_tasks(
    State(state): State<WorkerState>,
    Extension(actor): Extension<InternalIdentity>,
    ContractPath(id): ContractPath<String>,
    xcss::server_cli::ContractQuery(query): xcss::server_cli::ContractQuery<TaskListQuery>,
) -> AppResult<Response> {
    query.validate()?;
    let permit = state.history_reads.acquire(&id)?;
    let task = tokio::spawn(async move {
        let bytes = if query.recent.is_some() {
            serde_json::to_vec(
                &state
                    .operations
                    .recent_for_actor(&actor.subject, &id)
                    .await?,
            )
        } else if let (Some(start), Some(end)) = (&query.start_date, &query.end_date) {
            serde_json::to_vec(
                &state
                    .operations
                    .history_range_for_actor(
                        &actor.subject,
                        &id,
                        start,
                        end,
                        query.cursor.as_deref(),
                    )
                    .await?,
            )
        } else {
            serde_json::to_vec(
                &state
                    .operations
                    .history_for_actor(
                        &actor.subject,
                        &id,
                        query.date.as_deref(),
                        query.cursor.as_deref(),
                    )
                    .await?,
            )
        }
        .map_err(|error| AppError::Internal(error.into()))?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(AppError::Internal(anyhow::anyhow!(
                "stored task page exceeds response limit"
            )));
        }
        Ok::<_, AppError>(task_page_response(bytes, permit))
    });
    timeout(Duration::from_secs(5), task)
        .await
        .map_err(|_| AppError::TooManyRequests { retry_after: 1 })?
        .map_err(|error| AppError::Internal(error.into()))?
}
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct TaskListQuery {
    recent: Option<u16>,
    date: Option<String>,
    start_date: Option<String>,
    end_date: Option<String>,
    cursor: Option<String>,
}
impl TaskListQuery {
    fn validate(&self) -> AppResult<()> {
        match (
            &self.recent,
            &self.date,
            &self.start_date,
            &self.end_date,
            &self.cursor,
        ) {
            (Some(50), None, None, None, None) => {}
            (None, Some(date), None, None, _) => {
                crate::operations::server_date_bounds(date)?;
            }
            (None, None, Some(start), Some(end), _) => {
                crate::operations::server_date_range_bounds(start, end)?;
            }
            _ => return Err(AppError::BadRequest("无效的任务日志查询参数".into())),
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct TaskCalendar {
    today: String,
}
async fn device_task_calendar() -> Json<TaskCalendar> {
    Json(TaskCalendar {
        today: Utc::now().format("%Y-%m-%d").to_string(),
    })
}
async fn device_task_summary(
    State(state): State<WorkerState>,
    ContractPath(id): ContractPath<String>,
) -> AppResult<Json<OperationSummary>> {
    Ok(Json(state.operations.summary_for_device(&id).await?))
}
async fn submit_task(
    State(state): State<WorkerState>,
    Extension(actor): Extension<InternalIdentity>,
    ContractPath(id): ContractPath<String>,
    headers: HeaderMap,
    ContractJson(command): ContractJson<Command>,
) -> AppResult<(StatusCode, Json<OperationView>)> {
    let key = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| AppError::BadRequest("缺少 Idempotency-Key".into()))?;
    crate::operations::validate_idempotency_key(key)?;
    let key = key.to_owned();
    let admission = state.task_writes.acquire(&id)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(
            run_admitted_task_write(admission, async move {
                state
                    .operations
                    .enqueue(&actor.subject, &id, &key, command)
                    .await
            })
            .await?,
        ),
    ))
}
async fn run_admitted_task_write<T: Send + 'static>(
    admission: HistoryReadPermit,
    write: impl std::future::Future<Output = AppResult<T>> + Send + 'static,
) -> AppResult<T> {
    // HTTP cancellation cannot release a write slot while its SQLite worker is
    // still executing or closing. A retry uses the same durable idempotency key.
    let task = tokio::spawn(async move {
        let _admission = admission;
        write.await
    });
    timeout(Duration::from_secs(6), task)
        .await
        .map_err(|_| AppError::TooManyRequests { retry_after: 1 })?
        .map_err(|error| AppError::Internal(error.into()))?
}
async fn operation_get(
    State(state): State<WorkerState>,
    Extension(actor): Extension<InternalIdentity>,
    ContractPath(id): ContractPath<String>,
) -> AppResult<Json<OperationView>> {
    Ok(Json(
        state.operations.get_for_actor(&actor.subject, &id).await?,
    ))
}
async fn operation_resolve(
    State(state): State<WorkerState>,
    Extension(actor): Extension<InternalIdentity>,
    ContractPath(id): ContractPath<String>,
    ContractJson(value): ContractJson<OperationResolutionRequest>,
) -> AppResult<Json<OperationView>> {
    Ok(Json(
        state
            .operations
            .resolve_for_actor(&actor.subject, &id, value.resolution)
            .await?,
    ))
}

// TLS is provided by the deployment entry point. Device requests use their own
// credentials and do not share the browser session channel.
fn require_client_channel(headers: &HeaderMap) -> AppResult<()> {
    if headers.contains_key("origin") || headers.contains_key("cookie") {
        return Err(AppError::Forbidden(
            "Client channel does not accept browser Origin or cookies".into(),
        ));
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PairingRequest {
    authorization_code: String,
}
async fn resolve_pairing(
    State(state): State<WorkerState>,
    headers: HeaderMap,
    ContractJson(value): ContractJson<PairingRequest>,
) -> AppResult<Response> {
    require_client_channel(&headers)?;
    Ok((
        [("cache-control", "no-store")],
        Json(db::resolve_pairing(&state.pool, &value.authorization_code).await?),
    )
        .into_response())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EnrollmentRequest {
    device_id: uuid::Uuid,
    installation_id: uuid::Uuid,
    token: String,
    credential: String,
}
async fn enroll(
    State(state): State<WorkerState>,
    headers: HeaderMap,
    ContractJson(value): ContractJson<EnrollmentRequest>,
) -> AppResult<Response> {
    require_client_channel(&headers)?;
    let binding = db::enroll(
        &state.pool,
        &value.device_id.to_string(),
        value.installation_id,
        &value.token,
        &value.credential,
    )
    .await?;
    Ok(([("cache-control", "no-store")], Json(binding)).into_response())
}
async fn client_identity(
    State(state): State<WorkerState>,
    headers: HeaderMap,
) -> AppResult<Response> {
    require_client_channel(&headers)?;
    let credential = headers
        .get("authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "))
        .ok_or(AppError::Unauthorized)?;
    let device = db::authenticate_device(&state.pool, credential).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(db::binding(&state.pool, &device).await?),
    )
        .into_response())
}
async fn client_connect(
    State(state): State<WorkerState>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> AppResult<Response> {
    require_client_channel(&headers)?;
    let credential = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(AppError::Unauthorized)?;
    let device = db::authenticate_device(&state.pool, credential).await?;
    let authenticated_credential_hash = device
        .credential_hash
        .clone()
        .ok_or(AppError::Unauthorized)?;
    let binding = db::binding(&state.pool, &device).await?;
    if headers
        .get("sec-websocket-protocol")
        .and_then(|v| v.to_str().ok())
        != Some(WEBSOCKET_SUBPROTOCOL)
    {
        return Err(AppError::BadRequest("Client 子协议不支持".into()));
    }
    let permit = state.client_slots.acquire(&binding.device_id.to_string())?;
    let request_id = headers
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    Ok(upgrade
        .protocols([WEBSOCKET_SUBPROTOCOL])
        .max_message_size(MAX_MESSAGE_BYTES)
        .max_frame_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| async move {
            let _permit = permit;
            let mut pending = None;
            let session = uuid::Uuid::new_v4().to_string();
            tracing::info!(event = "xscs.instance.connected", instance_id = %binding.device_id, instance_type = "rustdesk", task_id = session.as_str(), request_id = request_id.as_str());
            if let Err(error) = serve_client(&state, socket, &binding, &authenticated_credential_hash, &session, &mut pending).await {
                tracing::warn!(event = "xscs.instance.connection_failed", instance_id = %binding.device_id, instance_type = "rustdesk", task_id = session.as_str(), request_id = request_id.as_str(), error = %error, "client session ended with an error");
            }
            if let Some(operation) = pending {
                let _ = state.operations.disconnected(&operation).await;
            }
            let _ = sqlx::query(
                "UPDATE devices SET session_id=NULL WHERE device_id=? AND session_id=?",
            )
            .bind(binding.device_id.to_string())
            .bind(&session)
            .execute(&state.pool)
            .await;
            tracing::info!(event = "xscs.instance.disconnected", instance_id = %binding.device_id, instance_type = "rustdesk", task_id = session.as_str(), request_id = request_id.as_str());
        }))
}
async fn receive(socket: &mut WebSocket) -> AppResult<ClientMessage> {
    let frame = timeout(Duration::from_secs(10), socket.recv())
        .await
        .map_err(|_| AppError::Unauthorized)?
        .ok_or(AppError::Unauthorized)?
        .map_err(|_| AppError::Unauthorized)?;
    let Message::Text(text) = frame else {
        return Err(AppError::Unauthorized);
    };
    serde_json::from_str(&text).map_err(|_| AppError::BadRequest("Client 消息无效".into()))
}
async fn send(socket: &mut WebSocket, message: ManagerMessage) -> AppResult<()> {
    let text = serde_json::to_string(&message)
        .map_err(|_| AppError::BadRequest("Client 指令无效".into()))?;
    timeout(
        Duration::from_secs(10),
        socket.send(Message::Text(text.into())),
    )
    .await
    .map_err(|_| AppError::Unauthorized)?
    .map_err(|_| AppError::Unauthorized)
}
fn result_completes_pending(
    pending_operation: Option<&str>,
    awaiting_inspection_start: bool,
    result_operation: &str,
) -> bool {
    !awaiting_inspection_start && pending_operation == Some(result_operation)
}

async fn serve_client(
    state: &WorkerState,
    mut socket: WebSocket,
    binding: &Binding,
    authenticated_credential_hash: &[u8],
    session: &str,
    pending: &mut Option<xcss::operations::StoredOperation>,
) -> AppResult<()> {
    let ClientMessage::Hello {
        binding: hello,
        capabilities,
        active_operation,
    } = receive(&mut socket).await?
    else {
        return Err(AppError::Unauthorized);
    };
    if &hello != binding || capabilities.validate().is_err() {
        return Err(AppError::Unauthorized);
    }
    let id = binding.device_id.to_string();
    let changed=sqlx::query("UPDATE devices SET session_id=?,last_seen_at_micros=?,capabilities_json=? WHERE device_id=? AND installation_id=? AND revoked_at_micros IS NULL AND credential_hash=?")
        .bind(session).bind(db::now_micros()?).bind(serde_json::to_string(&capabilities).map_err(|_|AppError::Unauthorized)?).bind(&id).bind(binding.installation_id.to_string()).bind(authenticated_credential_hash).execute(&state.pool).await?.rows_affected();
    if changed != 1 {
        return Err(AppError::Unauthorized);
    }
    if let Some(active) = active_operation {
        let operation = state
            .operations
            .receipt_operation(&active.operation_id, binding)
            .await?;
        if state
            .operations
            .task(&operation)?
            .fingerprint()
            .map_err(|_| AppError::Unauthorized)?
            != active.fingerprint
        {
            return Err(AppError::Unauthorized);
        }
        // Fence the old connection before adopting its evidence-only continuation.
        if operation.operation.state == xcss::operations::OperationState::Running
            && operation.operation.lease_owner.as_deref() != Some(session)
        {
            state.operations.disconnected(&operation).await?;
        }
        *pending = Some(
            state
                .operations
                .receipt_operation(&active.operation_id, binding)
                .await?,
        );
    }
    let wake = state.operations.wake_for_device(&id);
    let mut poll = tokio::time::interval(Duration::from_secs(5));
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_seen = Instant::now();
    let mut dispatched = Instant::now();
    let mut inspected = HashSet::new();
    // Hello-adopted work is already active. A newly dispatched inspection must
    // first cross its Progress boundary on the ordered Client socket.
    let mut awaiting_inspection_start = false;
    loop {
        tokio::select! {
            message=socket.recv()=>{
                let frame=message.ok_or(AppError::Unauthorized)?.map_err(|_|AppError::Unauthorized)?;
                let current=db::get_device(&state.pool,&id).await?;
                if current.revoked_at_micros.is_some()||current.session_id.as_deref()!=Some(session){let _=send(&mut socket,ManagerMessage::Revoked{}).await;return Err(AppError::Unauthorized);}
                last_seen=Instant::now();
                sqlx::query("UPDATE devices SET last_seen_at_micros=? WHERE device_id=? AND session_id=?").bind(db::now_micros()?).bind(&id).bind(session).execute(&state.pool).await?;
                match frame{
                    Message::Ping(bytes)=>{timeout(Duration::from_secs(10),socket.send(Message::Pong(bytes))).await.map_err(|_|AppError::Unauthorized)?.map_err(|_|AppError::Unauthorized)?;}
                    Message::Pong(_)=>{}
                    Message::Text(text)=>{
                        let message:ClientMessage=serde_json::from_str(&text).map_err(|_|AppError::BadRequest("Client 消息无效".into()))?;
                        match message{
                            ClientMessage::Heartbeat{diagnostics}=>{
                                if let Some(snapshot)=&diagnostics{crate::operations::validate_diagnostics(snapshot)?;}
                                sqlx::query("UPDATE devices SET health_at_micros=?,diagnostics_json=COALESCE(?,diagnostics_json) WHERE device_id=? AND session_id=? AND revoked_at_micros IS NULL")
                                    .bind(db::now_micros()?).bind(diagnostics.as_ref().map(serde_json::to_string).transpose().map_err(|_|AppError::Unauthorized)?)
                                    .bind(&id).bind(session).execute(&state.pool).await?;
                            }
                            ClientMessage::Progress{operation_id,fingerprint}=>{
                                let operation=state.operations.receipt_operation(&operation_id,binding).await?;
                                if state.operations.task(&operation)?.fingerprint().map_err(|_|AppError::Unauthorized)?.as_str()!=fingerprint{return Err(AppError::Unauthorized);}
                                if pending.as_ref().is_none_or(|op|op.operation.operation_id==operation_id){
                                    if pending.is_none(){dispatched=Instant::now();*pending=Some(operation);}
                                    awaiting_inspection_start=false;
                                }else{return Err(AppError::BadRequest("执行进度不匹配".into()));}
                                // Liveness evidence does not extend the fixed 90s execution budget/120s lease.
                            }
                            ClientMessage::Result{operation_id,report}=>{
                                let operation = if let Some(operation)=pending.as_ref().filter(|op|op.operation.operation_id==operation_id){operation.clone()}else{state.operations.receipt_operation(&operation_id,binding).await?};
                                let fingerprint=state.operations.task(&operation)?.fingerprint().map_err(|_|AppError::Unauthorized)?;
                                let report_digest=xscs_protocol::report_digest(&report).map_err(|_|AppError::Unauthorized)?;
                                state.operations.accept_replayed_result(&operation,session,report).await?;
                                if result_completes_pending(pending.as_ref().map(|op|op.operation.operation_id.as_str()), awaiting_inspection_start, &operation_id){*pending=None;wake.notify_one();}
                                // Only acknowledge after the transaction has durably committed.
                                let accepted=state.operations.receipt_operation(&operation_id,binding).await?;
                                let finalized=!matches!(accepted.operation.state,xcss::operations::OperationState::Pending|xcss::operations::OperationState::Running|xcss::operations::OperationState::Unknown);
                                send(&mut socket,ManagerMessage::ResultAccepted{operation_id,fingerprint,report_digest,finalized}).await?;
                            }
                            _=>return Err(AppError::BadRequest("重复 Client Hello".into())),
                        }
                    }
                    _=>return Err(AppError::Unauthorized),
                }
            }
            _=async {tokio::select!{_=poll.tick()=>{},_=wake.notified()=>{}}}=>{
                if last_seen.elapsed()>Duration::from_secs(45)||(pending.is_some()&&dispatched.elapsed()>Duration::from_secs(110)){return Err(AppError::Unauthorized);}
                let current=db::get_device(&state.pool,&id).await?;
                if current.revoked_at_micros.is_some()||current.session_id.as_deref()!=Some(session){let _=send(&mut socket,ManagerMessage::Revoked{}).await;return Err(AppError::Unauthorized);}
                if pending.is_none(){
                    let (operation,mode)=if let Some(unknown)=state.operations.uncertain(&id, &binding.installation_id).await?{
                        if inspected.insert(unknown.operation.operation_id.clone()) {
                            (Some(unknown),DeliveryMode::InspectOnly)
                        } else {
                            (state.operations.next(&id,session).await?,DeliveryMode::Execute)
                        }
                    }else{(state.operations.next(&id,session).await?,DeliveryMode::Execute)};
                    if let Some(operation)=operation{
                        let task=state.operations.task(&operation)?;
                        *pending=Some(operation);dispatched=Instant::now();
                        awaiting_inspection_start=mode==DeliveryMode::InspectOnly;
                        let (expires_at_unix_ms,remaining_ms)=state.operations.execution_budget(pending.as_ref().expect("stored dispatch"))?;
                        send(&mut socket,ManagerMessage::Task{mode,task:Box::new(task),expires_at_unix_ms,remaining_ms}).await?;
                    }
                }
            }
        }
    }
}

async fn log_request(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use tracing::Instrument;
    let request_id = request
        .extensions()
        .get::<xcss::contracts::RequestId>()
        .map(|value| value.as_str().to_owned())
        .unwrap_or_default();
    let span = tracing::info_span!("http.request", request_id = request_id.as_str());
    async move {
        let started = std::time::Instant::now();
        let response = next.run(request).await;
        tracing::info!(
            event = "xscs.http.completed",
            component = "http",
            status = response.status().as_u16() as u64,
            duration_ms = started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64
        );
        response
    }
    .instrument(span)
    .await
}

#[cfg(test)]
mod tests {
    #[test]
    fn old_result_cannot_release_a_newly_dispatched_inspection() {
        assert!(!super::result_completes_pending(
            Some("inspect"),
            true,
            "inspect"
        ));
        assert!(!super::result_completes_pending(
            Some("inspect"),
            true,
            "other"
        ));
        assert!(super::result_completes_pending(
            Some("inspect"),
            false,
            "inspect"
        ));
        assert!(!super::result_completes_pending(
            Some("inspect"),
            false,
            "other"
        ));
        assert!(!super::result_completes_pending(None, false, "inspect"));
    }

    #[test]
    fn hello_adopted_execution_can_complete_without_a_new_inspection_start() {
        assert!(super::result_completes_pending(
            Some("adopted"),
            false,
            "adopted"
        ));
    }

    #[test]
    fn log_range_query_validates_before_history_admission() {
        use serde_json::json;
        for value in [
            json!({}),
            json!({"start_date":"2022-02-01"}),
            json!({"end_date":"2023-02-02"}),
            json!({"date":"2022-02-01","start_date":"2022-02-01","end_date":"2023-02-02"}),
            json!({"recent":50,"start_date":"2022-02-01","end_date":"2023-02-02"}),
            json!({"start_date":"2023-02-02","end_date":"2022-02-01"}),
            json!({"start_date":"2022-02-29","end_date":"2023-02-02"}),
            json!({"start_date":"0000-01-01","end_date":"2023-02-02"}),
        ] {
            let query: super::TaskListQuery = serde_json::from_value(value).unwrap();
            assert!(query.validate().is_err());
        }
        for value in [
            json!({"date":"2024-02-29"}),
            json!({"start_date":"2022-02-01","end_date":"2023-02-02"}),
            json!({"recent":50}),
        ] {
            let query: super::TaskListQuery = serde_json::from_value(value).unwrap();
            assert!(query.validate().is_ok());
        }
    }

    use super::*;
    use axum::body::Body;
    use axum::http::Method;
    use axum::http::{HeaderValue, header};
    use http_body_util::BodyExt;
    use serde_json::Value;
    use sqlx::sqlite::SqlitePoolOptions;
    use tower::ServiceExt;
    use xcss::error::ErrorEnvelope;

    #[tokio::test]
    async fn cancelled_http_write_retains_its_instance_slot_until_the_worker_finishes() {
        let writes = HistoryReads::new();
        let (started, running) = tokio::sync::oneshot::channel();
        let (complete, done) = tokio::sync::oneshot::channel();
        let caller = tokio::spawn(run_admitted_task_write(
            writes.acquire("cancelled-device").unwrap(),
            async move {
                started.send(()).unwrap();
                done.await.unwrap();
                Ok(())
            },
        ));
        running.await.unwrap();
        caller.abort();
        assert!(caller.await.unwrap_err().is_cancelled());
        assert!(matches!(
            writes.acquire("cancelled-device"),
            Err(AppError::TooManyRequests { .. })
        ));
        let unaffected = writes.acquire("unaffected-device").unwrap();
        complete.send(()).unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if writes.acquire("cancelled-device").is_ok() {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        drop(unaffected);
    }

    #[tokio::test]
    async fn historical_read_admission_is_per_device_global_and_held_by_the_response_body() {
        let reads = HistoryReads::new();
        let response = task_page_response(
            vec![b' '; 32 * 1024],
            reads.acquire("noisy-device").unwrap(),
        );
        assert!(matches!(
            reads.acquire("noisy-device"),
            Err(AppError::TooManyRequests { .. })
        ));
        let second = reads.acquire("second-device").unwrap();
        let third = reads.acquire("third-device").unwrap();
        let fourth = reads.acquire("fourth-device").unwrap();
        assert!(matches!(
            reads.acquire("fifth-device"),
            Err(AppError::TooManyRequests { .. })
        ));
        assert_eq!(
            response
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .len(),
            32 * 1024
        );
        let resumed = reads.acquire("noisy-device").unwrap();
        drop((second, third, fourth, resumed));
        let permit = reads.acquire("different-device").unwrap();
        assert_eq!(reads.devices.lock().unwrap().len(), 1);
        let response = task_page_response(vec![0; 32 * 1024], permit);
        drop(response);
        assert!(reads.acquire("different-device").is_ok());
    }

    #[test]
    fn websocket_admission_rejects_duplicate_devices_without_using_another_instances_slot() {
        let slots = HistoryReads::with_capacity(256);
        let first = slots.acquire("first-device").unwrap();
        assert!(matches!(
            slots.acquire("first-device"),
            Err(AppError::TooManyRequests { .. })
        ));
        let other = slots.acquire("other-device").unwrap();
        assert_eq!(slots.global.available_permits(), 254);
        drop(first);
        assert!(slots.acquire("first-device").is_ok());
        drop(other);
    }

    fn test_static_dir() -> Option<PathBuf> {
        None
    }

    fn test_runtime() -> xcss::server_runtime::RuntimeHandle {
        xcss::server_runtime::platform_handle(xcss::server_runtime::ProductDescriptor {
            id: "xscs".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            common_revision: env!("XCSS_REVISION").to_owned(),
            profile: "server-control-plane".to_owned(),
            capabilities: vec!["embedded-web".into(), "server-runtime".to_owned()],
        })
        .unwrap()
    }

    async fn test_router() -> Router {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db::initialize_empty(&pool).await.unwrap();
        let state = WorkerState::new(
            pool,
            SecretBox::new("test", [2; 32]).unwrap(),
            false,
            test_static_dir(),
        )
        .unwrap();
        router(state, test_runtime()).unwrap()
    }

    #[tokio::test]
    async fn browser_routes_serve_the_compiled_client_and_exact_font_bytes() {
        let app = test_router().await;
        for path in [
            "index.html",
            crate::web_assets::ASSETS
                .iter()
                .find(|asset| asset.path.ends_with(".woff2"))
                .unwrap()
                .path,
        ] {
            let expected = crate::web_assets::ASSETS
                .iter()
                .find(|asset| asset.path == path)
                .unwrap();
            let response = app
                .clone()
                .oneshot(
                    Request::get(format!("/{path}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(
                response.headers()[header::CONTENT_TYPE],
                expected.content_type
            );
            assert_eq!(
                response.headers()[header::X_CONTENT_TYPE_OPTIONS],
                "nosniff"
            );
            assert_eq!(
                response
                    .into_body()
                    .collect()
                    .await
                    .unwrap()
                    .to_bytes()
                    .as_ref(),
                expected.bytes
            );
        }
        let missing = app
            .oneshot(
                Request::get("/assets/not-an-asset.js")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn health_is_public() {
        let live = test_router()
            .await
            .oneshot(Request::get("/healthz").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(live.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn private_route_requires_a_session_cookie() {
        for path in [
            "/api/v1/rustdesk/devices",
            "/api/v1/rustdesk/devices/unknown/tasks/calendar",
            "/api/v1/rustdesk/devices/unknown/tasks/summary",
        ] {
            let private = test_router()
                .await
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(private.status(), StatusCode::UNAUTHORIZED, "{path}");
        }
    }

    #[test]
    fn device_channel_keeps_browser_requests_separate() {
        assert!(require_client_channel(&HeaderMap::new()).is_ok());
        for (name, value) in [
            (header::ORIGIN, "https://rustdesk.example.com"),
            (header::COOKIE, "browser-session=fixture"),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(name, HeaderValue::from_static(value));
            assert!(require_client_channel(&headers).is_err());
        }
    }

    #[tokio::test]
    async fn device_lifecycle_accepts_a_remote_backend_peer_without_proxy_headers() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db::initialize_empty(&pool).await.unwrap();
        let secrets = SecretBox::new("test", [2; 32]).unwrap();
        let ticket = db::create_device(&pool, &secrets, "remote backend", "admin")
            .await
            .unwrap();
        let state = WorkerState::new(pool, secrets, true, None).unwrap();
        let application = router(state, test_runtime()).unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, application).await.unwrap() });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let pairing = client
            .post(format!("{base}{}", xscs_protocol::PAIRING_PATH))
            .json(&serde_json::json!({"authorization_code": ticket.token}))
            .send()
            .await
            .unwrap();
        assert_eq!(pairing.status(), StatusCode::OK);
        assert_eq!(
            pairing.json::<Value>().await.unwrap()["device_id"],
            ticket.device.id
        );

        let installation = uuid::Uuid::new_v4();
        let credential = db::random_token();
        let enrollment = client
            .post(format!("{base}{}", xscs_protocol::ENROLL_PATH))
            .json(&serde_json::json!({
                "device_id": ticket.device.id,
                "installation_id": installation,
                "token": ticket.token,
                "credential": credential,
            }))
            .send()
            .await
            .unwrap();
        assert_eq!(enrollment.status(), StatusCode::OK);
        let binding = enrollment.json::<Value>().await.unwrap();
        assert_eq!(binding["device_id"], ticket.device.id);
        assert_eq!(binding["installation_id"], installation.to_string());

        let identity = client
            .get(format!("{base}{}", xscs_protocol::IDENTITY_PATH))
            .bearer_auth(&credential)
            .send()
            .await
            .unwrap();
        assert_eq!(identity.status(), StatusCode::OK);
        assert_eq!(identity.json::<Value>().await.unwrap(), binding);

        let connection = client
            .get(format!("{base}{}", xscs_protocol::CONNECT_PATH))
            .bearer_auth(&credential)
            .header(header::CONNECTION, "Upgrade")
            .header(header::UPGRADE, "websocket")
            .header("sec-websocket-version", "13")
            .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
            .header("sec-websocket-protocol", WEBSOCKET_SUBPROTOCOL)
            .send()
            .await
            .unwrap();
        assert_eq!(connection.status(), StatusCode::SWITCHING_PROTOCOLS);
        assert_eq!(
            connection.headers()["sec-websocket-protocol"],
            WEBSOCKET_SUBPROTOCOL
        );
        drop(connection);
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn client_websocket_authentication_has_a_distinct_manager_marker() {
        let application = test_router().await;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, application).await.unwrap() });
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let unknown_credential = format!("Bearer {}", "a".repeat(64));
        for (authorization, forwarded, expected, terminal) in [
            (None, Some("https"), StatusCode::UNAUTHORIZED, false),
            (
                Some("Bearer malformed"),
                Some("https"),
                StatusCode::UNAUTHORIZED,
                false,
            ),
            (
                Some(unknown_credential.as_str()),
                Some("https"),
                StatusCode::UNAUTHORIZED,
                true,
            ),
            (None, None, StatusCode::UNAUTHORIZED, false),
        ] {
            let mut request = client
                .get(format!("http://{address}{}", xscs_protocol::CONNECT_PATH))
                .header(header::CONNECTION, "Upgrade")
                .header(header::UPGRADE, "websocket")
                .header("sec-websocket-version", "13")
                .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
                .header("sec-websocket-protocol", WEBSOCKET_SUBPROTOCOL);
            if let Some(value) = forwarded {
                request = request.header("x-forwarded-proto", value);
            }
            if let Some(value) = authorization {
                request = request.header(header::AUTHORIZATION, value);
            }
            let response = request.send().await.unwrap();
            assert_eq!(response.status(), expected);
            assert_eq!(response.headers().contains_key("x-error-code"), terminal);
            if terminal {
                assert_eq!(response.headers()["x-error-code"], "unauthorized");
            }
            if expected == StatusCode::UNAUTHORIZED {
                let bytes = response.bytes().await.unwrap();
                let envelope: ErrorEnvelope = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(envelope.code.as_str(), "unauthorized");
            }
        }
        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn auth_route_is_public() {
        let response = test_router()
            .await
            .oneshot(
                Request::post("/api/v1/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::HOST, "localhost")
                    .header(header::ORIGIN, "http://localhost")
                    .header(xcss::admin_auth::SEC_FETCH_SITE_HEADER, "same-origin")
                    .body(Body::from(
                        r#"{"username":"admin","password":"bad-password"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn login_uses_one_unambiguous_host_or_uri_authority() {
        let application = test_router().await;
        let body = r#"{"username":"admin","password":"bad-password"}"#;

        let authority_only = Request::post("http://localhost/api/v1/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ORIGIN, "http://localhost")
            .header(xcss::admin_auth::SEC_FETCH_SITE_HEADER, "same-origin")
            .body(Body::from(body))
            .unwrap();
        assert_eq!(
            application
                .clone()
                .oneshot(authority_only)
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );

        let ambiguous = Request::post("http://localhost/api/v1/auth/login")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::HOST, "localhost")
            .header(header::ORIGIN, "http://localhost")
            .header(xcss::admin_auth::SEC_FETCH_SITE_HEADER, "same-origin")
            .body(Body::from(body))
            .unwrap();
        assert_eq!(
            application.oneshot(ambiguous).await.unwrap().status(),
            StatusCode::FORBIDDEN
        );
    }

    #[tokio::test]
    async fn login_policy_violations_share_the_credentials_failure() {
        for body in [
            r#"{"username":"admin@example.com","password":"correct-password"}"#,
            r#"{"username":"admin","password":"too-short"}"#,
        ] {
            let response = test_router()
                .await
                .oneshot(
                    Request::post("/api/v1/auth/login")
                        .header(header::CONTENT_TYPE, "application/json")
                        .header(header::HOST, "localhost")
                        .header(header::ORIGIN, "http://localhost")
                        .header(xcss::admin_auth::SEC_FETCH_SITE_HEADER, "same-origin")
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
    }

    #[tokio::test]
    async fn email_field_is_rejected_instead_of_becoming_a_login_alias() {
        let response = test_router()
            .await
            .oneshot(
                Request::post("/api/v1/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::HOST, "localhost")
                    .header(header::ORIGIN, "http://localhost")
                    .header(xcss::admin_auth::SEC_FETCH_SITE_HEADER, "same-origin")
                    .body(Body::from(
                        r#"{"email":"admin","password":"correct-password"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn old_and_unversioned_api_paths_are_not_routes() {
        for (method, path) in [
            (Method::POST, "/api/v0/auth/login"),
            (Method::POST, "/xscc/v0/pairing"),
            (Method::GET, "/xscc/v0/connect"),
            (Method::POST, "/xscc/v3/enroll"),
            (Method::POST, "/xscc/v3/pairing"),
            (Method::GET, "/xscc/v3/identity"),
            (Method::GET, "/xscc/v3/connect"),
            (Method::POST, "/api/v1/rustdesk/operations/removed/retry"),
            (Method::GET, "/api/services/rustdesk/hosts"),
            (Method::GET, "/api/rustdesk/hosts"),
        ] {
            let response = test_router()
                .await
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(path)
                        .header(header::CONTENT_TYPE, "application/json")
                        .body(Body::from(r#"{"username":"admin","password":"password"}"#))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::NOT_FOUND, "{path}");
        }
    }

    #[tokio::test]
    async fn login_body_is_bounded_before_password_work() {
        let oversized = serde_json::json!({
            "username": "admin",
            "password": "x".repeat(20 * 1024)
        })
        .to_string();
        let response = test_router()
            .await
            .oneshot(
                Request::post("/api/v1/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::HOST, "localhost")
                    .header(header::ORIGIN, "http://localhost")
                    .header(xcss::admin_auth::SEC_FETCH_SITE_HEADER, "same-origin")
                    .body(Body::from(oversized))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn database_session_requires_csrf_and_logout_revokes_it() {
        // Exercise the same file-backed history connection lifecycle as production.
        let directory = tempfile::tempdir().unwrap();
        let pool = db::open_or_initialize(&format!(
            "sqlite://{}",
            directory.path().join("state.db").display()
        ))
        .await
        .unwrap();
        crate::test_support::seed_administrator(
            &AdministratorService::new(SqliteAdministratorStore::new(pool.clone())),
            "admin",
            "correct horse battery staple",
            u64::try_from(db::now_micros().unwrap()).unwrap(),
        )
        .await
        .unwrap();
        let state = WorkerState::new(
            pool.clone(),
            SecretBox::new("test", [2; 32]).unwrap(),
            false,
            test_static_dir(),
        )
        .unwrap();
        let application = router(state, test_runtime()).unwrap();

        let login = application
            .clone()
            .oneshot(
                Request::post("/api/v1/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .header(header::HOST, "localhost")
                    .header(header::ORIGIN, "http://localhost")
                    .header(xcss::admin_auth::SEC_FETCH_SITE_HEADER, "same-origin")
                    .body(Body::from(
                        r#"{"username":" Admin ","password":"correct horse battery staple"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(login.status(), StatusCode::OK);
        assert_eq!(
            login.headers()[header::CACHE_CONTROL],
            "no-store, private, max-age=0"
        );
        let cookie = login
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .map(|value| value.to_str().unwrap().split(';').next().unwrap())
            .collect::<Vec<_>>()
            .join("; ");
        assert!(cookie.contains("admin-xscs-session="));
        assert!(!cookie.contains("rustdesk_csrf="));
        let login_body: Value =
            serde_json::from_slice(&login.into_body().collect().await.unwrap().to_bytes()).unwrap();
        let mut session_keys = login_body
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        session_keys.sort_unstable();
        assert_eq!(
            session_keys,
            ["authenticated", "csrf_token", "role", "user_id", "username"]
        );
        assert_eq!(login_body["authenticated"], true);
        assert_eq!(login_body["username"], "admin");
        assert_eq!(login_body["role"], "admin");
        let csrf = login_body["csrf_token"].as_str().unwrap().to_string();
        assert!(!csrf.is_empty());
        let stored_hash: Vec<u8> =
            sqlx::query_scalar("SELECT token_hash FROM _common_admin_sessions")
                .fetch_one(&pool)
                .await
                .unwrap();
        let session_token = cookie
            .split("; ")
            .find_map(|value| value.strip_prefix("admin-xscs-session="))
            .unwrap();
        assert_eq!(stored_hash.len(), 32);
        assert_eq!(
            xcss::admin_auth::token_hash(session_token).as_slice(),
            stored_hash
        );
        assert_ne!(session_token.as_bytes(), stored_hash);

        for (name, value) in [
            (xcss::admin_auth::ORIGIN_HEADER, "http://localhost"),
            (xcss::admin_auth::HOST_HEADER, "localhost"),
            (xcss::admin_auth::SEC_FETCH_SITE_HEADER, "same-origin"),
            (xcss::admin_auth::CSRF_HEADER, csrf.as_str()),
        ] {
            let mut request = Request::post("/api/v1/auth/logout")
                .header(header::COOKIE, &cookie)
                .header(xcss::admin_auth::CSRF_HEADER, &csrf)
                .header(header::HOST, "localhost")
                .header(header::ORIGIN, "http://localhost")
                .header(xcss::admin_auth::SEC_FETCH_SITE_HEADER, "same-origin")
                .body(Body::empty())
                .unwrap();
            request.headers_mut().append(
                axum::http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                HeaderValue::from_str(value).unwrap(),
            );
            let response = application.clone().oneshot(request).await.unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{name}");
        }

        let current = application
            .clone()
            .oneshot(
                Request::get("/api/v1/auth/session")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(current.status(), StatusCode::OK);
        let current_body: Value =
            serde_json::from_slice(&current.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let refreshed_csrf = current_body["csrf_token"].as_str().unwrap().to_string();
        assert_eq!(refreshed_csrf, csrf);

        let before_calendar = Utc::now().format("%Y-%m-%d").to_string();
        let calendar = application
            .clone()
            .oneshot(
                Request::get("/api/v1/rustdesk/devices/unknown/tasks/calendar")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(calendar.status(), StatusCode::OK);
        let calendar: Value =
            serde_json::from_slice(&calendar.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        let today = calendar["today"].as_str().unwrap();
        let after_calendar = Utc::now().format("%Y-%m-%d").to_string();
        assert!(today == before_calendar || today == after_calendar);
        for (query, expected) in [
            (format!("?date={today}"), StatusCode::OK),
            ("?recent=50".to_string(), StatusCode::OK),
            ("".to_string(), StatusCode::BAD_REQUEST),
            ("?date=2027-02-30".to_string(), StatusCode::BAD_REQUEST),
            (
                "?date=2027-01-15&recent=50".to_string(),
                StatusCode::BAD_REQUEST,
            ),
        ] {
            let response = application
                .clone()
                .oneshot(
                    Request::get(format!("/api/v1/rustdesk/devices/unknown/tasks{query}"))
                        .header(header::COOKIE, &cookie)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), expected, "{query}");
        }
        let summary = application
            .clone()
            .oneshot(
                Request::get("/api/v1/rustdesk/devices/unknown/tasks/summary")
                    .header(header::COOKIE, &cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(summary.status(), StatusCode::OK);
        let summary: Value =
            serde_json::from_slice(&summary.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(summary["blocking_count"], 0);

        let missing_csrf = application
            .clone()
            .oneshot(
                Request::post("/api/v1/auth/logout")
                    .header(header::COOKIE, &cookie)
                    .header(header::HOST, "localhost")
                    .header(header::ORIGIN, "http://localhost")
                    .header(xcss::admin_auth::SEC_FETCH_SITE_HEADER, "same-origin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing_csrf.status(), StatusCode::FORBIDDEN);
        let request_id = missing_csrf
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let envelope: ErrorEnvelope =
            serde_json::from_slice(&missing_csrf.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(envelope.code.as_str(), "auth.csrf_rejected");
        assert!(!envelope.retryable);
        if let Some(request_id) = request_id {
            assert_eq!(
                envelope.request_id.as_ref().map(|value| value.as_str()),
                Some(request_id.as_str())
            );
        }
        assert!(envelope.details.is_empty());

        let missing_origin = application
            .clone()
            .oneshot(
                Request::post("/api/v1/auth/logout")
                    .header(header::COOKIE, &cookie)
                    .header("x-csrf-token", &refreshed_csrf)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing_origin.status(), StatusCode::FORBIDDEN);

        let logout = application
            .clone()
            .oneshot(
                Request::post("/api/v1/auth/logout")
                    .header(header::COOKIE, &cookie)
                    .header("x-csrf-token", &refreshed_csrf)
                    .header(header::HOST, "localhost")
                    .header(header::ORIGIN, "http://localhost")
                    .header(xcss::admin_auth::SEC_FETCH_SITE_HEADER, "same-origin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(logout.status(), StatusCode::NO_CONTENT);

        let revoked = application
            .oneshot(
                Request::get("/api/v1/auth/session")
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(revoked.status(), StatusCode::UNAUTHORIZED);
        let envelope: ErrorEnvelope =
            serde_json::from_slice(&revoked.into_body().collect().await.unwrap().to_bytes())
                .unwrap();
        assert_eq!(envelope.code.as_str(), "auth.session_required");
        assert!(!envelope.retryable);
    }
}
