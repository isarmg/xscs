//! One directed ACL, edited from either endpoint and filtered for each bound client.
use crate::{
    db::{self, Device},
    error::{AppError, AppResult},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Sqlite, SqlitePool, Transaction};
use xscs_protocol::{
    ACCESS_LEASE_MS, AccessPeer, AccessPolicy, Backend, Binding, DiagnosticSnapshot, HostAddress,
    InstanceRole, MAX_ACCESS_PEERS, validate_addresses,
};

pub fn role(device: &Device) -> AppResult<InstanceRole> {
    stored_role(&device.role)
}
fn stored_role(role: &str) -> AppResult<InstanceRole> {
    match role {
        "controller" => Ok(InstanceRole::Controller),
        "controlled" => Ok(InstanceRole::Controlled),
        _ => Err(AppError::Internal(anyhow::anyhow!(
            "invalid stored instance role"
        ))),
    }
}
pub(crate) fn stored_addresses(
    json: Option<&str>,
    role: InstanceRole,
) -> AppResult<Vec<HostAddress>> {
    let addresses = match json {
        Some(json) if json.len() <= 2048 => {
            serde_json::from_str(json).map_err(|e| AppError::Internal(e.into()))?
        }
        Some(_) => {
            return Err(AppError::Internal(anyhow::anyhow!(
                "stored address report too large"
            )));
        }
        None => Vec::new(),
    };
    validate_addresses(&addresses).map_err(|e| AppError::Internal(e.into()))?;
    if role == InstanceRole::Controller && !addresses.is_empty() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "controller has host addresses"
        )));
    }
    Ok(addresses)
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccessRules {
    pub peer_ids: Vec<String>,
    pub revision: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateAccessRules {
    pub peer_ids: Vec<String>,
    pub expected_revision: String,
}
async fn rules_in(tx: &mut Transaction<'_, Sqlite>, id: &str) -> AppResult<AccessRules> {
    let ids: Vec<String> = sqlx::query_scalar("SELECT CASE WHEN controller_id=? THEN controlled_id ELSE controller_id END AS peer_id FROM device_access WHERE controller_id=? OR controlled_id=? ORDER BY peer_id")
        .bind(id).bind(id).bind(id).fetch_all(&mut **tx).await?;
    let revision = hex::encode(Sha256::digest(
        serde_json::to_vec(&ids).map_err(|e| AppError::Internal(e.into()))?,
    ));
    Ok(AccessRules {
        peer_ids: ids,
        revision,
    })
}
pub async fn rules(pool: &SqlitePool, id: &str) -> AppResult<AccessRules> {
    let mut tx = pool.begin().await?;
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM devices WHERE device_id=?)")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    if !exists {
        return Err(AppError::NotFound("实例不存在".into()));
    }
    let result = rules_in(&mut tx, id).await?;
    tx.commit().await?;
    Ok(result)
}
pub async fn replace_rules(
    pool: &SqlitePool,
    id: &str,
    mut request: UpdateAccessRules,
    actor: &str,
) -> AppResult<AccessRules> {
    if request.peer_ids.len() > MAX_ACCESS_PEERS {
        return Err(AppError::BadRequest(format!(
            "每个实例最多允许 {MAX_ACCESS_PEERS} 个对端"
        )));
    }
    request.peer_ids.sort();
    if request.peer_ids.windows(2).any(|p| p[0] == p[1]) || request.peer_ids.iter().any(|p| p == id)
    {
        return Err(AppError::BadRequest("授权对端不能重复或包含自身".into()));
    }
    let mut tx = pool.begin_with("BEGIN IMMEDIATE").await?;
    let own: Device = sqlx::query_as("SELECT * FROM devices WHERE device_id=?")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| AppError::NotFound("实例不存在".into()))?;
    if own.revoked_at_micros.is_some() {
        return Err(AppError::Conflict("已撤销实例不能设置授权".into()));
    }
    if rules_in(&mut tx, id).await?.revision != request.expected_revision {
        return Err(AppError::Conflict(
            "授权已被其他页面修改，请重新读取后保存".into(),
        ));
    }
    let own_role = role(&own)?;
    sqlx::query("DELETE FROM device_access WHERE controller_id=? OR controlled_id=?")
        .bind(id)
        .bind(id)
        .execute(&mut *tx)
        .await?;
    for peer_id in &request.peer_ids {
        let peer: Device = sqlx::query_as("SELECT * FROM devices WHERE device_id=?")
            .bind(peer_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or_else(|| AppError::BadRequest("授权对端不存在".into()))?;
        if role(&peer)? == own_role || peer.revoked_at_micros.is_some() {
            return Err(AppError::BadRequest(
                "只能授权未撤销且角色相反的实例".into(),
            ));
        }
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM device_access WHERE controller_id=? OR controlled_id=?",
        )
        .bind(peer_id)
        .bind(peer_id)
        .fetch_one(&mut *tx)
        .await?;
        if count >= MAX_ACCESS_PEERS as i64 {
            return Err(AppError::Conflict("对端授权数量已达上限".into()));
        }
        let (controller, controlled) = if own_role == InstanceRole::Controller {
            (id, peer_id.as_str())
        } else {
            (peer_id.as_str(), id)
        };
        sqlx::query("INSERT INTO device_access(controller_id,controlled_id) VALUES(?,?)")
            .bind(controller)
            .bind(controlled)
            .execute(&mut *tx)
            .await?;
    }
    let result = rules_in(&mut tx, id).await?;
    db::audit(
        &mut tx,
        "device.access.replace",
        id,
        actor,
        Some(&serde_json::to_string(&request.peer_ids).map_err(|e| AppError::Internal(e.into()))?),
    )
    .await?;
    tx.commit().await?;
    Ok(result)
}

