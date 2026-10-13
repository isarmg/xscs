//! Durable lifecycle belongs to xcss; this module only dispatches product commands to devices.
use crate::{
    crypto::{SecretBox, constant_time_equal_32},
    db,
    error::{AppError, AppResult},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Connection, FromRow};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
    time::Duration,
};
use tokio::sync::{Notify, watch};
use uuid::Uuid;
pub use xcss::operations::OperationState;
use xcss::operations::{
    EnqueueOutcome, NewOperation, SqliteOperationStore, StoredOperation, Transition,
};
#[cfg(test)]
use xscs_protocol::PROTOCOL;
use xscs_protocol::{
    Binding, Capabilities, Command, ConfigSnapshot, MAX_REPORT_BYTES, Permission, Report,
    TASK_PROTOCOL, Task,
};

const RESTART_EXECUTION_WINDOW_MICROS: i64 = 15 * 60 * 1_000_000;

#[derive(Clone)]
pub struct OperationManager {
    pub pool: sqlx::SqlitePool,
    pub store: SqliteOperationStore,
    secrets: SecretBox,
    notifications: Arc<Mutex<HashMap<String, Weak<Notify>>>>,
    history_capacity: HistoryCapacity,
}
#[derive(Clone, Copy)]
struct HistoryCapacity {
    device_rows: i64,
    total_rows: i64,
    database_bytes: u64,
    device_bytes: u64,
    free_floor: u64,
}
impl HistoryCapacity {
    const fn production() -> Self {
        Self {
            device_rows: 100_000,
            total_rows: 1_000_000,
            database_bytes: crate::database_schema::DATABASE_BYTE_BUDGET,
            device_bytes: 1024 * 1024 * 1024,
            free_floor: 1024 * 1024 * 1024,
        }
    }
}
const FUTURE_AUDIT_RESERVE: u64 = 16 * 1024;
const NEW_TASK_METADATA_RESERVE: u64 = 32 * 1024;

async fn check_history_capacity(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    value: &NewOperation,
    limits: HistoryCapacity,
) -> AppResult<()> {
    let existing: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM _common_operations WHERE namespace=? AND idempotency_digest=?)",
    )
    .bind(&value.namespace)
    .bind(value.idempotency_digest.as_slice())
    .fetch_one(&mut **tx)
    .await?;
    // xcss still validates the fingerprint of a retry, including at full capacity.
    if existing {
        return Ok(());
    }
    let (total, device, missing_reports, device_missing, device_payload): (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT COUNT(*),COALESCE(SUM(op.target_key=?),0), \
         COALESCE(SUM((op.result_payload IS NULL)+(obs.operation_id IS NULL)),0), \
         COALESCE(SUM(CASE WHEN op.target_key=? THEN (op.result_payload IS NULL)+(obs.operation_id IS NULL) ELSE 0 END),0), \
         COALESCE(SUM(CASE WHEN op.target_key=? THEN length(op.request_payload)+COALESCE(length(op.result_payload),0)+COALESCE(length(CAST(obs.report_json AS BLOB)),0)+16384 ELSE 0 END),0) \
         FROM _common_operations op LEFT JOIN client_observations obs ON obs.operation_id=op.operation_id")
        .bind(&value.target_key).bind(&value.target_key).bind(&value.target_key).fetch_one(&mut **tx).await?;
    if total >= limits.total_rows || device >= limits.device_rows {
        return Err(AppError::HistoryCapacity);
    }
    let pages: i64 = sqlx::query_scalar("PRAGMA page_count")
        .fetch_one(&mut **tx)
        .await?;
    let page_size: i64 = sqlx::query_scalar("PRAGMA page_size")
        .fetch_one(&mut **tx)
        .await?;
    let allocated = (pages.max(0) as u64).saturating_mul(page_size.max(0) as u64);
    let path: String =
        sqlx::query_scalar("SELECT file FROM pragma_database_list WHERE name='main'")
            .fetch_one(&mut **tx)
            .await?;
    let (wal_bytes, available) = if path.is_empty() {
        (0, u64::MAX) // Internal in-memory fixtures have no filesystem allocation.
    } else {
        tokio::task::spawn_blocking(move || -> AppResult<(u64, u64)> {
            let main = std::path::Path::new(&path);
            let wal = std::path::PathBuf::from(format!("{path}-wal"));
            let wal_bytes = match std::fs::symlink_metadata(&wal) {
                Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
                    metadata.len()
                }
                Ok(_) => {
                    return Err(internal(anyhow::anyhow!(
                        "invalid database journal identity"
                    )));
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => 0,
                Err(error) => return Err(internal(error)),
            };
            let stats = rustix::fs::statvfs(
                main.parent()
                    .ok_or_else(|| internal(anyhow::anyhow!("database path has no parent")))?,
            )
            .map_err(internal)?;
            Ok((wal_bytes, stats.f_bavail.saturating_mul(stats.f_frsize)))
        })
        .await
        .map_err(internal)??
    };
    // Each absent report may still need its full protocol allowance plus a stored configuration snapshot.
    // Charge twice for main database plus WAL, and reserve future state/audit pages for
    // every retained operation. This deliberately keeps uncertain and resolved facts.
    let existing_reserve = (missing_reports.max(0) as u64)
        .saturating_mul(2 * MAX_REPORT_BYTES as u64)
        .saturating_add((total.max(0) as u64).saturating_mul(FUTURE_AUDIT_RESERVE))
        .saturating_mul(2);
    let new_reserve = (value.request_payload.len() as u64)
        .saturating_add(4 * MAX_REPORT_BYTES as u64)
        .saturating_add(FUTURE_AUDIT_RESERVE)
        .saturating_add(NEW_TASK_METADATA_RESERVE)
        .saturating_mul(2);
    let device_reserve = (device_missing.max(0) as u64)
        .saturating_mul(2 * MAX_REPORT_BYTES as u64)
        .saturating_add((device.max(0) as u64).saturating_mul(FUTURE_AUDIT_RESERVE));
    let device_charge = (device_payload.max(0) as u64)
        .saturating_add(device_reserve)
        .saturating_mul(2)
        .saturating_add(new_reserve)
        .saturating_add(2 * MAX_REPORT_BYTES as u64);
    if device_charge > limits.device_bytes {
        return Err(AppError::HistoryCapacity);
    }
    let reserve = existing_reserve.saturating_add(new_reserve);
    if allocated.saturating_add(wal_bytes).saturating_add(reserve) > limits.database_bytes
        || available < limits.free_floor.saturating_add(reserve)
    {
        return Err(AppError::HistoryCapacity);
    }
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Payload {
    actor: String,
    request_ciphertext: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    binding: Binding,
    command: Command,
}
#[derive(Serialize)]
pub struct OperationView {
    pub operation_id: String,
    pub device_id: String,
    pub action: String,
    pub state: OperationState,
    pub attempt: i64,
    pub created_at_micros: i64,
    pub created_at_server: String,
    pub updated_at_micros: i64,
    pub result: Option<Report>,
    pub reconciliation: Option<Report>,
    /// Receipt time of the reconciliation evidence, unaffected by human resolution.
    pub reconciliation_observed_at_micros: Option<i64>,
    pub resolution: Option<String>,
    pub status_reason: Option<&'static str>,
}

pub const HISTORY_PAGE_SIZE: usize = 50;
const MAX_HISTORY_CURSOR_BYTES: usize = 512;

#[derive(Serialize)]
pub struct OperationPage {
    pub operations: Vec<OperationView>,
    pub next_cursor: Option<String>,
    pub previous_cursor: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct HistoryCursor {
    version: u8,
    newer: bool,
    created_at_micros: i64,
    operation_id: String,
    scope: String,
}
impl HistoryCursor {
    fn decode(value: &str, expected_scope: &str) -> AppResult<Self> {
        let invalid = || AppError::BadRequest("无效的任务日志游标".into());
        if value.is_empty() || value.len() > MAX_HISTORY_CURSOR_BYTES || !value.is_ascii() {
            return Err(invalid());
        }
        let bytes = URL_SAFE_NO_PAD.decode(value).map_err(|_| invalid())?;
        let cursor: Self = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if cursor.version != 1
            || cursor.scope != expected_scope
            || cursor.created_at_micros < 0
            || cursor.operation_id.is_empty()
            || cursor.operation_id.len() > 128
            || !cursor
                .operation_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(invalid());
        }
        Ok(cursor)
    }
    fn encode(row: &OperationView, newer: bool, scope: &str) -> AppResult<String> {
        let value = Self {
            version: 1,
            newer,
            created_at_micros: row.created_at_micros,
            operation_id: row.operation_id.clone(),
            scope: scope.to_owned(),
        };
        let encoded = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&value).map_err(internal)?);
        if encoded.len() > MAX_HISTORY_CURSOR_BYTES {
            return Err(internal(anyhow::anyhow!(
                "invalid stored operation identifier"
            )));
        }
        Ok(encoded)
    }
}
fn history_scope(actor: &str, device: &str, range: Option<(i64, i64)>) -> String {
    // Scope is a navigation constraint. SQL still independently checks actor and device.
    let value = serde_json::to_vec(&(actor, device, range))
        .expect("history scope consists of JSON primitives");
    hex::encode(Sha256::digest(value))
}
fn bounded_report(bytes: Option<&[u8]>) -> AppResult<Option<Report>> {
    if bytes.is_some_and(|value| value.len() > MAX_REPORT_BYTES) {
        return Err(internal(anyhow::anyhow!(
            "stored task report exceeds the protocol limit"
        )));
    }
    bytes
        .map(serde_json::from_slice)
        .transpose()
        .map_err(internal)
}
fn operation_list_view(row: OperationListRow) -> AppResult<OperationView> {
    if row.oversized_text {
        return Err(internal(anyhow::anyhow!(
            "stored task text exceeds the history field limit"
        )));
    }
    Ok(OperationView {
        operation_id: row.operation_id,
        device_id: row.target_key,
        action: row.action,
        state: OperationState::parse(&row.state).map_err(internal)?,
        attempt: row.attempt,
        created_at_micros: row.created_at_micros,
        created_at_server: server_timestamp(row.created_at_micros)?,
        updated_at_micros: row.updated_at_micros,
        result: bounded_report(row.result_payload.as_deref())?,
        reconciliation: bounded_report(row.report_json.as_deref())?,
        reconciliation_observed_at_micros: row.reconciliation_observed_at_micros,
        resolution: row.resolution_code,
        status_reason: public_status_reason(row.error_code.as_deref()),
    })
}

