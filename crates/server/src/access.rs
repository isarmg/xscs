//! One directed ACL, edited from either endpoint and filtered for each bound client.
use crate::{
    db::{self, Device},
    error::{AppError, AppResult},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Sqlite, SqlitePool, Transaction};
use xscs_protocol::{
    ACCESS_LEASE_MS, AccessPeer, AccessPolicy, Binding, HostAddress, InstanceRole,
    MAX_ACCESS_PEERS, validate_addresses,
};

pub fn role(device: &Device) -> AppResult<InstanceRole> {
    match device.role.as_str() {
        "controller" => Ok(InstanceRole::Controller),
        "controlled" => Ok(InstanceRole::Controlled),
        _ => Err(AppError::Internal(anyhow::anyhow!(
            "invalid stored instance role"
        ))),
    }
}
pub fn stored_addresses(device: &Device) -> AppResult<Vec<HostAddress>> {
    let addresses = match &device.addresses_json {
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
    if role(device)? == InstanceRole::Controller && !addresses.is_empty() {
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
    let device = db::get_device(pool, &binding.device_id.to_string()).await?;
    if role(&device)? == InstanceRole::Controller {
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

pub async fn policy(
    pool: &SqlitePool,
    secrets: &crate::crypto::SecretBox,
    authenticated: &Device,
    session: Option<&str>,
) -> AppResult<AccessPolicy> {
    let mut tx = pool.begin().await?;
    let own: Device = sqlx::query_as("SELECT * FROM devices WHERE device_id=?")
        .bind(&authenticated.device_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(AppError::Unauthorized)?;
    if own.revoked_at_micros.is_some()
        || own.installation_id.is_none()
        || own.installation_id != authenticated.installation_id
        || own.credential_hash.is_none()
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
    let binding_of = |device: &Device| -> AppResult<Binding> {
        Ok(Binding {
            manager_id,
            device_id: uuid::Uuid::parse_str(&device.device_id)
                .map_err(|e| AppError::Internal(e.into()))?,
            installation_id: uuid::Uuid::parse_str(
                device
                    .installation_id
                    .as_deref()
                    .ok_or(AppError::Unauthorized)?,
            )
            .map_err(|e| AppError::Internal(e.into()))?,
        })
    };
    let rows: Vec<Device> = sqlx::query_as("SELECT d.* FROM device_access a JOIN devices d ON d.device_id=CASE WHEN a.controller_id=? THEN a.controlled_id ELSE a.controller_id END WHERE (a.controller_id=? OR a.controlled_id=?) AND d.revoked_at_micros IS NULL AND d.installation_id IS NOT NULL ORDER BY d.device_id")
        .bind(&own.device_id).bind(&own.device_id).bind(&own.device_id).fetch_all(&mut *tx).await?;
    let now = db::now_micros()?;
    let own_binding = binding_of(&own)?;
    let mut peers = Vec::with_capacity(rows.len());
    for device in rows {
        let peer_binding = binding_of(&device)?;
        let own_credential = own
            .credential_hash
            .as_deref()
            .ok_or(AppError::Unauthorized)?;
        let peer_credential = device
            .credential_hash
            .as_deref()
            .ok_or(AppError::Unauthorized)?;
        let session_key = if role(&own)? == InstanceRole::Controller {
            secrets.session_key(&own_binding, &peer_binding, own_credential, peer_credential)?
        } else {
            secrets.session_key(&peer_binding, &own_binding, peer_credential, own_credential)?
        };
        let view = device.view(now)?;
        let fresh = device
            .addresses_at_micros
            .is_some_and(|at| now >= at && now - at < 45_000_000);
        peers.push(AccessPeer {
            binding: peer_binding,
            session_key,
            name: device.name.clone(),
            role: view.role,
            online: view.client_online,
            addresses: if view.client_online && fresh {
                view.addresses
            } else {
                Vec::new()
            },
        });
    }
    let policy = AccessPolicy {
        binding: own_binding,
        role: role(&own)?,
        valid_for_ms: ACCESS_LEASE_MS,
        peers,
    };
    policy
        .validate(&policy.binding)
        .map_err(|e| AppError::Internal(e.into()))?;
    tx.commit().await?;
    Ok(policy)
}