/// A report can affect only the authenticated, still-current controlled session.
pub async fn report_addresses(
    pool: &SqlitePool,
    binding: &Binding,
    session: &str,
    addresses: &[HostAddress],
) -> AppResult<()> {
    validate_addresses(addresses).map_err(|_| {
        AppError::BadRequest("主机地址必须是有效的 IP/域名和非零端口，最多四个且不能重复".into())
    })?;
    let role: String = sqlx::query_scalar("SELECT role FROM devices WHERE device_id=?")
        .bind(binding.device_id.to_string())
        .fetch_optional(pool)
        .await?
        .ok_or_else(|| AppError::NotFound("设备不存在".into()))?;
    if stored_role(&role)? == InstanceRole::Controller {
        if !addresses.is_empty() {
            return Err(AppError::BadRequest("控制端不能上报被控地址".into()));
        }
        return Ok(());
    }
    let changed = sqlx::query("UPDATE devices SET addresses_json=?,addresses_at_micros=? WHERE device_id=? AND installation_id=? AND session_id=? AND revoked_at_micros IS NULL")
        .bind(serde_json::to_string(addresses).map_err(|e| AppError::Internal(e.into()))?).bind(db::now_micros()?).bind(binding.device_id.to_string()).bind(binding.installation_id.to_string()).bind(session).execute(pool).await?.rows_affected();
    if changed != 1 {
        return Err(AppError::Unauthorized);
    }
    Ok(())
}