fn public_status_reason(code: Option<&str>) -> Option<&'static str> {
    // Only product-owned, administrator-facing reasons cross the API boundary.
    match code {
        Some("authorization_rotated") => Some("authorization_rotated"),
        Some("device_revoked") => Some("device_revoked"),
        Some("execution_deadline_expired") => Some("execution_deadline_expired"),
        _ => None,
    }
}

#[derive(Serialize)]
pub struct OperationSummary {
    pub blocking_count: i64,
}

fn server_timestamp(micros: i64) -> AppResult<String> {
    let utc = DateTime::<Utc>::from_timestamp_micros(micros)
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("invalid task creation timestamp")))?;
    Ok(utc.format("%Y-%m-%d %H:%M:%S%.6f UTC").to_string())
}

pub fn server_date_bounds(date: &str) -> AppResult<(i64, i64)> {
    let bytes = date.as_bytes();
    if bytes.len() != 10
        || date.starts_with("0000-")
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| index != 4 && index != 7 && !byte.is_ascii_digit())
    {
        return Err(AppError::BadRequest("无效的服务器日期".into()));
    }
    let day = NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .map_err(|_| AppError::BadRequest("无效的服务器日期".into()))?;
    let next = day
        .succ_opt()
        .ok_or_else(|| AppError::BadRequest("无效的服务器日期".into()))?;
    fn start(day: NaiveDate) -> AppResult<i64> {
        let midnight = day
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| AppError::BadRequest("无效的服务器日期".into()))?;
        Ok(midnight.and_utc().timestamp_micros())
    }
    Ok((start(day)?, start(next)?))
}

pub fn server_date_range_bounds(start: &str, end: &str) -> AppResult<(i64, i64)> {
    let (from, _) = server_date_bounds(start)?;
    let (_, to) = server_date_bounds(end)?;
    if start > end {
        return Err(AppError::BadRequest("结束日期不能早于开始日期".into()));
    }
    Ok((from, to))
}

#[derive(FromRow)]
struct OperationListRow {
    operation_id: String,
    target_key: String,
    action: String,
    state: String,
    attempt: i64,
    created_at_micros: i64,
    updated_at_micros: i64,
    result_payload: Option<Vec<u8>>,
    report_json: Option<Vec<u8>>,
    reconciliation_observed_at_micros: Option<i64>,
    resolution_code: Option<String>,
    error_code: Option<String>,
    oversized_text: bool,
}
async fn record_configuration(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    device: &str,
    report: &Report,
) -> AppResult<()> {
    match report {
        Report::ConfigRead { snapshot } => {
            sqlx::query("UPDATE devices SET snapshot_json=?,configuration_state=CASE WHEN configuration_state='awaiting_restart' THEN configuration_state ELSE 'pending_verification' END WHERE device_id=?")
                    .bind(serde_json::to_string(snapshot).map_err(internal)?)
                    .bind(device)
                    .execute(&mut **tx)
                    .await?;
        }
        Report::ConfigSaved { snapshot } | Report::RestartAcknowledged { snapshot, .. } => {
            let state = if matches!(report, Report::ConfigSaved { .. }) {
                "awaiting_restart"
            } else {
                "pending_verification"
            };
            sqlx::query("UPDATE devices SET snapshot_json=?,saved_revision=?,configuration_state=? WHERE device_id=?").bind(serde_json::to_string(snapshot).map_err(internal)?).bind(&snapshot.revision).bind(state).bind(device).execute(&mut **tx).await?;
        }
        _ => {}
    }
    Ok(())
}

