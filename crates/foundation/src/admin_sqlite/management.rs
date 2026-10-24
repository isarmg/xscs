use super::*;
use crate::admin_core::{
    AdministratorManagementContext, AdministratorMutation, AuditOutcome, ManagementError,
};
use std::time::{SystemTime, UNIX_EPOCH};

type Failure = ManagementError<Error>;

fn storage_error(error: sqlx::Error) -> Failure {
    if error
        .as_database_error()
        .is_some_and(|error| error.is_unique_violation())
    {
        ManagementError::Conflict
    } else {
        ManagementError::Store(Error::Sqlx(error))
    }
}

pub(super) async fn execute(
    store: &SqliteAdministratorStore,
    context: &AdministratorManagementContext,
    mutation: AdministratorMutation,
) -> Result<(), Failure> {
    context
        .validate()
        .map_err(|_| ManagementError::InvalidInput)?;
    // Serialize authorization, account checks, data and audit writes.
    // Read the clock AFTER waiting for the writer lock and password hashing.
    let mut transaction = store
        .pool
        .begin_with("BEGIN IMMEDIATE")
        .await
        .map_err(storage_error)?;
    let wall_now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| ManagementError::Store(Error::IntegerRange))?
        .as_micros();
    let now = u64::try_from(wall_now)
        .map_err(|_| ManagementError::Store(Error::IntegerRange))?
        .max(context.now_micros);
    let timestamp = to_i64(now).map_err(ManagementError::Store)?;
    let authorized: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM _common_admin_sessions s JOIN _common_administrators a ON a.administrator_id=s.administrator_id WHERE s.session_id=? AND s.administrator_id=? AND s.csrf_hash=? AND s.revoked_at_micros IS NULL AND s.idle_expires_at_micros>? AND s.absolute_expires_at_micros>? AND a.active=1 AND a.session_version=s.administrator_session_version)"
    ).bind(context.identity.session_id.as_str()).bind(context.identity.administrator_id.as_str())
        .bind(context.identity.csrf_hash.as_slice()).bind(timestamp).bind(timestamp)
        .fetch_one(&mut *transaction).await.map_err(storage_error)?;
    if !authorized {
        return Err(ManagementError::Unauthorized);
    }

    let (subject, action, revokes_sessions) = match mutation {
        AdministratorMutation::UpdateOwnAccount {
            username,
            expected_password_hash,
            password_hash,
        } => {
            crate::admin_auth::require_canonical_administrator_username(&username)
                .map_err(|_| ManagementError::InvalidInput)?;
            if let Some(hash) = &password_hash {
                crate::admin_auth::require_current_password_hash(hash)
                    .map_err(|_| ManagementError::InvalidInput)?;
            }
            let id = &context.identity.administrator_id;
            let changed = sqlx::query("UPDATE _common_administrators SET username=?, password_hash=COALESCE(?, password_hash), session_version=session_version+1, updated_at_micros=? WHERE administrator_id=? AND active=1 AND password_hash=?")
                .bind(&username).bind(password_hash).bind(timestamp).bind(id.as_str()).bind(expected_password_hash)
                .execute(&mut *transaction).await.map_err(storage_error)?.rows_affected();
            if changed != 1 {
                return Err(ManagementError::Unauthorized);
            }
            revoke_administrator_sessions(&mut transaction, id, now)
                .await
                .map_err(ManagementError::Store)?;
            (username, SecurityAction::AdministratorAccountUpdated, true)
        }
    };
    let primary_event = event(context, &subject, action, now)?;
    insert_audit(&mut transaction, &primary_event)
        .await
        .map_err(ManagementError::Store)?;
    if revokes_sessions {
        let event = event(
            context,
            &subject,
            SecurityAction::AdministratorSessionsRevoked,
            now,
        )?;
        insert_audit(&mut transaction, &event)
            .await
            .map_err(ManagementError::Store)?;
    }
    transaction.commit().await.map_err(storage_error)
}

