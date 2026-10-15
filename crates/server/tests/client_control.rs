#[path = "support/database.rs"]
mod fixture_database;
use uuid::Uuid;
use xscs::{
    crypto::SecretBox,
    db,
    operations::{OperationManager, OperationState},
};
use xscs_protocol::*;
async fn fixture() -> (tempfile::TempDir, OperationManager, String, String) {
    let dir = tempfile::tempdir().unwrap();
    let pool = fixture_database::open_or_initialize(&format!(
        "sqlite://{}",
        dir.path().join("state.db").display()
    ))
    .await
    .unwrap();
    let secrets = SecretBox::new("test", [7; 32]).unwrap();
    let ticket = db::create_device(&pool, &secrets, "RustDesk host", "admin")
        .await
        .unwrap();
    let credential = db::random_token();
    db::enroll(
        &pool,
        &ticket.device.id,
        Uuid::new_v4(),
        &ticket.token,
        &credential,
    )
    .await
    .unwrap();
    let caps = Capabilities {
        protocol: PROTOCOL.into(),
        client_version: "2.0.0-dev".into(),
        os: ClientOs::LinuxX86_64,
        backend: Backend::Rustdesk,
        execution_mode: ExecutionMode::Embedded,
        diagnostics: true,
    };
    sqlx::query("UPDATE devices SET capabilities_json=?,session_id='session' WHERE device_id=?")
        .bind(serde_json::to_string(&caps).unwrap())
        .bind(&ticket.device.id)
        .execute(&pool)
        .await
        .unwrap();
    (
        dir,
        OperationManager::new(pool, secrets),
        ticket.device.id,
        credential,
    )
}
#[tokio::test]
async fn durable_read_is_idempotent_and_final_receipt_cannot_be_rewritten() {
    let (_dir, manager, id, _) = fixture().await;
    let key = Uuid::new_v4().to_string();
    let op = manager
        .enqueue("admin", &id, &key, Command::ReadDiagnostics {})
        .await
        .unwrap();
    assert_eq!(
        manager
            .enqueue("admin", &id, &key, Command::ReadDiagnostics {})
            .await
            .unwrap()
            .operation_id,
        op.operation_id
    );
    let claimed = manager.next(&id, "session").await.unwrap().unwrap();
    assert!(manager.next(&id, "session").await.unwrap().is_none());
    let report = Report::DiagnosticsRead {
        snapshot: DiagnosticSnapshot::awaiting_integration(),
    };
    manager
        .complete(&claimed, "session", report.clone())
        .await
        .unwrap();
    manager
        .accept_replayed_result(
            &manager.store.get(&op.operation_id).await.unwrap().unwrap(),
            "session",
            report,
        )
        .await
        .unwrap();
    assert_eq!(
        manager
            .get_for_actor("admin", &op.operation_id)
            .await
            .unwrap()
            .state,
        OperationState::Succeeded
    );
    let current = manager.store.get(&op.operation_id).await.unwrap().unwrap();
    assert!(
        manager
            .accept_replayed_result(
                &current,
                "session",
                Report::Rejected {
                    reason: Rejection::RustdeskUnavailable
                }
            )
            .await
            .is_err()
    );
    assert_eq!(
        db::get_device(&manager.pool, &id)
            .await
            .unwrap()
            .view(db::now_micros().unwrap())
            .unwrap()
            .diagnostics
            .unwrap()
            .integration_state,
        IntegrationState::AwaitingIntegration
    );
}
#[tokio::test]
async fn revoked_device_cannot_claim_or_authenticate() {
    let (_dir, manager, id, credential) = fixture().await;
    manager
        .enqueue(
            "admin",
            &id,
            &Uuid::new_v4().to_string(),
            Command::ReadDiagnostics {},
        )
        .await
        .unwrap();
    db::revoke(&manager.pool, &id, "admin").await.unwrap();
    assert!(
        db::authenticate_device(&manager.pool, &credential)
            .await
            .is_err()
    );
    assert!(manager.next(&id, "session").await.unwrap().is_none());
}
#[tokio::test]
async fn authorization_rotation_clears_old_host_observations_and_credentials() {
    let (_dir, manager, id, credential) = fixture().await;
    let secrets = SecretBox::new("test", [7; 32]).unwrap();
    let code = "b".repeat(AUTHORIZATION_CODE_LENGTH);
    db::rotate_authorization(&manager.pool, &secrets, &id, &code, "admin")
        .await
        .unwrap();
    assert!(
        db::authenticate_device(&manager.pool, &credential)
            .await
            .is_err()
    );
    let view = db::get_device(&manager.pool, &id)
        .await
        .unwrap()
        .view(db::now_micros().unwrap())
        .unwrap();
    assert!(!view.registered);
    assert!(view.pairing_pending);
    assert!(view.diagnostics.is_none());
    assert!(view.capabilities.is_none());
}
#[tokio::test]
async fn replaced_session_cannot_commit_a_result() {
    let (_dir, manager, id, _) = fixture().await;
    manager
        .enqueue(
            "admin",
            &id,
            &Uuid::new_v4().to_string(),
            Command::ReadDiagnostics {},
        )
        .await
        .unwrap();
    let claimed = manager.next(&id, "session").await.unwrap().unwrap();
    sqlx::query("UPDATE devices SET session_id='replacement' WHERE device_id=?")
        .bind(&id)
        .execute(&manager.pool)
        .await
        .unwrap();
    assert!(
        manager
            .complete(
                &claimed,
                "session",
                Report::DiagnosticsRead {
                    snapshot: DiagnosticSnapshot::awaiting_integration()
                }
            )
            .await
            .is_err()
    );
}