fn namespace(device_id: &str, installation_id: &Uuid, resource: &str) -> String {
    format!("xscc.client.v1.{device_id}.{installation_id}.{resource}")
}
fn resource(command: &Command) -> &'static str {
    if matches!(
        command,
        Command::ReadConfig {}
            | Command::ListApplications {}
            | Command::ListPendingPairings {}
            | Command::ListPairedClients {}
            | Command::ReadLogs { .. }
            | Command::ReadDiagnostics {}
            | Command::ReadVirtualInputStatus {}
            | Command::ReadServiceStatus {}
    ) {
        "read"
    } else {
        "write"
    }
}
pub fn action(command: &Command) -> &'static str {
    match command {
        Command::ReadConfig {} => "sunshine.config.read",
        Command::PatchConfig { .. } => "sunshine.config.patch",
        Command::SaveConfig { .. } => "sunshine.config.save",
        Command::Restart { .. } => "sunshine.restart",
        Command::ListApplications {} => "sunshine.applications.list",
        Command::SaveApplication { .. } => "sunshine.applications.save",
        Command::DeleteApplication { .. } => "sunshine.applications.delete",
        Command::CloseApplication { .. } => "sunshine.applications.close",
        Command::UploadCover { .. } => "sunshine.applications.cover.upload",
        Command::SubmitPairingPin { .. } => "sunshine.pairing.pin.submit",
        Command::ListPendingPairings {} => "sunshine.pairing.pending.list",
        Command::ListPairedClients {} => "sunshine.pairing.clients.list",
        Command::SetPairedClientEnabled { .. } => "sunshine.pairing.clients.update",
        Command::UnpairClient { .. } => "sunshine.pairing.clients.unpair",
        Command::UnpairAllClients { .. } => "sunshine.pairing.clients.unpair_all",
        Command::ReadLogs { .. } => "sunshine.logs.read",
        Command::ReadDiagnostics {} => "sunshine.diagnostics.read",
        Command::ReadVirtualInputStatus {} => "sunshine.virtual_input.read",
        Command::RunMaintenance { .. } => "sunshine.maintenance.run",
        Command::ReadServiceStatus {} => "sunshine.service.read",
        Command::ControlService { .. } => "sunshine.service.control",
    }
}
fn permission(command: &Command) -> Permission {
    match command {
        Command::ReadConfig {} => Permission::ReadConfig,
        Command::PatchConfig { .. } | Command::SaveConfig { .. } => Permission::WriteConfig,
        Command::Restart { .. } => Permission::Restart,
        Command::ListApplications {} => Permission::ReadApplications,
        Command::SaveApplication { .. }
        | Command::DeleteApplication { .. }
        | Command::CloseApplication { .. }
        | Command::UploadCover { .. } => Permission::ManageApplications,
        Command::SubmitPairingPin { .. }
        | Command::ListPendingPairings {}
        | Command::ListPairedClients {}
        | Command::SetPairedClientEnabled { .. }
        | Command::UnpairClient { .. }
        | Command::UnpairAllClients { .. } => Permission::ManagePairing,
        Command::ReadLogs { .. }
        | Command::ReadDiagnostics {}
        | Command::ReadVirtualInputStatus {} => Permission::ReadDiagnostics,
        Command::RunMaintenance { .. } => Permission::MaintainSunshine,
        Command::ReadServiceStatus {} | Command::ControlService { .. } => {
            Permission::ControlService
        }
    }
}
fn internal(error: impl Into<anyhow::Error>) -> AppError {
    AppError::Internal(error.into())
}
fn payload(stored: &StoredOperation) -> AppResult<Payload> {
    serde_json::from_slice(&stored.request_payload).map_err(internal)
}
pub fn decode_task(secrets: &SecretBox, stored: &StoredOperation) -> AppResult<Task> {
    let payload = payload(stored)?;
    let plaintext = secrets.decrypt_operation_request(
        &stored.operation.operation_id,
        &stored.action,
        &payload.request_ciphertext,
    )?;
    if !constant_time_equal_32(
        &stored.operation.request_fingerprint,
        &secrets.operation_request_fingerprint(&plaintext),
    ) {
        return Err(AppError::Crypto);
    }
    let request: Request = serde_json::from_str(&plaintext).map_err(internal)?;
    let task = Task {
        protocol: TASK_PROTOCOL.into(),
        operation_id: stored.operation.operation_id.clone(),
        permission: permission(&request.command),
        binding: request.binding,
        command: request.command,
    };
    if action(&task.command) != stored.action
        || task.binding.device_id.to_string() != stored.operation.target_key
        || stored.operation.namespace
            != namespace(
                &stored.operation.target_key,
                &task.binding.installation_id,
                resource(&task.command),
            )
    {
        return Err(AppError::Crypto);
    }
    Ok(task)
}
pub fn validate_idempotency_key(key: &str) -> AppResult<()> {
    if key.is_empty()
        || key.len() > 128
        || !key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    {
        Err(AppError::BadRequest(
            "必须提供 1–128 字符的 Idempotency-Key".into(),
        ))
    } else {
        Ok(())
    }
}
impl OperationManager {
    pub fn new(pool: sqlx::SqlitePool, secrets: SecretBox) -> Self {
        Self {
            store: SqliteOperationStore::new(pool.clone()),
            pool,
            secrets,
            notifications: Arc::new(Mutex::new(HashMap::new())),
            history_capacity: HistoryCapacity::production(),
        }
    }
    pub fn wake_for_device(&self, device: &str) -> Arc<Notify> {
        let mut notifications = self
            .notifications
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        notifications.retain(|_, value| value.strong_count() > 0);
        if let Some(value) = notifications.get(device).and_then(Weak::upgrade) {
            return value;
        }
        let value = Arc::new(Notify::new());
        notifications.insert(device.to_owned(), Arc::downgrade(&value));
        value
    }
    pub fn execution_budget(&self, stored: &StoredOperation) -> AppResult<(u64, u64)> {
        let deadline = stored
            .created_at_micros
            .saturating_add(RESTART_EXECUTION_WINDOW_MICROS);
        let remaining = deadline.saturating_sub(db::now_micros()?).max(0);
        Ok(((deadline.max(0) / 1000) as u64, (remaining / 1000) as u64))
    }
    pub async fn receipt_operation(
        &self,
        id: &str,
        binding: &Binding,
    ) -> AppResult<StoredOperation> {
        let stored = self
            .store
            .get(id)
            .await
            .map_err(internal)?
            .ok_or_else(|| AppError::BadRequest("结果任务不存在".into()))?;
        if self.task(&stored)?.binding != *binding {
            return Err(AppError::Unauthorized);
        }
        Ok(stored)
    }
    pub async fn accept_replayed_result(
        &self,
        stored: &StoredOperation,
        session: &str,
        report: Report,
    ) -> AppResult<()> {
        let mut stored = stored.clone();
        if stored.operation.state == OperationState::Running
            && stored.operation.lease_owner.as_deref() != Some(session)
        {
            self.disconnected(&stored).await?;
            stored = self
                .store
                .get(&stored.operation.operation_id)
                .await
                .map_err(internal)?
                .ok_or(AppError::Unauthorized)?;
        }
        if stored.operation.state == OperationState::Pending {
            return Err(AppError::BadRequest("任务尚未投递".into()));
        }
        validate_report(&self.task(&stored)?.command, &report)?;
        if matches!(
            stored.operation.state,
            OperationState::Running | OperationState::Unknown
        ) {
            return self.complete(&stored, session, report).await;
        }
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let valid: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM devices WHERE device_id=? AND session_id=? AND revoked_at_micros IS NULL)")
            .bind(&stored.operation.target_key).bind(session).fetch_one(&mut *tx).await?;
        if !valid {
            return Err(AppError::Unauthorized);
        }
        if let Some(bytes) = &stored.result_payload {
            let prior: Report = serde_json::from_slice(bytes).map_err(internal)?;
            let observed: Option<String> = sqlx::query_scalar(
                "SELECT report_json FROM client_observations WHERE operation_id=?",
            )
            .bind(&stored.operation.operation_id)
            .fetch_optional(&mut *tx)
            .await?;
            let observed_matches = observed
                .as_deref()
                .and_then(|text| serde_json::from_str::<Report>(text).ok())
                .as_ref()
                == Some(&report);
            if prior != report && !observed_matches {
                let previous_observation = observed
                    .as_deref()
                    .map(serde_json::from_str::<Report>)
                    .transpose()
                    .map_err(internal)?;
                if stored.operation.state == OperationState::Resolved
                    && matches!(prior, Report::Unknown { .. })
                    && previous_observation
                        .as_ref()
                        .is_none_or(|previous| matches!(previous, Report::Unknown { .. }))
                {
                    // Preserve the human final outcome; a late inspection only supplies evidence.
                    sqlx::query("INSERT INTO client_observations(operation_id,report_json,observed_at_micros) VALUES(?,?,?) ON CONFLICT(operation_id) DO UPDATE SET report_json=excluded.report_json,observed_at_micros=excluded.observed_at_micros")
                        .bind(&stored.operation.operation_id).bind(serde_json::to_string(&report).map_err(internal)?).bind(db::now_micros()?).execute(&mut *tx).await?;
                } else {
                    return Err(AppError::Conflict("最终结果不可改写".into()));
                }
            }
        } else {
            // A cancelled read or manually resolved unknown may retain evidence, never change outcome.
            sqlx::query("INSERT INTO client_observations(operation_id,report_json,observed_at_micros) VALUES(?,?,?) ON CONFLICT(operation_id) DO NOTHING")
                .bind(&stored.operation.operation_id).bind(serde_json::to_string(&report).map_err(internal)?).bind(db::now_micros()?).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn enqueue(
        &self,
        actor: &str,
        id: &str,
        key: &str,
        command: Command,
    ) -> AppResult<OperationView> {
        validate_idempotency_key(key)?;
        let mut connection = tokio::time::timeout(Duration::from_secs(2), self.pool.acquire())
            .await
            .map_err(|_| AppError::TooManyRequests { retry_after: 1 })??;
        connection.close_on_drop();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        connection
            .lock_handle()
            .await?
            .set_progress_handler(1_000, move || std::time::Instant::now() < deadline);
        let result = self
            .enqueue_in(&mut connection, actor, id, key, command)
            .await;
        // Await SQLite's worker close before releasing admission, including a rollback.
        connection.close().await?;
        let stored = result?;
        self.wake_for_device(id).notify_waiters();
        self.view(stored).await
    }
    async fn enqueue_in(
        &self,
        connection: &mut sqlx::SqliteConnection,
        actor: &str,
        id: &str,
        key: &str,
        command: Command,
    ) -> AppResult<StoredOperation> {
        let mut tx = connection.begin_with("BEGIN IMMEDIATE").await?;
        let device: db::Device = sqlx::query_as("SELECT * FROM devices WHERE device_id=?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| AppError::NotFound("设备不存在".into()))?;
        if device.revoked_at_micros.is_some() {
            return Err(AppError::Conflict("设备凭据已撤销".into()));
        }
        // Resolve the identity in the same transaction to fence enrollment/revocation.
        let manager: String =
            sqlx::query_scalar("SELECT manager_id FROM manager_identity WHERE singleton=1")
                .fetch_one(&mut *tx)
                .await?;
        let binding = Binding {
            manager_id: Uuid::parse_str(&manager).map_err(internal)?,
            device_id: Uuid::parse_str(id).map_err(internal)?,
            installation_id: device
                .installation_id
                .as_deref()
                .and_then(|id| Uuid::parse_str(id).ok())
                .ok_or_else(|| AppError::Conflict("请先注册 Client".into()))?,
        };
        let capabilities: Capabilities = device
            .capabilities_json
            .as_deref()
            .ok_or_else(|| AppError::Conflict("Client 尚未上报管理能力".into()))
            .and_then(|value| serde_json::from_str(value).map_err(internal))?;
        let operation_id = format!("op_{}", Uuid::new_v4());
        let task = Task {
            protocol: TASK_PROTOCOL.into(),
            operation_id: operation_id.clone(),
            binding: binding.clone(),
            permission: permission(&command),
            command: command.clone(),
        };
        task.validate(&binding, &capabilities)
            .map_err(|_| AppError::BadRequest("业务指令不符合白名单、权限或修订要求".into()))?;
        let plaintext = serde_json::to_string(&Request {
            binding: binding.clone(),
            command,
        })
        .map_err(internal)?;
        let mut digest = Sha256::new();
        for part in [
            actor.as_bytes(),
            id.as_bytes(),
            action(&task.command).as_bytes(),
            self.secrets.operation_idempotency_key_hash(key).as_slice(),
        ] {
            digest.update((part.len() as u64).to_be_bytes());
            digest.update(part);
        }
        let payload = Payload {
            actor: actor.into(),
            request_ciphertext: self.secrets.encrypt_operation_request(
                &operation_id,
                action(&task.command),
                &plaintext,
            )?,
        };
        let now = db::now_micros()?;
        let value = NewOperation {
            operation_id,
            namespace: namespace(id, &binding.installation_id, resource(&task.command)),
            target_key: id.into(),
            action: action(&task.command).into(),
            idempotency_digest: digest.finalize().into(),
            request_fingerprint: self.secrets.operation_request_fingerprint(&plaintext),
            request_payload: serde_json::to_vec(&payload).map_err(internal)?,
            max_attempts: 1,
            not_before_micros: now,
            created_at_micros: now,
        };
        if let Err(error) = check_history_capacity(&mut tx, &value, self.history_capacity).await {
            if matches!(error, AppError::HistoryCapacity) {
                tracing::warn!(event = "xscs.instance.task_capacity_exhausted", instance_id = %id, instance_type = "sunshine", error_code = "history_capacity_exhausted", "Task not accepted: history capacity exhausted");
            }
            return Err(error);
        }
        let stored = SqliteOperationStore::enqueue_in(&mut tx, value)
            .await
            .map_err(|error| match error {
                xcss::operations::Error::IdempotencyConflict => {
                    AppError::Conflict("幂等键已用于不同指令".into())
                }
                other => internal(other),
            })?;
        if resource(&task.command) == "write" && matches!(&stored, EnqueueOutcome::Created(_)) {
            let uncertain: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM _common_operations WHERE target_key=? AND namespace LIKE '%.write' AND state='unknown')")
                .bind(id).fetch_one(&mut *tx).await?;
            if uncertain {
                return Err(AppError::Conflict(
                    "请先核对未确认的写入任务；读取与诊断仍可使用".into(),
                ));
            }
        }
        tx.commit().await?;
        let stored = match stored {
            EnqueueOutcome::Created(stored) | EnqueueOutcome::Existing(stored) => stored,
        };
        Ok(stored)
    }
    async fn view(&self, stored: StoredOperation) -> AppResult<OperationView> {
        let reconciliation: Option<(String, i64)> = sqlx::query_as(
            "SELECT report_json,observed_at_micros FROM client_observations WHERE operation_id=?",
        )
        .bind(&stored.operation.operation_id)
        .fetch_optional(&self.pool)
        .await?;
        Ok(OperationView {
            operation_id: stored.operation.operation_id,
            device_id: stored.operation.target_key,
            action: stored.action,
            state: stored.operation.state,
            attempt: i64::from(stored.operation.attempt),
            created_at_micros: stored.created_at_micros,
            created_at_server: server_timestamp(stored.created_at_micros)?,
            updated_at_micros: stored.updated_at_micros,
            result: stored
                .result_payload
                .as_deref()
                .map(serde_json::from_slice)
                .transpose()
                .map_err(internal)?,
            reconciliation: reconciliation
                .as_ref()
                .map(|(report, _)| serde_json::from_str(report))
                .transpose()
                .map_err(internal)?,
            reconciliation_observed_at_micros: reconciliation.map(|(_, at)| at),
            resolution: stored.resolution_code,
            // xcss error codes may contain internal diagnostics. Expose
            // only reasons with an explicit administrator-facing meaning.
            status_reason: public_status_reason(stored.operation.error_code.as_deref()),
        })
    }
    pub async fn get_for_actor(&self, actor: &str, id: &str) -> AppResult<OperationView> {
        let stored = self
            .store
            .get(id)
            .await
            .map_err(internal)?
            .ok_or_else(|| AppError::NotFound("任务不存在".into()))?;
        if payload(&stored)?.actor != actor {
            return Err(AppError::Forbidden("不能访问其他管理员的任务".into()));
        }
        self.view(stored).await
    }
    /// Complete history is traversed with bounded keyset pages; rows are never discarded.
    pub async fn history_for_actor(
        &self,
        actor: &str,
        device: &str,
        date: Option<&str>,
        cursor: Option<&str>,
    ) -> AppResult<OperationPage> {
        let range = date.map(server_date_bounds).transpose()?;
        self.history_for_actor_bounds(actor, device, range, cursor)
            .await
    }

    pub async fn history_range_for_actor(
        &self,
        actor: &str,
        device: &str,
        start: &str,
        end: &str,
        cursor: Option<&str>,
    ) -> AppResult<OperationPage> {
        let range = server_date_range_bounds(start, end)?;
        self.history_for_actor_bounds(actor, device, Some(range), cursor)
            .await
    }

    async fn history_for_actor_bounds(
        &self,
        actor: &str,
        device: &str,
        range: Option<(i64, i64)>,
        cursor: Option<&str>,
    ) -> AppResult<OperationPage> {
        let scope = history_scope(actor, device, range);
        let cursor = cursor
            .map(|value| HistoryCursor::decode(value, &scope))
            .transpose()?;
        let newer = cursor.as_ref().is_some_and(|value| value.newer);
        let projection = "SELECT \
             CASE WHEN length(CAST(op.operation_id AS BLOB))>128 THEN '' ELSE op.operation_id END AS operation_id, \
             CASE WHEN length(CAST(op.target_key AS BLOB))>128 THEN '' ELSE op.target_key END AS target_key, \
             CASE WHEN length(CAST(op.action AS BLOB))>128 THEN '' ELSE op.action END AS action, \
             CASE WHEN length(CAST(op.state AS BLOB))>128 THEN '' ELSE op.state END AS state,op.attempt, \
             op.created_at_micros,op.updated_at_micros, \
             obs.observed_at_micros AS reconciliation_observed_at_micros, \
             substr(CAST(op.result_payload AS BLOB),1,?) AS result_payload, \
             substr(CAST(obs.report_json AS BLOB),1,?) AS report_json, \
             CASE WHEN length(CAST(op.resolution_code AS BLOB))>128 THEN '' ELSE op.resolution_code END AS resolution_code, \
             CASE WHEN length(CAST(op.error_code AS BLOB))>128 THEN '' ELSE op.error_code END AS error_code, \
             (length(CAST(op.operation_id AS BLOB))>128 OR length(CAST(op.target_key AS BLOB))>128 \
              OR length(CAST(op.action AS BLOB))>128 OR length(CAST(op.state AS BLOB))>128 \
              OR COALESCE(length(CAST(op.resolution_code AS BLOB)),0)>128 \
              OR COALESCE(length(CAST(op.error_code AS BLOB)),0)>128) AS oversized_text \
             FROM _common_operations AS op \
             LEFT JOIN client_observations AS obs ON obs.operation_id=op.operation_id";
        let time_filter = if range.is_some() {
            "AND op.created_at_micros>=? AND op.created_at_micros<?"
        } else {
            ""
        };
        let anchor_filter = match cursor.as_ref() {
            Some(_) if newer => "AND (op.created_at_micros,op.operation_id)>(?,?)",
            Some(_) => "AND (op.created_at_micros,op.operation_id)<(?,?)",
            None => "",
        };
        let order = if newer { "ASC" } else { "DESC" };
        let sql = format!(
            "{projection} WHERE op.target_key=? \
            AND json_extract(CAST(substr(op.request_payload,1,131073) AS TEXT),'$.actor')=? \
            {time_filter} {anchor_filter} ORDER BY op.created_at_micros {order},operation_id {order} LIMIT ?"
        );
        let mut query = sqlx::query_as::<_, OperationListRow>(sqlx::AssertSqlSafe(sql))
            .bind((MAX_REPORT_BYTES + 1) as i64)
            .bind((MAX_REPORT_BYTES + 1) as i64)
            .bind(device)
            .bind(actor);
        if let Some((from, to)) = range {
            query = query.bind(from).bind(to);
        }
        if let Some(value) = &cursor {
            query = query
                .bind(value.created_at_micros)
                .bind(&value.operation_id);
        }
        let mut connection = tokio::time::timeout(Duration::from_secs(2), self.pool.acquire())
            .await
            .map_err(|_| AppError::TooManyRequests { retry_after: 1 })??;
        // A cancelled caller must never return a connection with this private progress
        // callback to the pool. SQLite closes it through its own VFS and worker.
        connection.close_on_drop();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        connection
            .lock_handle()
            .await?
            .set_progress_handler(1_000, move || std::time::Instant::now() < deadline);
        let result = query
            .bind((HISTORY_PAGE_SIZE + 1) as i64)
            .fetch_all(&mut *connection)
            .await;
        connection.close().await?;
        let mut rows = result?;
        let more = rows.len() > HISTORY_PAGE_SIZE;
        rows.truncate(HISTORY_PAGE_SIZE);
        if newer {
            rows.reverse();
        }
        let operations = rows
            .into_iter()
            .map(operation_list_view)
            .collect::<AppResult<Vec<_>>>()?;
        let previous_cursor = if (newer && more) || (!newer && cursor.is_some()) {
            operations
                .first()
                .map(|row| HistoryCursor::encode(row, true, &scope))
                .transpose()?
        } else {
            None
        };
        let next_cursor = if (!newer && more) || newer {
            operations
                .last()
                .map(|row| HistoryCursor::encode(row, false, &scope))
                .transpose()?
        } else {
            None
        };
        Ok(OperationPage {
            operations,
            next_cursor,
            previous_cursor,
        })
    }

    pub async fn recent_for_actor(
        &self,
        actor: &str,
        device: &str,
    ) -> AppResult<Vec<OperationView>> {
        Ok(self
            .history_for_actor(actor, device, None, None)
            .await?
            .operations)
    }

    pub async fn summary_for_device(&self, device: &str) -> AppResult<OperationSummary> {
        let blocking_count = sqlx::query_scalar(
            "SELECT COUNT(*) FROM _common_operations WHERE target_key=? \
             AND namespace LIKE '%.write' AND state IN ('pending','running','unknown')",
        )
        .bind(device)
        .fetch_one(&self.pool)
        .await?;
        Ok(OperationSummary { blocking_count })
    }
    pub async fn resolve_for_actor(
        &self,
        actor: &str,
        id: &str,
        resolution: xcss::operations::Resolution,
    ) -> AppResult<OperationView> {
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let bytes: Vec<u8> = sqlx::query_scalar(
            "SELECT request_payload FROM _common_operations WHERE operation_id=?",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::NotFound("任务不存在".into()))?;
        if serde_json::from_slice::<Payload>(&bytes)
            .map_err(internal)?
            .actor
            != actor
        {
            return Err(AppError::Forbidden("不能处理其他管理员的任务".into()));
        }
        let updated = SqliteOperationStore::resolve_in(&mut tx, id, resolution, db::now_micros()?)
            .await
            .map_err(|_| AppError::Conflict("任务不需要人工核对".into()))?;
        db::audit(
            &mut tx,
            "operation.resolve",
            &updated.operation.target_key,
            actor,
            Some(&format!(
                "operation_id={id} resolution={}",
                resolution.as_str()
            )),
        )
        .await?;
        tx.commit().await?;
        self.view(updated).await
    }
    // Reads have no uncertain side effects to reconcile. The shared recovery API
    // first fences interrupted leases as unknown; close only the read lane with
    // its explicit unable-to-confirm outcome, preserving reports and audit history.
    async fn release_uncertain_reads(&self) -> AppResult<()> {
        let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM _common_operations WHERE namespace LIKE 'xscc.client.v1.%.read' AND state='unknown')")
            .fetch_one(&self.pool).await?;
        if !pending {
            return Ok(());
        }
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT operation_id FROM _common_operations WHERE namespace LIKE 'xscc.client.v1.%.read' AND state='unknown'",
        )
        .fetch_all(&mut *tx)
        .await?;
        for id in ids {
            SqliteOperationStore::resolve_in(
                &mut tx,
                &id,
                xcss::operations::Resolution::UnableToConfirm,
                db::now_micros()?,
            )
            .await
            .map_err(internal)?;
        }
        tx.commit().await?;
        Ok(())
    }
    pub async fn recover_startup(&self) -> AppResult<u64> {
        sqlx::query("UPDATE devices SET session_id=NULL,last_seen_at_micros=NULL,health_at_micros=NULL,sunshine_reachable=NULL").execute(&self.pool).await?;
        let spaces: Vec<String> =
            sqlx::query_scalar("SELECT DISTINCT namespace FROM _common_operations")
                .fetch_all(&self.pool)
                .await?;
        let mut count = 0;
        for space in spaces {
            count += self
                .store
                .recover_running(&space, "manager_interrupted", db::now_micros()?)
                .await
                .map_err(internal)?;
        }
        self.release_uncertain_reads().await?;
        Ok(count)
    }
    pub async fn next(&self, device: &str, owner: &str) -> AppResult<Option<StoredOperation>> {
        loop {
            let now = db::now_micros()?;
            // Avoid a SQLite write lock for idle devices. Authorization is rechecked in the claim.
            let eligible: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM _common_operations WHERE target_key=? AND state='pending' AND not_before_micros<=?)")
                .bind(device).bind(now).fetch_one(&self.pool).await?;
            if !eligible {
                return Ok(None);
            }
            // Linearize authorization and claim with revocation/session replacement. A task
            // already claimed before revocation can be in flight; revocation cannot undo it.
            let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
            let installation: String = sqlx::query_scalar("SELECT installation_id FROM devices WHERE device_id=? AND session_id=? AND revoked_at_micros IS NULL AND credential_hash IS NOT NULL")
                .bind(device).bind(owner).fetch_optional(&mut *tx).await?.ok_or(AppError::Unauthorized)?;
            let installation = Uuid::parse_str(&installation).map_err(internal)?;
            let already_running: bool = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM _common_operations WHERE target_key=? AND state='running')",
            )
            .bind(device)
            .fetch_one(&mut *tx)
            .await?;
            if already_running {
                tx.commit().await?;
                return Ok(None);
            }
            // Oldest eligible task wins. An unresolved write blocks later writes, not reads.
            let candidate: Option<String> = sqlx::query_scalar("SELECT namespace FROM _common_operations WHERE target_key=? AND state='pending' AND not_before_micros<=? AND namespace IN (?,?) AND (namespace LIKE '%.read' OR NOT EXISTS(SELECT 1 FROM _common_operations WHERE target_key=? AND namespace LIKE '%.write' AND state='unknown')) ORDER BY created_at_micros,operation_id LIMIT 1")
                .bind(device).bind(now).bind(namespace(device, &installation, "read")).bind(namespace(device, &installation, "write")).bind(device).fetch_optional(&mut *tx).await?;
            let claimed = match candidate {
                Some(candidate) => SqliteOperationStore::claim_next_in(
                    &mut tx,
                    &candidate,
                    owner,
                    now,
                    now + 120_000_000,
                )
                .await
                .map_err(internal)?,
                None => None,
            };
            tx.commit().await?;
            let Some(claimed) = claimed else {
                return Ok(None);
            };
            if claimed
                .created_at_micros
                .saturating_add(RESTART_EXECUTION_WINDOW_MICROS)
                <= db::now_micros()?
            {
                let mut expired = self.pool.begin_with("BEGIN IMMEDIATE").await?;
                SqliteOperationStore::apply_transition_owned_in(
                    &mut expired,
                    &claimed.operation,
                    Transition::Fail {
                        code: "execution_deadline_expired".into(),
                        retryable: false,
                        retry_not_before_micros: db::now_micros()?,
                    },
                    None,
                    db::now_micros()?,
                )
                .await
                .map_err(internal)?;
                expired.commit().await?;
                continue;
            }
            return Ok(Some(claimed));
        }
    }
    pub async fn uncertain(
        &self,
        device: &str,
        installation: &Uuid,
    ) -> AppResult<Option<StoredOperation>> {
        let id:Option<String>=sqlx::query_scalar("SELECT operation_id FROM _common_operations WHERE namespace=? AND state='unknown' AND NOT EXISTS(SELECT 1 FROM client_observations WHERE client_observations.operation_id=_common_operations.operation_id AND json_extract(report_json,'$.kind')!='unknown') ORDER BY created_at_micros LIMIT 1").bind(namespace(device, installation, "write")).fetch_optional(&self.pool).await?;
        match id {
            Some(id) => self.store.get(&id).await.map_err(internal),
            None => Ok(None),
        }
    }
    pub fn task(&self, stored: &StoredOperation) -> AppResult<Task> {
        decode_task(&self.secrets, stored)
    }
    pub async fn disconnected(&self, stored: &StoredOperation) -> AppResult<()> {
        if stored.operation.state == OperationState::Running {
            if resource(&self.task(stored)?.command) == "read" {
                self.store
                    .apply_transition_owned(
                        &stored.operation,
                        Transition::Fail {
                            code: "read_interrupted".into(),
                            retryable: false,
                            retry_not_before_micros: db::now_micros()?,
                        },
                        None,
                        db::now_micros()?,
                    )
                    .await
                    .map_err(internal)?;
                return Ok(());
            }
            self.store
                .abandon_claim(&stored.operation, "client_disconnected", db::now_micros()?)
                .await
                .map_err(internal)?;
        }
        Ok(())
    }
    pub async fn complete(
        &self,
        stored: &StoredOperation,
        session: &str,
        report: Report,
    ) -> AppResult<()> {
        let task = self.task(stored)?;
        validate_report(&task.command, &report)?;
        let mut tx = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let valid:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM devices WHERE device_id=? AND session_id=? AND revoked_at_micros IS NULL)").bind(&stored.operation.target_key).bind(session).fetch_one(&mut *tx).await?;
        if !valid {
            return Err(AppError::Unauthorized);
        }
        let bytes = serde_json::to_vec(&report).map_err(internal)?;
        if stored.operation.state == OperationState::Unknown {
            // Evidence only: never forge a human xcss resolution.
            let state: String =
                sqlx::query_scalar("SELECT state FROM _common_operations WHERE operation_id=?")
                    .bind(&stored.operation.operation_id)
                    .fetch_one(&mut *tx)
                    .await?;
            if state != "unknown" {
                return Err(AppError::Conflict("任务已由管理员处理".into()));
            }
            let observed: Option<String> = sqlx::query_scalar(
                "SELECT report_json FROM client_observations WHERE operation_id=?",
            )
            .bind(&stored.operation.operation_id)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(observed) = &observed {
                let previous: Report = serde_json::from_str(observed).map_err(internal)?;
                if previous == report {
                    tx.commit().await?;
                    return Ok(());
                }
                if !matches!(previous, Report::Unknown { .. }) {
                    return Err(AppError::Conflict("已确认的执行证据不可改写".into()));
                }
            }
            sqlx::query("INSERT INTO client_observations(operation_id,report_json,observed_at_micros) VALUES(?,?,?) ON CONFLICT(operation_id) DO UPDATE SET report_json=excluded.report_json,observed_at_micros=excluded.observed_at_micros")
                .bind(&stored.operation.operation_id).bind(std::str::from_utf8(&bytes).map_err(internal)?).bind(db::now_micros()?).execute(&mut *tx).await?;
            db::audit(
                &mut tx,
                "operation.reconciled",
                &stored.operation.target_key,
                "client",
                Some(&stored.operation.operation_id),
            )
            .await?;
            record_configuration(&mut tx, &stored.operation.target_key, &report).await?;
            tx.commit().await?;
            return Ok(());
        } else {
            let transition = match &report {
                Report::ConfigRead { .. }
                | Report::ConfigSaved { .. }
                | Report::RestartAcknowledged { .. }
                | Report::ApplicationsRead { .. }
                | Report::ApplicationSaved { .. }
                | Report::ApplicationDeleted { .. }
                | Report::ApplicationClosed {}
                | Report::CoverUploaded { .. }
                | Report::PairingPinSubmitted {}
                | Report::PendingPairingsRead { .. }
                | Report::PairedClientsRead { .. }
                | Report::PairedClientUpdated { .. }
                | Report::LogsRead { .. }
                | Report::DiagnosticsRead { .. }
                | Report::VirtualInputStatusRead { .. }
                | Report::MaintenanceCompleted { .. }
                | Report::ServiceStatusRead { .. }
                | Report::ServiceControlled { .. } => Transition::Succeed,
                Report::Unknown { .. } if resource(&task.command) == "read" => Transition::Fail {
                    code: "read_interrupted".into(),
                    retryable: false,
                    retry_not_before_micros: db::now_micros()?,
                },
                Report::Unknown { .. } => Transition::MarkIndeterminate {
                    code: "client_uncertain".into(),
                },
                Report::Conflict { .. } | Report::Rejected { .. } => Transition::Fail {
                    code: if matches!(report, Report::Conflict { .. }) {
                        "configuration_conflict"
                    } else {
                        "client_rejected"
                    }
                    .into(),
                    retryable: false,
                    retry_not_before_micros: db::now_micros()?,
                },
            };
            SqliteOperationStore::apply_transition_owned_in(
                &mut tx,
                &stored.operation,
                transition,
                Some(&bytes),
                db::now_micros()?,
            )
            .await
            .map_err(internal)?;
        }
        record_configuration(&mut tx, &stored.operation.target_key, &report).await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn deliver_outbox(&self) -> AppResult<u64> {
        let events = self
            .store
            .pending_audit_events(128)
            .await
            .map_err(internal)?;
        let mut count = 0;
        for event in events {
            let stored = self
                .store
                .get(&event.operation_id)
                .await
                .map_err(internal)?
                .ok_or_else(|| internal(anyhow::anyhow!("missing audited operation")))?;
            let mut tx = self.pool.begin().await?;
            sqlx::query("INSERT INTO audit_logs(action,target,detail,actor,created_at_micros,outbox_id) VALUES(?,?,?,?,?,?) ON CONFLICT(outbox_id) DO NOTHING")
                .bind(format!("{}.{}",stored.action,event.to_state)).bind(&stored.operation.target_key).bind(format!("operation_id={} from={} state={}",event.operation_id,event.from_state,event.to_state)).bind(payload(&stored)?.actor).bind(event.created_at_micros).bind(&event.event_id).execute(&mut *tx).await?;
            if SqliteOperationStore::mark_audit_delivered_in(
                &mut tx,
                &event.event_id,
                db::now_micros()?,
            )
            .await
            .map_err(internal)?
            {
                count += 1;
            }
            tx.commit().await?;
        }
        Ok(count)
    }
    pub async fn run_until(self, mut shutdown: watch::Receiver<bool>) -> Result<(), String> {
        loop {
            if *shutdown.borrow() {
                break;
            }
            self.deliver_outbox()
                .await
                .map_err(|_| "operation audit unavailable".to_owned())?;
            let spaces: Vec<String> = sqlx::query_scalar(
                "SELECT DISTINCT namespace FROM _common_operations WHERE state='running'",
            )
            .fetch_all(&self.pool)
            .await
            .map_err(|_| "operation database unavailable".to_owned())?;
            for space in spaces {
                self.store
                    .recover_expired(
                        &space,
                        db::now_micros().map_err(|_| "clock unavailable".to_owned())?,
                    )
                    .await
                    .map_err(|_| "operation recovery unavailable".to_owned())?;
            }
            self.release_uncertain_reads()
                .await
                .map_err(|_| "read operation recovery unavailable".to_owned())?;
            tokio::select! {_ = xcss::server_runtime::wait_for_shutdown(&mut shutdown)=>break,_=tokio::time::sleep(Duration::from_secs(1))=>{}}
        }
        Ok(())
    }
}
pub fn validate_snapshot(snapshot: &ConfigSnapshot) -> AppResult<()> {
    xscs_protocol::validate_revision(&snapshot.revision)
        .map_err(|_| AppError::BadRequest("Client 配置修订无效".into()))?;
    if !xscs_protocol::is_supported_sunshine_version(&snapshot.sunshine_version)
        || snapshot
            .fields
            .iter()
            .any(|(key, value)| !xscs_protocol::config::contains_field(key) || value.len() > 512)
    {
        return Err(AppError::BadRequest("Client 配置响应无效".into()));
    }
    Ok(())
}
fn validate_report(command: &Command, report: &Report) -> AppResult<()> {
    xscs_protocol::validate_report(command, report)
        .map_err(|_| AppError::BadRequest("Client 结果与业务指令不符".into()))
}