/// Persist one validated heartbeat atomically, fencing superseded or revoked sessions.
/// The role and backend were checked against immutable instance fields at Hello.
pub(crate) async fn record_heartbeat(
    pool: &SqlitePool,
    binding: &Binding,
    session: &str,
    role: InstanceRole,
    backend: Backend,
    diagnostics: Option<&DiagnosticSnapshot>,
    addresses: &[HostAddress],
) -> AppResult<()> {
    validate_addresses(addresses).map_err(|_| {
        AppError::BadRequest("主机地址必须是有效的 IP/域名和非零端口，最多四个且不能重复".into())
    })?;
    if role == InstanceRole::Controller && !addresses.is_empty() {
        return Err(AppError::BadRequest("控制端不能上报被控地址".into()));
    }
    if let Some(snapshot) = diagnostics {
        crate::operations::validate_diagnostics(snapshot)?;
        if snapshot.backend != backend {
            return Err(AppError::BadRequest("Client 诊断类型与实例不匹配".into()));
        }
    }
    let diagnostics = diagnostics
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| AppError::Internal(error.into()))?;
    let addresses =
        serde_json::to_string(addresses).map_err(|error| AppError::Internal(error.into()))?;
    let controlled = role == InstanceRole::Controlled;
    let now = db::now_micros()?;
    let changed = sqlx::query(
        "UPDATE devices SET last_seen_at_micros=?,health_at_micros=?, \
         diagnostics_json=COALESCE(?,diagnostics_json), \
         addresses_json=CASE WHEN ? THEN ? ELSE addresses_json END, \
         addresses_at_micros=CASE WHEN ? THEN ? ELSE addresses_at_micros END \
         WHERE device_id=? AND installation_id=? AND session_id=? AND revoked_at_micros IS NULL",
    )
    .bind(now)
    .bind(now)
    .bind(diagnostics)
    .bind(controlled)
    .bind(addresses)
    .bind(controlled)
    .bind(now)
    .bind(binding.device_id.to_string())
    .bind(binding.installation_id.to_string())
    .bind(session)
    .execute(pool)
    .await?
    .rows_affected();
    if changed != 1 {
        return Err(AppError::Unauthorized);
    }
    Ok(())
}

// Policy refreshes do not read authorization ciphertext, capabilities or diagnostic
// snapshots. Those belong to other boundaries and can be substantially larger.
#[derive(sqlx::FromRow)]
struct PolicyDevice {
    device_id: String,
    name: String,
    role: String,
    installation_id: Option<String>,
    credential_hash: Option<Vec<u8>>,
    revoked_at_micros: Option<i64>,
    session_id: Option<String>,
    last_seen_at_micros: Option<i64>,
    addresses_json: Option<String>,
    addresses_at_micros: Option<i64>,
}

