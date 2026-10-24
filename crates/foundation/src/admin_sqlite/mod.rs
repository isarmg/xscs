//! Transactional persistent implementation of the xcss administrator store.

mod installation_contract;
mod login_attempts;
mod management;
pub use installation_contract::installation_contract;

use crate::admin_core::{
    AdministratorRecord, AdministratorStore, DIGEST_BYTES, Identifier, LoginSuccess,
    SESSIONS_GLOBAL, SESSIONS_PER_ADMINISTRATOR, SecurityAction, SecurityAuditEvent,
    SessionAndAdministrator, SessionRecord,
};
use sqlx::{Row, Sqlite, SqliteConnection, SqlitePool, Transaction};
use std::sync::{Arc, OnceLock};
use thiserror::Error;

pub const ADMIN_PERSISTENT_DDL: &str = include_str!("../../schemas/admin/persistent-v1.sql");
pub const ADMINISTRATOR_INSERT_SQL: &str = "INSERT INTO _common_administrators(administrator_id, username, password_hash, active, session_version, created_at_micros, updated_at_micros, last_login_at_micros) VALUES(?, ?, ?, ?, ?, ?, ?, ?)";
pub const ADMINISTRATOR_PASSWORD_RESET_SQL: &str = "UPDATE _common_administrators SET password_hash=?, session_version=session_version+1, updated_at_micros=? WHERE administrator_id=?";
pub const ADMINISTRATOR_SESSIONS_REVOKE_SQL: &str = "UPDATE _common_admin_sessions SET revoked_at_micros=? WHERE administrator_id=? AND revoked_at_micros IS NULL";
pub const SECURITY_AUDIT_INSERT_SQL: &str = "INSERT INTO _common_security_audit_events(event_id, action, outcome, actor_administrator_id, subject_digest, request_id, detail_json, occurred_at_micros) VALUES(?, ?, ?, ?, ?, ?, ?, ?)";
pub const ADMINISTRATORS_QUERY_SQL: &str = "SELECT administrator_id, username, password_hash, active, session_version, created_at_micros, updated_at_micros, last_login_at_micros FROM _common_administrators ORDER BY administrator_id LIMIT 2";
pub const ADMIN_SESSIONS_QUERY_SQL: &str = "SELECT session_id, administrator_id, token_hash, csrf_hash, administrator_session_version, created_at_micros, last_seen_at_micros, idle_expires_at_micros, absolute_expires_at_micros, revoked_at_micros FROM _common_admin_sessions";

/// Validate the existing account in an already-open read-only snapshot.
/// Does not open another database, initialize tables or change persistent state.
pub async fn validate_administrators_on_connection(
    connection: &mut SqliteConnection,
) -> Result<(), Error> {
    let rows = sqlx::query(ADMINISTRATORS_QUERY_SQL)
        .fetch_all(connection)
        .await?;
    if rows.len() != crate::admin_core::ADMINISTRATORS_REQUIRED {
        return Err(Error::ExpectedSingleActiveAdministrator);
    }
    if !administrator_from_row(&rows[0])?.active {
        return Err(Error::ExpectedSingleActiveAdministrator);
    }
    Ok(())
}

/// Insert the sole administrator inside the caller's account-creation
/// transaction, preserving a supplied stable identity and password hash.
pub async fn insert_administrator_on_connection(
    connection: &mut SqliteConnection,
    administrator: &AdministratorRecord,
) -> Result<(), Error> {
    administrator.validate()?;
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _common_administrators")
        .fetch_one(&mut *connection)
        .await?;
    if count != 0 {
        return Err(Error::AdministratorAlreadyExists);
    }
    sqlx::query(ADMINISTRATOR_INSERT_SQL)
        .bind(administrator.administrator_id.as_str())
        .bind(&administrator.username)
        .bind(&administrator.password_hash)
        .bind(i64::from(administrator.active))
        .bind(to_i64(administrator.session_version)?)
        .bind(to_i64(administrator.created_at_micros)?)
        .bind(to_i64(administrator.updated_at_micros)?)
        .bind(optional_i64(administrator.last_login_at_micros)?)
        .execute(connection)
        .await?;
    Ok(())
}

