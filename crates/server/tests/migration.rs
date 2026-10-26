//! Explicit offline migration preserves real native credentials and durable requests.
#[path = "support/database.rs"]
mod fixture_database;
use std::{path::Path, process::Command};
use xscs::{access, crypto::SecretBox, db, operations::OperationManager};
use xscs_protocol::{
    Backend, Capabilities, ClientOs, Command as DeviceCommand, ExecutionMode, InstanceRole,
    PROTOCOL,
};

async fn close_workers(pool: &sqlx::SqlitePool) {
    let mut connections = Vec::new();
    for _ in 0..pool.size() {
        connections.push(pool.acquire().await.unwrap());
    }
    for connection in connections {
        connection.close().await.unwrap();
    }
    pool.close().await;
}

#[tokio::test]
async fn offline_v3_migration_keeps_native_credentials_grants_tasks_and_audit() {
    let directory = tempfile::tempdir().unwrap();
    let database = directory.path().join("xscs.sqlite3");
    let url = format!("sqlite://{}", database.display());
    let pool = fixture_database::open_or_initialize(&url).await.unwrap();
    let secrets = SecretBox::new("primary", [7; 32]).unwrap();
    let mut clients = Vec::new();
    for (name, role) in [
        ("Native controller", InstanceRole::Controller),
        ("Native host", InstanceRole::Controlled),
    ] {
        let ticket = db::create_device(&pool, &secrets, name, role, "admin")
            .await
            .unwrap();
        let credential = db::random_token();
        let binding = db::enroll(
            &pool,
            &ticket.device.id,
            uuid::Uuid::new_v4(),
            &ticket.token,
            &credential,
        )
        .await
        .unwrap();
        clients.push((ticket, credential, binding));
    }
    let (controller, controller_token, _) = &clients[0];
    let (host, _, _) = &clients[1];
    let rules = access::rules(&pool, &controller.device.id).await.unwrap();
    access::replace_rules(
        &pool,
        &controller.device.id,
        access::UpdateAccessRules {
            peer_ids: vec![host.device.id.clone()],
            expected_revision: rules.revision,
        },
        "admin",
    )
    .await
    .unwrap();
    let capabilities = Capabilities {
        protocol: PROTOCOL.into(),
        client_version: "migration-fixture".into(),
        os: ClientOs::LinuxX86_64,
        backend: Backend::Rustdesk,
        execution_mode: ExecutionMode::Embedded,
        diagnostics: true,
    };
    sqlx::query("UPDATE devices SET capabilities_json=? WHERE device_id=?")
        .bind(serde_json::to_string(&capabilities).unwrap())
        .bind(&host.device.id)
        .execute(&pool)
        .await
        .unwrap();
    let manager = OperationManager::new(pool.clone(), secrets.clone());
    let operation = manager
        .enqueue(
            "admin",
            &host.device.id,
            "retained-task",
            DeviceCommand::ReadDiagnostics {},
        )
        .await
        .unwrap();
    let payload: Vec<u8> =
        sqlx::query_scalar("SELECT request_payload FROM _common_operations WHERE operation_id=?")
            .bind(&operation.operation_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    let audits: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM audit_logs")
        .fetch_one(&pool)
        .await
        .unwrap();
    let own = db::authenticate_device(&pool, controller_token)
        .await
        .unwrap();
    let previous = access::policy(&pool, &secrets, &own, None).await.unwrap();
    xcss::sqlite::checkpoint(&pool).await.unwrap();
    close_workers(&pool).await;
    drop(manager);

    let product = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let downgrade = r#"
import sqlite3,sys
from pathlib import Path
schema=sqlite3.connect(':memory:');schema.executescript(Path(sys.argv[2]).read_text())
table=schema.execute("SELECT sql FROM sqlite_schema WHERE type='table' AND name='devices'").fetchone()[0]
triggers=schema.execute("SELECT sql FROM sqlite_schema WHERE type='trigger' AND tbl_name='devices'").fetchall()
con=sqlite3.connect(sys.argv[1]);con.execute('PRAGMA foreign_keys=OFF');con.execute('BEGIN IMMEDIATE')
columns=','.join('"'+r[1]+'"' for r in con.execute('PRAGMA table_info(devices)') if r[1]!='backend')
con.execute('CREATE TEMP TABLE retained_devices AS SELECT '+columns+' FROM devices');con.execute('DROP TABLE devices');con.execute(table)
con.execute('INSERT INTO devices('+columns+') SELECT '+columns+' FROM retained_devices')
for (sql,) in triggers:con.execute(sql)
con.execute("UPDATE product_metadata SET application_version='v3',schema_revision=3,schema_sha256='06af04920d4709289c0194f2d9c972dec19a703495bef08d06e3b94b6ee3c676'")
con.commit();con.execute('PRAGMA wal_checkpoint(TRUNCATE)');con.close()
"#;
    let prepared = Command::new("python3")
        .args(["-I", "-c", downgrade])
        .arg(&database)
        .arg(product.join("schema/legacy/v3.sql"))
        .output()
        .unwrap();
    assert!(
        prepared.status.success(),
        "{}",
        String::from_utf8_lossy(&prepared.stderr)
    );
    assert!(xscs::database_schema::open_existing(&url).await.is_err());
    let backup = directory.path().join("v3-backup.sqlite3");
    let migration = Command::new("python3")
        .arg("-I")
        .arg(product.join("scripts/migrate-v3-v4.py"))
        .arg("--database")
        .arg(&database)
        .arg("--backup")
        .arg(&backup)
        .output()
        .unwrap();
    assert!(
        migration.status.success(),
        "{}",
        String::from_utf8_lossy(&migration.stderr)
    );
    let reopened = xscs::database_schema::open_existing(&url).await.unwrap();
    db::require_current_runtime_state(&reopened, &secrets)
        .await
        .unwrap();
    for (ticket, credential, binding) in &clients {
        let device = db::authenticate_device(&reopened, credential)
            .await
            .unwrap();
        assert_eq!(db::binding(&reopened, &device).await.unwrap(), *binding);
        assert_eq!(device.backend().unwrap(), Backend::Rustdesk);
        assert_eq!(
            db::get_authorization(&reopened, &secrets, &ticket.device.id)
                .await
                .unwrap(),
            ticket.token
        );
    }
    let current = access::policy(
        &reopened,
        &secrets,
        &db::authenticate_device(&reopened, controller_token)
            .await
            .unwrap(),
        None,
    )
    .await
    .unwrap();
    assert_eq!(current, previous);
    let retained: Vec<u8> =
        sqlx::query_scalar("SELECT request_payload FROM _common_operations WHERE operation_id=?")
            .bind(&operation.operation_id)
            .fetch_one(&reopened)
            .await
            .unwrap();
    assert_eq!(retained, payload);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM audit_logs")
            .fetch_one(&reopened)
            .await
            .unwrap(),
        audits
    );
    close_workers(&reopened).await;
}