pub async fn policy(
    pool: &SqlitePool,
    secrets: &crate::crypto::SecretBox,
    authenticated: &Device,
    session: Option<&str>,
) -> AppResult<AccessPolicy> {
    let mut tx = pool.begin().await?;
    let own: PolicyDevice = sqlx::query_as("SELECT device_id,name,role,installation_id,credential_hash,revoked_at_micros,session_id,last_seen_at_micros,addresses_json,addresses_at_micros FROM devices WHERE device_id=?")
        .bind(&authenticated.device_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::Unauthorized)?;
    let own_credential = own
        .credential_hash
        .as_deref()
        .ok_or(AppError::Unauthorized)?;
    if own.revoked_at_micros.is_some()
        || own.installation_id.is_none()
        || own.installation_id != authenticated.installation_id
        || own.credential_hash != authenticated.credential_hash
        || session.is_some_and(|s| own.session_id.as_deref() != Some(s))
    {
        return Err(AppError::Unauthorized);
    }
    let manager_id: String =
        sqlx::query_scalar("SELECT manager_id FROM manager_identity WHERE singleton=1")
            .fetch_one(&mut *tx)
            .await?;
    let manager_id =
        uuid::Uuid::parse_str(&manager_id).map_err(|e| AppError::Internal(e.into()))?;
    let binding_of = |device: &PolicyDevice| -> AppResult<Binding> {
        Ok(Binding {
            manager_id,
            device_id: uuid::Uuid::parse_str(&device.device_id)
                .map_err(|e| AppError::Internal(e.into()))?,
            installation_id: uuid::Uuid::parse_str(device.installation_id.as_deref().ok_or_else(
                || AppError::Internal(anyhow::anyhow!("policy row lost installation id")),
            )?)
            .map_err(|e| AppError::Internal(e.into()))?,
        })
    };
    let rows: Vec<PolicyDevice> = sqlx::query_as("SELECT d.device_id,d.name,d.role,d.installation_id,d.credential_hash,d.revoked_at_micros,d.session_id,d.last_seen_at_micros,d.addresses_json,d.addresses_at_micros FROM device_access a JOIN devices d ON d.device_id=CASE WHEN a.controller_id=? THEN a.controlled_id ELSE a.controller_id END WHERE (a.controller_id=? OR a.controlled_id=?) AND d.revoked_at_micros IS NULL AND d.installation_id IS NOT NULL ORDER BY d.device_id")
        .bind(&own.device_id).bind(&own.device_id).bind(&own.device_id).fetch_all(&mut *tx).await?;
    let now = db::now_micros()?;
    let own_binding = binding_of(&own)?;
    let own_role = stored_role(&own.role)?;
    let mut peers = Vec::with_capacity(rows.len());
    for device in rows {
        let peer_binding = binding_of(&device)?;
        let peer_credential = device.credential_hash.as_deref().ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("policy row lost peer credential"))
        })?;
        let session_key = if own_role == InstanceRole::Controller {
            secrets.session_key(&own_binding, &peer_binding, own_credential, peer_credential)?
        } else {
            secrets.session_key(&peer_binding, &own_binding, peer_credential, own_credential)?
        };
        let peer_role = stored_role(&device.role)?;
        let addresses = stored_addresses(device.addresses_json.as_deref(), peer_role)?;
        let online = db::session_is_online(
            device.revoked_at_micros,
            device.session_id.as_deref(),
            device.last_seen_at_micros,
            now,
        );
        let fresh = device
            .addresses_at_micros
            .is_some_and(|at| now >= at && now - at < db::SESSION_FRESH_WINDOW_MICROS);
        peers.push(AccessPeer {
            binding: peer_binding,
            session_key,
            name: device.name,
            role: peer_role,
            online,
            addresses: if online && fresh {
                addresses
            } else {
                Vec::new()
            },
        });
    }
    let policy = AccessPolicy {
        binding: own_binding,
        role: own_role,
        valid_for_ms: ACCESS_LEASE_MS,
        peers,
    };
    policy
        .validate(&policy.binding)
        .map_err(|e| AppError::Internal(e.into()))?;
    tx.commit().await?;
    Ok(policy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use xscs_protocol::Backend;

    async fn session(role: InstanceRole) -> (SqlitePool, Binding) {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        db::initialize_empty(&pool).await.unwrap();
        let ticket = db::create_device(
            &pool,
            &crate::crypto::SecretBox::new("test", [7; 32]).unwrap(),
            "Heartbeat",
            role,
            "admin",
        )
        .await
        .unwrap();
        let binding = db::enroll(
            &pool,
            &ticket.device.id,
            uuid::Uuid::new_v4(),
            &ticket.token,
            &db::random_token(),
        )
        .await
        .unwrap();
        sqlx::query(
            "UPDATE devices SET session_id='current',last_seen_at_micros=1 WHERE device_id=?",
        )
        .bind(&ticket.device.id)
        .execute(&pool)
        .await
        .unwrap();
        (pool, binding)
    }

    #[tokio::test]
    async fn heartbeat_is_atomic_and_preserves_absent_diagnostics() {
        let (pool, binding) = session(InstanceRole::Controlled).await;
        let snapshot = DiagnosticSnapshot::awaiting_integration();
        let addresses = vec![HostAddress {
            host: "host.example.com".into(),
            port: 21118,
        }];
        record_heartbeat(
            &pool,
            &binding,
            "current",
            InstanceRole::Controlled,
            Backend::Rustdesk,
            Some(&snapshot),
            &addresses,
        )
        .await
        .unwrap();
        let first = db::get_device(&pool, &binding.device_id.to_string())
            .await
            .unwrap();
        assert_eq!(first.last_seen_at_micros, first.health_at_micros);
        assert_eq!(first.last_seen_at_micros, first.addresses_at_micros);
        assert_eq!(
            stored_addresses(first.addresses_json.as_deref(), InstanceRole::Controlled).unwrap(),
            addresses
        );
        record_heartbeat(
            &pool,
            &binding,
            "current",
            InstanceRole::Controlled,
            Backend::Rustdesk,
            None,
            &[],
        )
        .await
        .unwrap();
        let second = db::get_device(&pool, &binding.device_id.to_string())
            .await
            .unwrap();
        assert_eq!(second.diagnostics_json, first.diagnostics_json);
        assert_eq!(second.addresses_json.as_deref(), Some("[]"));
        assert_eq!(second.last_seen_at_micros, second.addresses_at_micros);
    }

    #[tokio::test]
    async fn heartbeat_and_liveness_fence_stale_installations_sessions_and_revocation() {
        let (pool, binding) = session(InstanceRole::Controlled).await;
        let mut previous_installation = binding.clone();
        previous_installation.installation_id = uuid::Uuid::new_v4();
        for (identity, owner) in [
            (&binding, "superseded"),
            (&previous_installation, "current"),
        ] {
            assert!(matches!(
                record_heartbeat(
                    &pool,
                    identity,
                    owner,
                    InstanceRole::Controlled,
                    Backend::Rustdesk,
                    None,
                    &[]
                )
                .await,
                Err(AppError::Unauthorized)
            ));
            assert!(!db::touch_session(&pool, identity, owner).await.unwrap());
            assert!(!db::current_session(&pool, identity, owner).await.unwrap());
        }
        let invalid_address = [HostAddress {
            host: "host.example.com".into(),
            port: 0,
        }];
        assert!(matches!(
            record_heartbeat(
                &pool,
                &binding,
                "current",
                InstanceRole::Controlled,
                Backend::Rustdesk,
                None,
                &invalid_address
            )
            .await,
            Err(AppError::BadRequest(_))
        ));
        let before = db::get_device(&pool, &binding.device_id.to_string())
            .await
            .unwrap();
        assert_eq!(before.last_seen_at_micros, Some(1));
        assert!(before.health_at_micros.is_none());
        assert!(before.addresses_json.is_none());
        assert!(
            db::current_session(&pool, &binding, "current")
                .await
                .unwrap()
        );
        assert!(db::touch_session(&pool, &binding, "current").await.unwrap());
        db::revoke(&pool, &binding.device_id.to_string(), "admin")
            .await
            .unwrap();
        let revoked = db::get_device(&pool, &binding.device_id.to_string())
            .await
            .unwrap();
        assert!(matches!(
            record_heartbeat(
                &pool,
                &binding,
                "current",
                InstanceRole::Controlled,
                Backend::Rustdesk,
                None,
                &[]
            )
            .await,
            Err(AppError::Unauthorized)
        ));
        assert!(!db::touch_session(&pool, &binding, "current").await.unwrap());
        assert!(
            !db::current_session(&pool, &binding, "current")
                .await
                .unwrap()
        );
        let after = db::get_device(&pool, &binding.device_id.to_string())
            .await
            .unwrap();
        assert_eq!(revoked.last_seen_at_micros, after.last_seen_at_micros);
        assert_eq!(revoked.health_at_micros, after.health_at_micros);
    }

    #[tokio::test]
    async fn controller_heartbeat_never_advertises_host_addresses_or_wrong_backend() {
        let (pool, binding) = session(InstanceRole::Controller).await;
        let addresses = [HostAddress {
            host: "host.example.com".into(),
            port: 21118,
        }];
        assert!(matches!(
            record_heartbeat(
                &pool,
                &binding,
                "current",
                InstanceRole::Controller,
                Backend::Rustdesk,
                None,
                &addresses
            )
            .await,
            Err(AppError::BadRequest(_))
        ));
        let snapshot = DiagnosticSnapshot::awaiting_integration();
        assert!(matches!(
            record_heartbeat(
                &pool,
                &binding,
                "current",
                InstanceRole::Controller,
                Backend::OneKvm,
                Some(&snapshot),
                &[]
            )
            .await,
            Err(AppError::BadRequest(_))
        ));
        record_heartbeat(
            &pool,
            &binding,
            "current",
            InstanceRole::Controller,
            Backend::Rustdesk,
            Some(&snapshot),
            &[],
        )
        .await
        .unwrap();
        let current = db::get_device(&pool, &binding.device_id.to_string())
            .await
            .unwrap();
        assert!(current.addresses_json.is_none());
        assert!(current.addresses_at_micros.is_none());
        assert_eq!(current.health_at_micros, current.last_seen_at_micros);
    }
}