#[derive(Clone, Debug)]
pub struct SqliteAdministratorStore {
    pool: SqlitePool,
    authentication: Arc<OnceLock<Arc<crate::admin_core::AuthenticationState>>>,
}

impl SqliteAdministratorStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self {
            pool,
            authentication: Arc::new(OnceLock::new()),
        }
    }
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Scan at most `limit` oldest rows and delete only revoked/expired sessions.
    /// Repeated login maintenance eventually advances past still-active rows;
    /// security audit retention is independently owned by the product.
    pub async fn prune_inactive_sessions(&self, now_micros: u64, limit: u32) -> Result<u64, Error> {
        if !(1..=128).contains(&limit) {
            return Err(Error::IntegerRange);
        }
        let now = to_i64(now_micros)?;
        let deleted = sqlx::query("DELETE FROM _common_admin_sessions WHERE rowid IN (SELECT rowid FROM _common_admin_sessions ORDER BY rowid LIMIT ?) AND (revoked_at_micros IS NOT NULL OR idle_expires_at_micros<=? OR absolute_expires_at_micros<=?)")
            .bind(i64::from(limit)).bind(now).bind(now).execute(&self.pool).await?;
        Ok(deleted.rows_affected())
    }

    /// Validates every stored administrator against the current xcss
    /// identity and password-hash contract before a server starts accepting
    /// requests.
    pub async fn validate_all_administrators(&self) -> Result<(), Error> {
        let mut connection = self.pool.acquire().await?;
        validate_administrators_on_connection(&mut connection).await
    }
}

#[async_trait::async_trait]
impl AdministratorStore for SqliteAdministratorStore {
    type StoreError = Error;

    async fn authentication_state(
        &self,
    ) -> Result<
        Arc<crate::admin_core::AuthenticationState>,
        crate::admin_core::AuthenticationStateError,
    > {
        if let Some(state) = self.authentication.get() {
            return Ok(state.clone());
        }
        let state = login_attempts::state_for_pool(&self.pool).await?;
        let _ = self.authentication.set(state.clone());
        Ok(state)
    }

    fn supports_account_updates(&self) -> bool {
        true
    }

    async fn manage_administrator(
        &self,
        context: &crate::admin_core::AdministratorManagementContext,
        mutation: crate::admin_core::AdministratorMutation,
    ) -> Result<(), crate::admin_core::ManagementError<Error>> {
        management::execute(self, context, mutation).await
    }

