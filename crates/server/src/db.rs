use crate::{
    crypto::SecretBox,
    error::{AppError, AppResult},
    model::{DeviceView, validate_instance_name},
};
use rand::TryRng;
use serde::Serialize;
use sha2::{Digest, Sha256};
use sqlx::{Sqlite, SqlitePool, Transaction};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;
use xscs_protocol::{
    AUTHORIZATION_CODE_LENGTH, Binding, Capabilities, DiagnosticSnapshot,
    is_valid_authorization_code,
};
pub const SCHEMA: &str = "xscs";
pub use crate::database_schema::open_existing;
#[cfg(test)]
pub use crate::database_schema::{initialize_empty, open_or_initialize};

type PairingStateRow = (Option<Vec<u8>>, Option<String>, Option<i64>);

#[derive(Clone, sqlx::FromRow)]
pub struct Device {
    pub device_id: String,
    pub name: String,
    pub role: String,
    pub installation_id: Option<String>,
    pub credential_hash: Option<Vec<u8>>,
    pub enrollment_hash: Option<Vec<u8>>,
    pub authorization_code_enc: String,
    pub revoked_at_micros: Option<i64>,
    pub session_id: Option<String>,
    pub last_seen_at_micros: Option<i64>,
    pub health_at_micros: Option<i64>,
    pub capabilities_json: Option<String>,
    pub diagnostics_json: Option<String>,
    pub addresses_json: Option<String>,
    pub addresses_at_micros: Option<i64>,
    pub created_at_micros: i64,
    pub updated_at_micros: i64,
}
impl Device {
    pub fn view(&self, now: i64) -> AppResult<DeviceView> {
        let online = self.revoked_at_micros.is_none()
            && self.session_id.is_some()
            && self
                .last_seen_at_micros
                .is_some_and(|seen| now.saturating_sub(seen) < 45_000_000);
        for json in [&self.diagnostics_json, &self.capabilities_json]
            .into_iter()
            .flatten()
        {
            if json.len() > xscs_protocol::MAX_REPORT_BYTES {
                return Err(AppError::Internal(anyhow::anyhow!(
                    "stored Client JSON exceeds byte boundary"
                )));
            }
        }
        let diagnostics: Option<DiagnosticSnapshot> = self
            .diagnostics_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| AppError::Internal(error.into()))?;
        if let Some(snapshot) = &diagnostics {
            crate::operations::validate_diagnostics(snapshot)?;
        }
        let capabilities: Option<Capabilities> = self
            .capabilities_json
            .as_deref()
            .map(serde_json::from_str)
            .transpose()
            .map_err(|error| AppError::Internal(error.into()))?;
        if let Some(value) = &capabilities {
            value
                .validate()
                .map_err(|error| AppError::Internal(error.into()))?;
        }
        Ok(DeviceView {
            id: self.device_id.clone(),
            name: self.name.clone(),
            role: crate::access::role(self)?,
            addresses: crate::access::stored_addresses(self)?,
            addresses_at_micros: self.addresses_at_micros,
            registered: self.installation_id.is_some(),
            pairing_pending: self.enrollment_hash.is_some()
                && self.installation_id.is_none()
                && self.revoked_at_micros.is_none(),
            revoked: self.revoked_at_micros.is_some(),
            client_online: online,
            diagnostics,
            capabilities,
            last_seen_at_micros: self.last_seen_at_micros,
        })
    }
}
#[derive(Serialize)]
pub struct EnrollmentTicket {
    pub device: DeviceView,
    pub manager_id: Uuid,
    pub token: String,
}
pub async fn manager_id(pool: &SqlitePool) -> AppResult<Uuid> {
    let id: String =
        sqlx::query_scalar("SELECT manager_id FROM manager_identity WHERE singleton=1")
            .fetch_one(pool)
            .await?;
    Uuid::parse_str(&id)
        .map_err(|_| AppError::Internal(anyhow::anyhow!("invalid Manager identity")))
}
pub async fn get_device(pool: &SqlitePool, id: &str) -> AppResult<Device> {
    sqlx::query_as("SELECT * FROM devices WHERE device_id=?")
        .bind(id)
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound("设备不存在".into()))
}
pub async fn list_devices(pool: &SqlitePool) -> AppResult<Vec<DeviceView>> {
    let now = now_micros()?;
    sqlx::query_as::<_, Device>("SELECT * FROM devices ORDER BY name COLLATE NOCASE,name,device_id")
        .fetch_all(pool)
        .await?
        .iter()
        .map(|device| device.view(now))
        .collect()
}
pub async fn create_device(
    pool: &SqlitePool,
    secrets: &SecretBox,
    name: &str,
    role: xscs_protocol::InstanceRole,
    actor: &str,
) -> AppResult<EnrollmentTicket> {
    validate_instance_name(name)?;
    let id = Uuid::new_v4().to_string();
    let now = now_micros()?;
    let token = random_authorization_code();
    let authorization_code_enc = secrets.encrypt_client_authorization(&id, &token)?;
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM devices")
        .fetch_one(&mut *tx)
        .await?;
    if count >= 4096 {
        return Err(AppError::Conflict("设备容量已满".into()));
    }
    sqlx::query("INSERT INTO devices(device_id,name,role,enrollment_hash,authorization_code_enc,created_at_micros,updated_at_micros) VALUES(?,?,?,?,?,?,?)")
        .bind(&id).bind(name).bind(role.as_str()).bind(token_hash(&token).as_slice()).bind(authorization_code_enc).bind(now).bind(now).execute(&mut *tx).await?;
    audit(&mut tx, "device.create", &id, actor, None).await?;
    tx.commit().await?;
    Ok(EnrollmentTicket {
        device: get_device(pool, &id).await?.view(now)?,
        manager_id: manager_id(pool).await?,
        token,
    })
}
pub async fn get_authorization(
    pool: &SqlitePool,
    secrets: &SecretBox,
    id: &str,
) -> AppResult<String> {
    let encrypted: Option<String> =
        sqlx::query_scalar("SELECT authorization_code_enc FROM devices WHERE device_id=?")
            .bind(id)
            .fetch_optional(pool)
            .await?;
    secrets.decrypt_client_authorization(
        id,
        &encrypted.ok_or_else(|| AppError::NotFound("设备不存在".into()))?,
    )
}
pub async fn rotate_authorization(
    pool: &SqlitePool,
    secrets: &SecretBox,
    id: &str,
    authorization_code: &str,
    actor: &str,
) -> AppResult<String> {
    validate_authorization_code(authorization_code)?;
    let encrypted = secrets.encrypt_client_authorization(id, authorization_code)?;
    let now = now_micros()?;
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let changed = sqlx::query(
        "UPDATE devices SET enrollment_hash=?,authorization_code_enc=?,installation_id=NULL,\
        credential_hash=NULL,session_id=NULL,last_seen_at_micros=NULL,health_at_micros=NULL,\
         capabilities_json=NULL,diagnostics_json=NULL,addresses_json=NULL,addresses_at_micros=NULL,updated_at_micros=? \
         WHERE device_id=? AND revoked_at_micros IS NULL",
    )
    .bind(token_hash(authorization_code).as_slice())
    .bind(encrypted)
    .bind(now)
    .bind(id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed != 1 {
        tx.rollback().await?;
        return if get_device(pool, id).await.is_err() {
            Err(AppError::NotFound("设备不存在".into()))
        } else {
            Err(AppError::Conflict("已撤销实例不能更换密码".into()))
        };
    }
    retire_operations_for_invalidation(&mut tx, id, now, "authorization_rotated").await?;
    audit(&mut tx, "device.authorization.rotate", id, actor, None).await?;
    tx.commit().await?;
    Ok(authorization_code.to_owned())
}

async fn retire_operations_for_invalidation(
    tx: &mut Transaction<'_, Sqlite>,
    device_id: &str,
    now: i64,
    reason: &str,
) -> AppResult<()> {
    // Credential invalidation fences delivery in the same write transaction that clears the
    // old credential. Pending work did not run; running work needs review.
    let operations: Vec<(String, String)> = sqlx::query_as(
        "SELECT operation_id,state FROM _common_operations \
         WHERE target_key=? AND state IN ('pending','running') ORDER BY operation_id",
    )
    .bind(device_id)
    .fetch_all(&mut **tx)
    .await?;
    for (operation_id, from) in operations {
        let to = if from == "pending" {
            "failed"
        } else {
            "unknown"
        };
        let changed = sqlx::query(
            "UPDATE _common_operations SET state=?,error_code=?,\
             lease_owner=NULL,lease_expiry_micros=NULL,updated_at_micros=? \
             WHERE operation_id=? AND state=?",
        )
        .bind(to)
        .bind(reason)
        .bind(now)
        .bind(&operation_id)
        .bind(&from)
        .execute(&mut **tx)
        .await?
        .rows_affected();
        if changed != 1 {
            return Err(AppError::Internal(anyhow::anyhow!(
                "operation changed during credential invalidation"
            )));
        }
        sqlx::query(
            "INSERT INTO _common_operation_audit_outbox \
             (event_id,operation_id,from_state,to_state,payload_json,created_at_micros) \
             VALUES(?,?,?,?,?,?)",
        )
        .bind(Uuid::new_v4().to_string())
        .bind(&operation_id)
        .bind(&from)
        .bind(to)
        .bind(serde_json::json!({"from":from,"to":to}).to_string())
        .bind(now)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}
pub async fn resolve_pairing(pool: &SqlitePool, token: &str) -> AppResult<serde_json::Value> {
    validate_pairing_authorization_code(token).map_err(|_| AppError::Unauthorized)?;
    let id: Option<String> = sqlx::query_scalar("SELECT device_id FROM devices WHERE enrollment_hash=? AND installation_id IS NULL AND revoked_at_micros IS NULL")
        .bind(token_hash(token).as_slice()).fetch_optional(pool).await?;
    let id = id.ok_or(AppError::Unauthorized)?;
    let device = get_device(pool, &id).await?;
    Ok(
        serde_json::json!({"manager_id": manager_id(pool).await?, "device_id": id, "role": crate::access::role(&device)?}),
    )
}
pub async fn cancel_pairing(pool: &SqlitePool, id: &str, actor: &str) -> AppResult<()> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let row: Option<PairingStateRow> = sqlx::query_as(
        "SELECT enrollment_hash,installation_id,revoked_at_micros FROM devices WHERE device_id=?",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    let Some((enrollment_hash, installation_id, revoked_at)) = row else {
        return Err(AppError::NotFound("设备不存在".into()));
    };
    if enrollment_hash.is_none() && installation_id.is_none() && revoked_at.is_none() {
        tx.commit().await?;
        return Ok(());
    }
    let changed = sqlx::query("UPDATE devices SET enrollment_hash=NULL,updated_at_micros=? WHERE device_id=? AND installation_id IS NULL AND revoked_at_micros IS NULL AND enrollment_hash IS NOT NULL")
        .bind(now_micros()?).bind(id).execute(&mut *tx).await?.rows_affected();
    if changed != 1 {
        return Err(AppError::Conflict("配对码已使用".into()));
    }
    audit(&mut tx, "device.pairing.cancel", id, actor, None).await?;
    tx.commit().await?;
    Ok(())
}
pub async fn rename(pool: &SqlitePool, id: &str, name: &str, actor: &str) -> AppResult<DeviceView> {
    validate_instance_name(name)?;
    let mut tx = pool.begin().await?;
    if sqlx::query("UPDATE devices SET name=?,updated_at_micros=? WHERE device_id=?")
        .bind(name)
        .bind(now_micros()?)
        .bind(id)
        .execute(&mut *tx)
        .await?
        .rows_affected()
        != 1
    {
        return Err(AppError::NotFound("设备不存在".into()));
    }
    audit(&mut tx, "device.rename", id, actor, None).await?;
    tx.commit().await?;
    get_device(pool, id).await?.view(now_micros()?)
}
pub async fn revoke(pool: &SqlitePool, id: &str, actor: &str) -> AppResult<()> {
    let now = now_micros()?;
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    if sqlx::query("UPDATE devices SET revoked_at_micros=?,credential_hash=NULL,enrollment_hash=NULL,session_id=NULL,updated_at_micros=? WHERE device_id=?")
        .bind(now).bind(now).bind(id).execute(&mut *tx).await?.rows_affected()!=1 { return Err(AppError::NotFound("设备不存在".into())); }
    retire_operations_for_invalidation(&mut tx, id, now, "device_revoked").await?;
    audit(&mut tx, "device.revoke", id, actor, None).await?;
    tx.commit().await?;
    Ok(())
}
pub async fn delete_device(pool: &SqlitePool, id: &str, actor: &str) -> AppResult<()> {
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM devices WHERE device_id=?)")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    if !exists {
        return Err(AppError::NotFound("设备不存在".into()));
    }
    audit(
        &mut tx,
        "device.delete",
        id,
        actor,
        Some("instance permanently deleted; credentials invalidated"),
    )
    .await?;
    sqlx::query("DELETE FROM devices WHERE device_id=?")
        .bind(id)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
pub async fn enroll(
    pool: &SqlitePool,
    id: &str,
    installation: Uuid,
    token: &str,
    credential: &str,
) -> AppResult<Binding> {
    validate_pairing_authorization_code(token)?;
    validate_token(credential)?;
    if installation.is_nil() {
        return Err(AppError::BadRequest("安装身份无效".into()));
    }
    let now = now_micros()?;
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let changed = sqlx::query("UPDATE devices SET installation_id=?,credential_hash=?,updated_at_micros=? WHERE device_id=? AND installation_id IS NULL AND revoked_at_micros IS NULL AND enrollment_hash=?")
        .bind(installation.to_string()).bind(token_hash(credential).as_slice()).bind(now).bind(id).bind(token_hash(token).as_slice()).execute(&mut *tx).await?.rows_affected();
    if changed != 1 {
        return Err(AppError::Unauthorized);
    }
    audit(&mut tx, "device.enroll", id, "client", None).await?;
    tx.commit().await?;
    Ok(Binding {
        manager_id: manager_id(pool).await?,
        device_id: Uuid::parse_str(id).map_err(|_| AppError::Unauthorized)?,
        installation_id: installation,
    })
}
pub async fn authenticate_device(pool: &SqlitePool, credential: &str) -> AppResult<Device> {
    validate_token(credential).map_err(|_| AppError::Unauthorized)?;
    sqlx::query_as("SELECT * FROM devices WHERE credential_hash=? AND revoked_at_micros IS NULL AND installation_id IS NOT NULL")
        .bind(token_hash(credential).as_slice()).fetch_optional(pool).await?.ok_or(AppError::DeviceCredentialRejected)
}
pub async fn unbind(pool: &SqlitePool, device: &Device) -> AppResult<()> {
    let now = now_micros()?;
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let changed = sqlx::query("UPDATE devices SET installation_id=NULL,credential_hash=NULL,session_id=NULL,last_seen_at_micros=NULL,health_at_micros=NULL,capabilities_json=NULL,diagnostics_json=NULL,addresses_json=NULL,addresses_at_micros=NULL,updated_at_micros=? WHERE device_id=? AND installation_id=? AND credential_hash=? AND revoked_at_micros IS NULL")
        .bind(now).bind(&device.device_id).bind(&device.installation_id).bind(&device.credential_hash).execute(&mut *tx).await?.rows_affected();
    if changed != 1 {
        return Err(AppError::Unauthorized);
    }
    retire_operations_for_invalidation(&mut tx, &device.device_id, now, "device_unbound").await?;
    audit(&mut tx, "device.unbind", &device.device_id, "client", None).await?;
    tx.commit().await?;
    Ok(())
}
pub async fn binding(pool: &SqlitePool, device: &Device) -> AppResult<Binding> {
    Ok(Binding {
        manager_id: manager_id(pool).await?,
        device_id: Uuid::parse_str(&device.device_id).map_err(|_| AppError::Unauthorized)?,
        installation_id: device
            .installation_id
            .as_ref()
            .and_then(|id| Uuid::parse_str(id).ok())
            .ok_or_else(|| AppError::Conflict("设备尚未注册".into()))?,
    })
}
pub fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::SysRng
        .try_fill_bytes(&mut bytes)
        .expect("operating system entropy is available");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
pub fn random_authorization_code() -> String {
    const ALPHABET: &[u8; 36] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut value = String::with_capacity(AUTHORIZATION_CODE_LENGTH);
    let mut bytes = [0_u8; 64];
    while value.len() < AUTHORIZATION_CODE_LENGTH {
        rand::rngs::SysRng
            .try_fill_bytes(&mut bytes)
            .expect("operating system entropy is available");
        for byte in bytes {
            if byte < 252 {
                value.push(ALPHABET[usize::from(byte % 36)] as char);
                if value.len() == AUTHORIZATION_CODE_LENGTH {
                    break;
                }
            }
        }
    }
    value
}
pub fn token_hash(token: &str) -> [u8; 32] {
    Sha256::digest(token.as_bytes()).into()
}
pub fn validate_token(token: &str) -> AppResult<()> {
    if token.len() != 64
        || !token
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        Err(AppError::BadRequest("设备凭据格式无效".into()))
    } else {
        Ok(())
    }
}
pub fn validate_authorization_code(value: &str) -> AppResult<()> {
    if !is_valid_authorization_code(value) {
        Err(AppError::BadRequest("实例密码格式无效".into()))
    } else {
        Ok(())
    }
}
fn validate_pairing_authorization_code(value: &str) -> AppResult<()> {
    if validate_authorization_code(value).is_ok() {
        Ok(())
    } else {
        Err(AppError::BadRequest("实例密码格式无效".into()))
    }
}
pub async fn audit(
    tx: &mut Transaction<'_, Sqlite>,
    action: &str,
    target: &str,
    actor: &str,
    detail: Option<&str>,
) -> AppResult<()> {
    sqlx::query(
        "INSERT INTO audit_logs(action,target,actor,detail,created_at_micros) VALUES(?,?,?,?,?)",
    )
    .bind(action)
    .bind(target)
    .bind(actor)
    .bind(detail)
    .bind(now_micros()?)
    .execute(&mut **tx)
    .await?;
    Ok(())
}
pub fn now_micros() -> anyhow::Result<i64> {
    Ok(i64::try_from(
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_micros(),
    )?)
}
pub async fn ready(pool: &SqlitePool) -> bool {
    crate::database_schema::is_current(pool).await
}
pub async fn require_current_runtime_state(
    pool: &SqlitePool,
    secrets: &SecretBox,
) -> anyhow::Result<()> {
    xcss::sqlite::integrity_check(pool).await?;
    xcss::sqlite::foreign_key_check(pool).await?;
    manager_id(pool).await?;
    use futures_util::TryStreamExt as _;
    let now = now_micros()?;
    let mut devices =
        sqlx::query_as::<_, Device>("SELECT * FROM devices ORDER BY device_id").fetch(pool);
    while let Some(device) = devices.try_next().await? {
        device.view(now)?;
        let code = secrets
            .decrypt_client_authorization(&device.device_id, &device.authorization_code_enc)
            .map_err(|_| anyhow::anyhow!("stored client authorization is unreadable"))?;
        validate_pairing_authorization_code(&code)
            .map_err(|_| anyhow::anyhow!("stored client authorization is invalid"))?;
        if let Some(stored_hash) = device.enrollment_hash {
            anyhow::ensure!(
                stored_hash == token_hash(&code),
                "stored client authorization digest does not match its encrypted value"
            );
        }
    }
    drop(devices);
    let store = xcss::operations::SqliteOperationStore::new(pool.clone());
    let mut cursor = String::new();
    loop {
        let ids:Vec<String>=sqlx::query_scalar("SELECT operation_id FROM _common_operations WHERE operation_id>? ORDER BY operation_id LIMIT 128").bind(&cursor).fetch_all(pool).await?;
        if ids.is_empty() {
            break;
        }
        for id in &ids {
            let stored = store
                .get(id)
                .await?
                .ok_or_else(|| anyhow::anyhow!("operation disappeared"))?;
            crate::operations::decode_task(secrets, &stored)?;
        }
        cursor = ids.last().expect("non-empty batch").clone();
    }
    Ok(())
}

#[cfg(test)]
mod authorization_code_tests {
    use super::*;

    #[test]
    fn generated_authorization_codes_have_the_shared_format() {
        for _ in 0..64 {
            let value = random_authorization_code();
            assert_eq!(value.len(), AUTHORIZATION_CODE_LENGTH);
            assert!(is_valid_authorization_code(&value));
            validate_authorization_code(&value).unwrap();
        }
    }
}