#[cfg(test)]
mod capacity_tests {
    use super::*;

    async fn registered(pool: &sqlx::SqlitePool) -> (String, String) {
        let ticket = db::create_device(
            pool,
            &SecretBox::new("test", [7; 32]).unwrap(),
            "测试设备",
            "admin",
        )
        .await
        .unwrap();
        let credential = db::random_token();
        db::enroll(
            pool,
            &ticket.device.id,
            Uuid::new_v4(),
            &ticket.token,
            &credential,
        )
        .await
        .unwrap();
        let capabilities = Capabilities {
            protocol: PROTOCOL.into(),
            client_version: "test".into(),
            os: xscs_protocol::ClientOs::LinuxX86_64,
            sunshine_version: xscs_protocol::SUNSHINE_VERSION.into(),
            restart_allowed: true,
            managed_fields: xscs_protocol::config::FIELD_DEFINITIONS
                .iter()
                .map(|field| field.key.to_owned())
                .collect(),
            configuration_overwrite: true,
            application_management: true,
            application_host_commands_allowed: true,
            moonlight_pairing_management: true,
            pending_pairing_listing: true,
            diagnostics: true,
            maintenance: true,
            service_control: true,
        };
        sqlx::query("UPDATE devices SET capabilities_json=? WHERE device_id=?")
            .bind(serde_json::to_string(&capabilities).unwrap())
            .bind(&ticket.device.id)
            .execute(pool)
            .await
            .unwrap();
        (ticket.device.id, credential)
    }