    async fn administrator_count(&self) -> Result<u64, Error> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _common_administrators")
            .fetch_one(&self.pool)
            .await?;
        u64::try_from(count).map_err(|_| Error::IntegerRange)
    }

    async fn administrator_by_username(
        &self,
        username: &str,
    ) -> Result<Option<AdministratorRecord>, Error> {
        crate::admin_auth::require_canonical_administrator_username(username)?;
        let row = sqlx::query(
            "SELECT administrator_id, username, password_hash, active, session_version, \
                    created_at_micros, updated_at_micros, last_login_at_micros \
             FROM _common_administrators WHERE username = ?",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|value| administrator_from_row(&value)).transpose()
    }

    async fn session_by_token_hash(
        &self,
        token_hash: [u8; DIGEST_BYTES],
    ) -> Result<Option<SessionAndAdministrator>, Error> {
        let row = sqlx::query(
            "SELECT s.session_id, s.administrator_id, s.token_hash, s.csrf_hash, \
                    s.administrator_session_version, s.created_at_micros, s.last_seen_at_micros, \
                    s.idle_expires_at_micros, s.absolute_expires_at_micros, s.revoked_at_micros, \
                    a.administrator_id, a.username, a.password_hash, a.active, a.session_version, \
                    a.created_at_micros, a.updated_at_micros, a.last_login_at_micros \
             FROM _common_admin_sessions s \
             JOIN _common_administrators a ON a.administrator_id = s.administrator_id \
             WHERE s.token_hash = ?",
        )
        .bind(token_hash.as_slice())
        .fetch_optional(&self.pool)
        .await?;
        row.map(|value| {
            Ok(SessionAndAdministrator {
                session: session_from_row(&value, 0)?,
                administrator: administrator_from_row_offset(&value, 10)?,
            })
        })
        .transpose()
    }

    async fn create_administrator(
        &self,
        administrator: AdministratorRecord,
        event: SecurityAuditEvent,
    ) -> Result<(), Error> {
        administrator.validate()?;
        require_action(&event, SecurityAction::AdministratorCreated)?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        insert_administrator_on_connection(&mut transaction, &administrator).await?;
        insert_audit(&mut transaction, &event).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn change_password(
        &self,
        administrator_id: &Identifier,
        password_hash: &str,
        now_micros: u64,
        events: [SecurityAuditEvent; 2],
    ) -> Result<(), Error> {
        crate::admin_auth::require_current_password_hash(password_hash)?;
        require_action(&events[0], SecurityAction::AdministratorPasswordChanged)?;
        require_action(&events[1], SecurityAction::AdministratorSessionsRevoked)?;
        let mut transaction = self.pool.begin().await?;
        let changed = sqlx::query(ADMINISTRATOR_PASSWORD_RESET_SQL)
            .bind(password_hash)
            .bind(to_i64(now_micros)?)
            .bind(administrator_id.as_str())
            .execute(&mut *transaction)
            .await?;
        require_changed(changed.rows_affected(), administrator_id)?;
        revoke_administrator_sessions(&mut transaction, administrator_id, now_micros).await?;
        insert_audit(&mut transaction, &events[0]).await?;
        insert_audit(&mut transaction, &events[1]).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn commit_login_success(&self, login: LoginSuccess) -> Result<(), Error> {
        login.session.validate()?;
        require_action(&login.session_created_event, SecurityAction::SessionCreated)?;
        require_action(&login.login_succeeded_event, SecurityAction::LoginSucceeded)?;
        self.prune_inactive_sessions(login.session.created_at_micros, 128)
            .await?;
        let mut transaction = self.pool.begin_with("BEGIN IMMEDIATE").await?;
        let current: Option<(i64, i64)> = sqlx::query_as(
            "SELECT active, session_version FROM _common_administrators WHERE administrator_id=?",
        )
        .bind(login.session.administrator_id.as_str())
        .fetch_optional(&mut *transaction)
        .await?;
        let Some((active, version)) = current else {
            return Err(Error::AdministratorNotFound(
                login.session.administrator_id.to_string(),
            ));
        };
        if active != 1
            || u64::try_from(version).ok() != Some(login.session.administrator_session_version)
        {
            return Err(Error::AdministratorNotEligible);
        }
        insert_session(&mut transaction, &login.session).await?;
        enforce_session_caps(
            &mut transaction,
            &login.session,
            login.session.created_at_micros,
        )
        .await?;
        sqlx::query("UPDATE _common_administrators SET last_login_at_micros=?, updated_at_micros=? WHERE administrator_id=?")
            .bind(to_i64(login.session.created_at_micros)?)
            .bind(to_i64(login.session.created_at_micros)?)
            .bind(login.session.administrator_id.as_str())
            .execute(&mut *transaction).await?;
        insert_audit(&mut transaction, &login.session_created_event).await?;
        insert_audit(&mut transaction, &login.login_succeeded_event).await?;
        transaction.commit().await?;
        Ok(())
    }

    async fn touch_session(
        &self,
        session_id: &Identifier,
        expected_csrf_hash: [u8; DIGEST_BYTES],
        now_micros: u64,
        idle_expires_at_micros: u64,
    ) -> Result<bool, Error> {
        if idle_expires_at_micros <= now_micros {
            return Ok(false);
        }
        let changed = sqlx::query(
            "UPDATE _common_admin_sessions SET last_seen_at_micros=MAX(last_seen_at_micros,?), idle_expires_at_micros=MAX(idle_expires_at_micros,?) \
             WHERE session_id=? AND revoked_at_micros IS NULL AND idle_expires_at_micros>? AND absolute_expires_at_micros>? AND csrf_hash=? AND created_at_micros<=? AND absolute_expires_at_micros>=? AND EXISTS (SELECT 1 FROM _common_administrators a WHERE a.administrator_id=_common_admin_sessions.administrator_id AND a.active=1 AND a.session_version=_common_admin_sessions.administrator_session_version)",
        )
        .bind(to_i64(now_micros)?)
        .bind(to_i64(idle_expires_at_micros)?)
        .bind(session_id.as_str())
        .bind(to_i64(now_micros)?)
        .bind(to_i64(now_micros)?)
        .bind(expected_csrf_hash.as_slice())
        .bind(to_i64(now_micros)?)
        .bind(to_i64(idle_expires_at_micros)?)
        .execute(&self.pool).await?;
        Ok(changed.rows_affected() == 1)
    }

    async fn revoke_session(
        &self,
        session_id: &Identifier,
        now_micros: u64,
        event: SecurityAuditEvent,
    ) -> Result<(), Error> {
        require_action(&event, SecurityAction::SessionRevoked)?;
        let mut transaction = self.pool.begin().await?;
        let changed = sqlx::query("UPDATE _common_admin_sessions SET revoked_at_micros=? WHERE session_id=? AND revoked_at_micros IS NULL")
            .bind(to_i64(now_micros)?).bind(session_id.as_str()).execute(&mut *transaction).await?;
        if changed.rows_affected() != 1 {
            return Err(Error::SessionNotActive);
        }
        insert_audit(&mut transaction, &event).await?;
        transaction.commit().await?;
        Ok(())
    }
}

fn administrator_from_row(row: &sqlx::sqlite::SqliteRow) -> Result<AdministratorRecord, Error> {
    administrator_from_row_offset(row, 0)
}

fn administrator_from_row_offset(
    row: &sqlx::sqlite::SqliteRow,
    offset: usize,
) -> Result<AdministratorRecord, Error> {
    let active: i64 = row.try_get(offset + 3)?;
    if !matches!(active, 0 | 1) {
        return Err(Error::InvalidStoredRecord);
    }
    let record = AdministratorRecord {
        administrator_id: Identifier::new(row.try_get::<String, _>(offset)?)?,
        username: row.try_get(offset + 1)?,
        password_hash: row.try_get(offset + 2)?,
        active: active == 1,
        session_version: from_i64(row.try_get(offset + 4)?)?,
        created_at_micros: from_i64(row.try_get(offset + 5)?)?,
        updated_at_micros: from_i64(row.try_get(offset + 6)?)?,
        last_login_at_micros: optional_u64(row.try_get(offset + 7)?)?,
    };
    record.validate()?;
    Ok(record)
}

fn session_from_row(row: &sqlx::sqlite::SqliteRow, offset: usize) -> Result<SessionRecord, Error> {
    let token: Vec<u8> = row.try_get(offset + 2)?;
    let csrf: Vec<u8> = row.try_get(offset + 3)?;
    let record = SessionRecord {
        session_id: Identifier::new(row.try_get::<String, _>(offset)?)?,
        administrator_id: Identifier::new(row.try_get::<String, _>(offset + 1)?)?,
        token_hash: token.try_into().map_err(|_| Error::InvalidStoredRecord)?,
        csrf_hash: csrf.try_into().map_err(|_| Error::InvalidStoredRecord)?,
        administrator_session_version: from_i64(row.try_get(offset + 4)?)?,
        created_at_micros: from_i64(row.try_get(offset + 5)?)?,
        last_seen_at_micros: from_i64(row.try_get(offset + 6)?)?,
        idle_expires_at_micros: from_i64(row.try_get(offset + 7)?)?,
        absolute_expires_at_micros: from_i64(row.try_get(offset + 8)?)?,
        revoked_at_micros: optional_u64(row.try_get(offset + 9)?)?,
    };
    record.validate()?;
    Ok(record)
}

async fn insert_session(
    transaction: &mut Transaction<'_, Sqlite>,
    session: &SessionRecord,
) -> Result<(), Error> {
    sqlx::query("INSERT INTO _common_admin_sessions(session_id, administrator_id, token_hash, csrf_hash, administrator_session_version, created_at_micros, last_seen_at_micros, idle_expires_at_micros, absolute_expires_at_micros, revoked_at_micros) VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)")
        .bind(session.session_id.as_str()).bind(session.administrator_id.as_str()).bind(session.token_hash.as_slice()).bind(session.csrf_hash.as_slice())
        .bind(to_i64(session.administrator_session_version)?).bind(to_i64(session.created_at_micros)?).bind(to_i64(session.last_seen_at_micros)?)
        .bind(to_i64(session.idle_expires_at_micros)?).bind(to_i64(session.absolute_expires_at_micros)?).bind(optional_i64(session.revoked_at_micros)?)
        .execute(&mut **transaction).await?;
    Ok(())
}

async fn enforce_session_caps(
    transaction: &mut Transaction<'_, Sqlite>,
    session: &SessionRecord,
    now: u64,
) -> Result<(), Error> {
    sqlx::query("UPDATE _common_admin_sessions SET revoked_at_micros=? WHERE session_id IN (SELECT session_id FROM _common_admin_sessions WHERE administrator_id=? AND revoked_at_micros IS NULL AND idle_expires_at_micros>? AND absolute_expires_at_micros>? ORDER BY created_at_micros DESC, session_id DESC LIMIT -1 OFFSET ?)")
        .bind(to_i64(now)?).bind(session.administrator_id.as_str()).bind(to_i64(now)?).bind(to_i64(now)?).bind(i64::try_from(SESSIONS_PER_ADMINISTRATOR).map_err(|_| Error::IntegerRange)?)
        .execute(&mut **transaction).await?;
    sqlx::query("UPDATE _common_admin_sessions SET revoked_at_micros=? WHERE session_id IN (SELECT session_id FROM _common_admin_sessions WHERE revoked_at_micros IS NULL AND idle_expires_at_micros>? AND absolute_expires_at_micros>? ORDER BY created_at_micros DESC, session_id DESC LIMIT -1 OFFSET ?)")
        .bind(to_i64(now)?).bind(to_i64(now)?).bind(to_i64(now)?).bind(i64::try_from(SESSIONS_GLOBAL).map_err(|_| Error::IntegerRange)?)
        .execute(&mut **transaction).await?;
    Ok(())
}

async fn revoke_administrator_sessions(
    transaction: &mut Transaction<'_, Sqlite>,
    administrator_id: &Identifier,
    now: u64,
) -> Result<(), Error> {
    sqlx::query(ADMINISTRATOR_SESSIONS_REVOKE_SQL)
        .bind(to_i64(now)?)
        .bind(administrator_id.as_str())
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

async fn insert_audit(
    transaction: &mut Transaction<'_, Sqlite>,
    event: &SecurityAuditEvent,
) -> Result<(), Error> {
    event.validate()?;
    sqlx::query(SECURITY_AUDIT_INSERT_SQL)
        .bind(event.event_id.as_str())
        .bind(event.action.as_str())
        .bind(event.outcome.as_str())
        .bind(
            event
                .actor_administrator_id
                .as_ref()
                .map(Identifier::as_str),
        )
        .bind(
            event
                .subject_digest
                .as_ref()
                .map(<[u8; DIGEST_BYTES]>::as_slice),
        )
        .bind(event.request_id.as_deref())
        .bind(&event.detail_json)
        .bind(to_i64(event.occurred_at_micros)?)
        .execute(&mut **transaction)
        .await?;
    Ok(())
}

fn require_action(event: &SecurityAuditEvent, expected: SecurityAction) -> Result<(), Error> {
    event.validate()?;
    if event.action == expected {
        Ok(())
    } else {
        Err(Error::WrongAuditAction {
            expected,
            actual: event.action,
        })
    }
}
fn require_changed(rows: u64, identifier: &Identifier) -> Result<(), Error> {
    if rows == 1 {
        Ok(())
    } else {
        Err(Error::AdministratorNotFound(identifier.to_string()))
    }
}
fn to_i64(value: u64) -> Result<i64, Error> {
    i64::try_from(value).map_err(|_| Error::IntegerRange)
}
fn from_i64(value: i64) -> Result<u64, Error> {
    u64::try_from(value).map_err(|_| Error::InvalidStoredRecord)
}
fn optional_i64(value: Option<u64>) -> Result<Option<i64>, Error> {
    value.map(to_i64).transpose()
}
fn optional_u64(value: Option<i64>) -> Result<Option<u64>, Error> {
    value.map(from_i64).transpose()
}

#[derive(Debug, Error)]
pub enum Error {
    #[error(
        "expected exactly one valid active administrator; initialize an empty store before validation"
    )]
    ExpectedSingleActiveAdministrator,
    #[error("only one administrator is allowed")]
    AdministratorAlreadyExists,
    #[error(transparent)]
    Sqlx(#[from] sqlx::Error),
    #[error(transparent)]
    Core(#[from] crate::admin_core::Error),
    #[error(transparent)]
    Authentication(#[from] crate::admin_auth::Error),
    #[error("administrator not found: {0}")]
    AdministratorNotFound(String),
    #[error("administrator is inactive or its session revision changed")]
    AdministratorNotEligible,
    #[error("session is absent, revoked, or expired")]
    SessionNotActive,
    #[error("stored administrator/session data violates the current contract")]
    InvalidStoredRecord,
    #[error("integer is outside the SQLite signed 64-bit range")]
    IntegerRange,
    #[error("audit action differs: expected {expected:?}, found {actual:?}")]
    WrongAuditAction {
        expected: SecurityAction,
        actual: SecurityAction,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin_core::{AdministratorStore, AuditOutcome};

    fn event(
        id: &str,
        action: SecurityAction,
        administrator_id: &Identifier,
    ) -> SecurityAuditEvent {
        SecurityAuditEvent {
            event_id: Identifier::new(id).unwrap(),
            action,
            outcome: AuditOutcome::Success,
            actor_administrator_id: Some(administrator_id.clone()),
            subject_digest: None,
            request_id: Some("request-1".into()),
            detail_json: "{}".into(),
            occurred_at_micros: 10,
        }
    }

    fn session(administrator_id: &Identifier, id: &str, byte: u8) -> SessionRecord {
        SessionRecord {
            session_id: Identifier::new(id).unwrap(),
            administrator_id: administrator_id.clone(),
            token_hash: [byte; 32],
            csrf_hash: [byte.wrapping_add(1); 32],
            administrator_session_version: 1,
            created_at_micros: 10,
            last_seen_at_micros: 10,
            idle_expires_at_micros: 100,
            absolute_expires_at_micros: 200,
            revoked_at_micros: None,
        }
    }

    #[tokio::test]
    async fn mutations_and_audit_are_atomic() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let pool = crate::sqlite::create_if_missing(
            directory.path().join("admin.sqlite3"),
            crate::sqlite::PoolOptions::new(1),
        )
        .await?;
        sqlx::raw_sql(ADMIN_PERSISTENT_DDL).execute(&pool).await?;
        let store = SqliteAdministratorStore::new(pool.clone());
        let administrator_id = Identifier::new("administrator-1")?;
        store
            .create_administrator(
                AdministratorRecord {
                    administrator_id: administrator_id.clone(),
                    username: "admin".into(),
                    password_hash: crate::admin_auth::hash_password("correct horse battery")?,
                    active: true,
                    session_version: 1,
                    created_at_micros: 1,
                    updated_at_micros: 1,
                    last_login_at_micros: None,
                },
                event(
                    "event-created",
                    SecurityAction::AdministratorCreated,
                    &administrator_id,
                ),
            )
            .await?;

        let rolled_back = session(&administrator_id, "session-rollback", 3);
        assert!(
            store
                .commit_login_success(LoginSuccess {
                    session: rolled_back.clone(),
                    session_created_event: event(
                        "event-created",
                        SecurityAction::SessionCreated,
                        &administrator_id
                    ),
                    login_succeeded_event: event(
                        "event-never",
                        SecurityAction::LoginSucceeded,
                        &administrator_id
                    ),
                })
                .await
                .is_err()
        );
        assert!(
            store
                .session_by_token_hash(rolled_back.token_hash)
                .await?
                .is_none()
        );

        let active = session(&administrator_id, "session-active", 7);
        store
            .commit_login_success(LoginSuccess {
                session: active.clone(),
                session_created_event: event(
                    "event-session",
                    SecurityAction::SessionCreated,
                    &administrator_id,
                ),
                login_succeeded_event: event(
                    "event-login",
                    SecurityAction::LoginSucceeded,
                    &administrator_id,
                ),
            })
            .await?;
        assert_eq!(
            store
                .session_by_token_hash(active.token_hash)
                .await?
                .unwrap()
                .session,
            active
        );
        let audit_count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM _common_security_audit_events")
                .fetch_one(&pool)
                .await?;
        assert_eq!(audit_count, 3);
        Ok(())
    }
}