fn event(
    context: &AdministratorManagementContext,
    subject: &str,
    action: SecurityAction,
    now: u64,
) -> Result<SecurityAuditEvent, Failure> {
    let token = crate::admin_auth::random_token()
        .map_err(|error| ManagementError::Store(Error::Authentication(error)))?;
    Ok(SecurityAuditEvent {
        event_id: Identifier::new(token)
            .map_err(|error| ManagementError::Store(Error::Core(error)))?,
        action,
        outcome: AuditOutcome::Success,
        actor_administrator_id: Some(context.identity.administrator_id.clone()),
        subject_digest: Some(crate::admin_auth::token_hash(subject)),
        request_id: context.request_id.clone(),
        detail_json: "{}".into(),
        occurred_at_micros: now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::admin_core::{AdministratorService, LoginContext};

    struct Fixture {
        _directory: tempfile::TempDir,
        service: AdministratorService<SqliteAdministratorStore>,
        context: AdministratorManagementContext,
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros()
            .try_into()
            .unwrap()
    }

    async fn fixture() -> Fixture {
        let directory = tempfile::tempdir().unwrap();
        let pool = crate::sqlite::create_if_missing(
            directory.path().join("admin.sqlite3"),
            crate::sqlite::PoolOptions::new(4),
        )
        .await
        .unwrap();
        sqlx::raw_sql(ADMIN_PERSISTENT_DDL)
            .execute(&pool)
            .await
            .unwrap();
        let service = AdministratorService::new(SqliteAdministratorStore::new(pool));
        crate::testkit::initialize_administrator_fixture(
            service.store().pool(),
            "admin",
            "correct horse battery",
            now(),
        )
        .await;
        let login = service
            .login(
                "admin",
                "correct horse battery",
                &LoginContext {
                    request_id: None,
                    now_micros: now(),
                },
            )
            .await
            .unwrap();
        let identity = service
            .authenticate_session(&login.session_token, now())
            .await
            .unwrap();
        Fixture {
            _directory: directory,
            service,
            context: AdministratorManagementContext {
                identity,
                request_id: Some("management-test".into()),
                now_micros: now(),
            },
        }
    }

    async fn create(fixture: &Fixture, name: &str) -> AdministratorRecord {
        sqlx::query("INSERT INTO _common_administrators(administrator_id, username, password_hash, active, session_version, created_at_micros, updated_at_micros) SELECT ?, ?, password_hash, 1, 1, created_at_micros+1, updated_at_micros FROM _common_administrators WHERE username='admin'")
            .bind(format!("fixture-{name}")).bind(name).execute(fixture.service.store().pool()).await.unwrap();
        fixture
            .service
            .store()
            .administrator_by_username(name)
            .await
            .unwrap()
            .unwrap()
    }

    async fn logical_snapshot(pool: &SqlitePool) -> Vec<Vec<Vec<String>>> {
        let mut snapshot = Vec::new();
        for table in [
            "_common_administrators",
            "_common_admin_sessions",
            "_common_security_audit_events",
        ] {
            let columns: Vec<String> =
                sqlx::query_scalar("SELECT name FROM pragma_table_info(?) ORDER BY cid")
                    .bind(table)
                    .fetch_all(pool)
                    .await
                    .unwrap();
            let sql = format!(
                "SELECT {} FROM {table} ORDER BY 1",
                columns
                    .iter()
                    .map(|name| format!("quote({name})"))
                    .collect::<Vec<_>>()
                    .join(",")
            );
            // Test-owned schema initialized from the fixed xcss DDL;
            // table and identifier names cannot come from a submitted value.
            let rows = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
                .fetch_all(pool)
                .await
                .unwrap();
            snapshot.push(
                rows.iter()
                    .map(|row| {
                        (0..columns.len())
                            .map(|index| row.get::<String, _>(index))
                            .collect()
                    })
                    .collect(),
            );
        }
        snapshot
    }

    #[tokio::test]
    async fn validation_rejects_noncurrent_accounts_without_writes() {
        let f = fixture().await;
        let store = f.service.store();
        let before = logical_snapshot(store.pool()).await;
        store.validate_all_administrators().await.unwrap();
        assert_eq!(before, logical_snapshot(store.pool()).await);
        for statement in [
            "UPDATE _common_administrators SET active=0",
            "UPDATE _common_administrators SET active=1, password_hash='invalid-phc'",
        ] {
            sqlx::query(statement).execute(store.pool()).await.unwrap();
            let before = logical_snapshot(store.pool()).await;
            assert!(store.validate_all_administrators().await.is_err());
            assert_eq!(before, logical_snapshot(store.pool()).await);
        }
        let f = fixture().await;
        create(&f, "secondary").await;
        let before = logical_snapshot(f.service.store().pool()).await;
        assert!(matches!(
            f.service.store().validate_all_administrators().await,
            Err(Error::ExpectedSingleActiveAdministrator)
        ));
        assert_eq!(before, logical_snapshot(f.service.store().pool()).await);
        let empty = crate::sqlite::create_if_missing(
            f._directory.path().join("empty.sqlite3"),
            crate::sqlite::PoolOptions::new(2),
        )
        .await
        .unwrap();
        sqlx::raw_sql(ADMIN_PERSISTENT_DDL)
            .execute(&empty)
            .await
            .unwrap();
        let service = AdministratorService::new(SqliteAdministratorStore::new(empty));
        let before = logical_snapshot(service.store().pool()).await;
        assert!(service.store().validate_all_administrators().await.is_err());
        assert_eq!(before, logical_snapshot(service.store().pool()).await);
        crate::testkit::initialize_administrator_fixture(
            service.store().pool(),
            "admin",
            "correct horse battery",
            now(),
        )
        .await;
        service.store().validate_all_administrators().await.unwrap();
    }

    #[tokio::test]
    async fn random_csrf_sessions_are_rejected_by_every_authentication_entry() {
        let f = fixture().await;
        let login = f
            .service
            .login(
                "admin",
                "correct horse battery",
                &LoginContext {
                    request_id: None,
                    now_micros: now(),
                },
            )
            .await
            .unwrap();
        let random = crate::admin_auth::random_token().unwrap();
        sqlx::query("UPDATE _common_admin_sessions SET csrf_hash=? WHERE token_hash=?")
            .bind(crate::admin_auth::token_hash(&random).as_slice())
            .bind(crate::admin_auth::token_hash(&login.session_token).as_slice())
            .execute(f.service.store().pool())
            .await
            .unwrap();
        let before = logical_snapshot(f.service.store().pool()).await;
        assert!(matches!(
            f.service.restore_session(&login.session_token, now()).await,
            Err(crate::admin_core::ServiceError::InvalidSession)
        ));
        assert!(matches!(
            f.service
                .authenticate_session(&login.session_token, now())
                .await,
            Err(crate::admin_core::ServiceError::InvalidSession)
        ));
        assert!(matches!(
            f.service
                .require_csrf(&login.session_token, &[random.into_bytes()], now())
                .await,
            Err(crate::admin_core::ServiceError::InvalidSession)
        ));
        assert!(matches!(
            f.service.logout(&login.session_token, now(), None).await,
            Err(crate::admin_core::ServiceError::InvalidSession)
        ));
        assert_eq!(before, logical_snapshot(f.service.store().pool()).await);
    }

    #[tokio::test]
    async fn login_waits_for_writer_then_revalidates_account() {
        let f = fixture().await;
        let store = f.service.store();
        let row = sqlx::query("SELECT session_id, administrator_id, token_hash, csrf_hash, administrator_session_version, created_at_micros, last_seen_at_micros, idle_expires_at_micros, absolute_expires_at_micros, revoked_at_micros FROM _common_admin_sessions LIMIT 1")
            .fetch_one(store.pool()).await.unwrap();
        let mut session = session_from_row(&row, 0).unwrap();
        session.session_id = Identifier::new("queued-login").unwrap();
        session.token_hash = [93; 32];
        let login = crate::admin_core::LoginSuccess {
            session,
            session_created_event: event(
                &f.context,
                "admin",
                SecurityAction::SessionCreated,
                now(),
            )
            .unwrap(),
            login_succeeded_event: event(
                &f.context,
                "admin",
                SecurityAction::LoginSucceeded,
                now(),
            )
            .unwrap(),
        };
        let mut writer = store.pool().begin_with("BEGIN IMMEDIATE").await.unwrap();
        sqlx::query("UPDATE _common_administrators SET session_version=session_version+1")
            .execute(&mut *writer)
            .await
            .unwrap();
        let mut pending = std::pin::pin!(store.commit_login_success(login));
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(50), &mut pending)
                .await
                .is_err()
        );
        writer.commit().await.unwrap();
        assert!(matches!(
            pending.await,
            Err(Error::AdministratorNotEligible)
        ));
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _common_admin_sessions")
            .fetch_one(store.pool())
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn session_cleanup_is_bounded_and_preserves_active_sessions_and_audit() {
        let f = fixture().await;
        let store = f.service.store();
        for index in 0..3 {
            sqlx::query("INSERT INTO _common_admin_sessions SELECT ?,administrator_id,?,csrf_hash,administrator_session_version,created_at_micros,last_seen_at_micros,idle_expires_at_micros,absolute_expires_at_micros,last_seen_at_micros FROM _common_admin_sessions LIMIT 1")
                .bind(format!("expired-{index}")).bind(vec![index; 32]).execute(store.pool()).await.unwrap();
        }
        let audits = logical_snapshot(store.pool()).await.pop().unwrap();
        // The scan includes the first, active row, so at most one row is deleted.
        assert_eq!(store.prune_inactive_sessions(now(), 2).await.unwrap(), 1);
        assert_eq!(store.prune_inactive_sessions(now(), 128).await.unwrap(), 2);
        assert_eq!(store.prune_inactive_sessions(now(), 128).await.unwrap(), 0);
        assert!(store.prune_inactive_sessions(now(), 129).await.is_err());
        assert_eq!(logical_snapshot(store.pool()).await.pop().unwrap(), audits);
        assert_eq!(logical_snapshot(store.pool()).await[1].len(), 1);
    }

    #[tokio::test]
    async fn explicit_empty_password_does_not_update_or_revoke_the_account() {
        let f = fixture().await;
        let before = logical_snapshot(f.service.store().pool()).await;
        assert!(matches!(
            f.service
                .update_own_account(&f.context, "renamed", "correct horse battery", Some(""))
                .await,
            Err(ManagementError::InvalidInput)
        ));
        assert_eq!(logical_snapshot(f.service.store().pool()).await, before);
    }

    #[tokio::test]
    async fn own_account_update_is_atomic_and_keeps_identity() {
        let f = fixture().await;
        let original = f.context.identity.administrator_id.clone();
        f.service
            .update_own_account(
                &f.context,
                "renamed",
                "correct horse battery",
                Some("updated correct password"),
            )
            .await
            .unwrap();
        assert!(
            f.service
                .store()
                .administrator_by_username("admin")
                .await
                .unwrap()
                .is_none()
        );
        let account = f
            .service
            .store()
            .administrator_by_username("renamed")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(account.administrator_id, original);
        assert!(crate::admin_auth::verify_password(
            "updated correct password",
            &account.password_hash
        ));
        let active: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _common_admin_sessions WHERE administrator_id=? AND revoked_at_micros IS NULL")
            .bind(original.as_str()).fetch_one(f.service.store().pool()).await.unwrap();
        assert_eq!(active, 0);
        assert!(matches!(
            f.service
                .update_own_account(&f.context, "again", "updated correct password", None)
                .await,
            Err(ManagementError::Unauthorized)
        ));
        let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM _common_security_audit_events WHERE action='administrator.account_updated'").fetch_one(f.service.store().pool()).await.unwrap();
        assert_eq!(events, 1);
    }

    #[tokio::test]
    async fn wrong_password_and_duplicate_name_leave_account_unchanged() {
        let f = fixture().await;
        create(&f, "secondary").await;
        assert!(matches!(
            f.service
                .update_own_account(&f.context, "renamed", "incorrect password", None)
                .await,
            Err(ManagementError::InvalidCurrentPassword)
        ));
        assert!(matches!(
            f.service
                .update_own_account(
                    &f.context,
                    "secondary",
                    "correct horse battery",
                    Some("updated correct password")
                )
                .await,
            Err(ManagementError::Conflict)
        ));
        let account = f
            .service
            .store()
            .administrator_by_username("admin")
            .await
            .unwrap()
            .unwrap();
        assert!(crate::admin_auth::verify_password(
            "correct horse battery",
            &account.password_hash
        ));
        assert_eq!(account.session_version, 1);
        f.service
            .update_own_account(&f.context, "renamed", "correct horse battery", None)
            .await
            .unwrap();
        let account = f
            .service
            .store()
            .administrator_by_username("renamed")
            .await
            .unwrap()
            .unwrap();
        assert!(crate::admin_auth::verify_password(
            "correct horse battery",
            &account.password_hash
        ));
    }

    #[tokio::test]
    async fn stale_touch_cannot_overwrite_a_changed_csrf_hash() {
        let f = fixture().await;
        let store = f.service.store();
        let id = &f.context.identity.session_id;
        let old = f.context.identity.csrf_hash;
        let current = [42; 32];
        let timestamp = now();
        sqlx::query("UPDATE _common_admin_sessions SET csrf_hash=? WHERE session_id=?")
            .bind(current.as_slice())
            .bind(id.as_str())
            .execute(store.pool())
            .await
            .unwrap();
        assert!(
            !store
                .touch_session(id, old, timestamp + 1, timestamp + 1_000_000)
                .await
                .unwrap()
        );
        assert!(
            !store
                .touch_session(id, old, timestamp + 2, timestamp + 1_000_000)
                .await
                .unwrap()
        );
        assert!(
            store
                .touch_session(id, current, timestamp - 1, timestamp + 1_000_000)
                .await
                .unwrap()
        );
        let stored: Vec<u8> =
            sqlx::query_scalar("SELECT csrf_hash FROM _common_admin_sessions WHERE session_id=?")
                .bind(id.as_str())
                .fetch_one(store.pool())
                .await
                .unwrap();
        assert_eq!(stored, current);
        assert!(matches!(
            f.service
                .update_own_account(&f.context, "admin", "correct horse battery", None)
                .await,
            Err(ManagementError::Unauthorized)
        ));
    }

    #[tokio::test]
    async fn account_audit_failure_rolls_back_password_identity_and_session_revocation() {
        let f = fixture().await;
        sqlx::raw_sql("CREATE TRIGGER reject_audit BEFORE INSERT ON _common_security_audit_events BEGIN SELECT RAISE(ABORT, 'injected audit failure'); END;")
            .execute(f.service.store().pool()).await.unwrap();
        assert!(matches!(
            f.service
                .update_own_account(
                    &f.context,
                    "changed",
                    "correct horse battery",
                    Some("updated correct password")
                )
                .await,
            Err(ManagementError::Store(_))
        ));
        let original = f
            .service
            .store()
            .administrator_by_username("admin")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(original.session_version, 1);
        assert!(crate::admin_auth::verify_password(
            "correct horse battery",
            &original.password_hash
        ));
        let revoked: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM _common_admin_sessions WHERE revoked_at_micros IS NOT NULL",
        )
        .fetch_one(f.service.store().pool())
        .await
        .unwrap();
        assert_eq!(revoked, 0);
    }

    #[tokio::test]
    async fn minute_activity_updates_can_finish_out_of_order_without_logging_out() {
        let f = fixture().await;
        let store = f.service.store();
        let id = &f.context.identity.session_id;
        let hash = f.context.identity.csrf_hash;
        let timestamp = now() + 61_000_000;
        let deadline = timestamp + crate::admin_core::SESSION_IDLE_MICROS;
        assert!(
            store
                .touch_session(id, hash, timestamp + 100, deadline + 100)
                .await
                .unwrap()
        );
        assert!(
            store
                .touch_session(id, hash, timestamp, deadline)
                .await
                .unwrap()
        );
        let row: (i64, i64) = sqlx::query_as("SELECT last_seen_at_micros, idle_expires_at_micros FROM _common_admin_sessions WHERE session_id=?").bind(id.as_str()).fetch_one(store.pool()).await.unwrap();
        assert_eq!(row, ((timestamp + 100) as i64, (deadline + 100) as i64));
    }
}