    async fn fixture() -> (tempfile::TempDir, OperationManager, String, String) {
        let directory = tempfile::tempdir().unwrap();
        let pool = db::open_or_initialize(&format!(
            "sqlite://{}",
            directory.path().join("state.db").display()
        ))
        .await
        .unwrap();
        let (first, _) = registered(&pool).await;
        let (second, _) = registered(&pool).await;
        let mut manager = OperationManager::new(pool, SecretBox::new("test", [7; 32]).unwrap());
        manager.history_capacity.free_floor = 0;
        (directory, manager, first, second)
    }

    #[tokio::test]
    async fn retained_row_budget_is_per_instance_and_retries_do_not_need_new_capacity() {
        let (_directory, mut manager, first, second) = fixture().await;
        manager.history_capacity.device_rows = 1;
        manager.history_capacity.total_rows = 2;
        let accepted = manager
            .enqueue("admin", &first, "accepted", Command::ReadConfig {})
            .await
            .unwrap();
        assert!(matches!(
            manager
                .enqueue("admin", &first, "too-many", Command::ReadDiagnostics {})
                .await,
            Err(AppError::HistoryCapacity)
        ));
        let repeated = manager
            .enqueue("admin", &first, "accepted", Command::ReadConfig {})
            .await
            .unwrap();
        assert_eq!(accepted.operation_id, repeated.operation_id);
        assert!(matches!(
            manager
                .enqueue("admin", &first, "accepted", Command::ReadDiagnostics {})
                .await,
            Err(AppError::HistoryCapacity)
        ));
        assert!(
            manager
                .enqueue("admin", &second, "other-instance", Command::ReadConfig {})
                .await
                .is_ok()
        );
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _common_operations")
            .fetch_one(&manager.pool)
            .await
            .unwrap();
        assert_eq!(count, 2);
        let (third, _) = registered(&manager.pool).await;
        assert!(matches!(
            manager
                .enqueue("admin", &third, "global-full", Command::ReadConfig {})
                .await,
            Err(AppError::HistoryCapacity)
        ));
        // A full admission budget cannot prevent existing work from reaching a final durable state.
        let lease = manager
            .store
            .claim_next(
                &format!(
                    "xscc.client.v1.{first}.{}.read",
                    db::get_device(&manager.pool, &first)
                        .await
                        .unwrap()
                        .installation_id
                        .unwrap()
                ),
                "worker",
                db::now_micros().unwrap(),
                db::now_micros().unwrap() + 120_000_000,
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(lease.operation.operation_id, accepted.operation_id);
        let result = serde_json::to_vec(&Report::ConfigRead {
            snapshot: ConfigSnapshot {
                revision: "a".repeat(64),
                sunshine_version: xscs_protocol::SUNSHINE_VERSION.into(),
                fields: std::collections::BTreeMap::new(),
                effectiveness: xscs_protocol::Effectiveness::PendingVerification,
            },
        })
        .unwrap();
        let finish = manager
            .store
            .apply_transition_owned(
                &lease.operation,
                Transition::Succeed,
                Some(&result),
                db::now_micros().unwrap(),
            )
            .await;
        assert!(finish.is_ok());
        assert_eq!(
            manager
                .store
                .get(&accepted.operation_id)
                .await
                .unwrap()
                .unwrap()
                .operation
                .state,
            OperationState::Succeeded
        );
        assert!(manager.store.pending_audit_count().await.unwrap() > 0);
    }

    #[tokio::test]
    async fn one_instance_byte_budget_leaves_other_instances_admission_and_durable_history() {
        let (_directory, mut manager, first, second) = fixture().await;
        manager.history_capacity.device_bytes = 900 * 1024;
        let accepted = manager
            .enqueue("admin", &first, "first", Command::ReadConfig {})
            .await
            .unwrap();
        assert!(matches!(
            manager
                .enqueue("admin", &first, "device-full", Command::ReadConfig {})
                .await,
            Err(AppError::HistoryCapacity)
        ));
        assert_eq!(
            manager
                .enqueue("admin", &first, "first", Command::ReadConfig {})
                .await
                .unwrap()
                .operation_id,
            accepted.operation_id
        );
        assert!(
            manager
                .enqueue(
                    "admin",
                    &second,
                    "different-instance",
                    Command::ReadConfig {}
                )
                .await
                .is_ok()
        );
        assert!(
            manager
                .store
                .get(&accepted.operation_id)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn byte_and_free_space_budgets_reserve_receipts_before_admission_and_never_delete_facts()
    {
        let (_directory, mut manager, first, _) = fixture().await;
        let accepted = manager
            .enqueue("admin", &first, "accepted", Command::ReadConfig {})
            .await
            .unwrap();
        manager.history_capacity.database_bytes = 1;
        assert!(matches!(
            manager
                .enqueue("admin", &first, "bytes-full", Command::ReadConfig {})
                .await,
            Err(AppError::HistoryCapacity)
        ));
        assert_eq!(
            manager
                .enqueue("admin", &first, "accepted", Command::ReadConfig {})
                .await
                .unwrap()
                .operation_id,
            accepted.operation_id
        );
        manager.history_capacity.database_bytes = u64::MAX;
        manager.history_capacity.free_floor = u64::MAX;
        assert!(matches!(
            manager
                .enqueue("admin", &first, "disk-full", Command::ReadConfig {})
                .await,
            Err(AppError::HistoryCapacity)
        ));
        assert_eq!(
            manager
                .store
                .get(&accepted.operation_id)
                .await
                .unwrap()
                .unwrap()
                .operation
                .state,
            OperationState::Pending
        );
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _common_operations")
            .fetch_one(&manager.pool)
            .await
            .unwrap();
        assert_eq!(count, 1);
    }
}
