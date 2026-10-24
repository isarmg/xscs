use super::*;
use sqlx::{Executor, Row, SqliteConnection};

async fn execute(
    connection: &mut SqliteConnection,
    descriptors: &Value,
    hash: &str,
    now: i64,
) -> Result<(), sqlx::Error> {
    for descriptor in descriptors.as_array().unwrap() {
        // Statements are generated from the owned SQL constants above;
        // every candidate credential and identity is separately bound.
        let mut query = sqlx::query(sqlx::AssertSqlSafe(descriptor["sql"].as_str().unwrap()));
        // A fresh event identity belongs to each statement, rather than being
        // reused across the password and session revocation audit events.
        let event_id = admin_auth::random_token().unwrap();
        for item in descriptor["parameters"].as_array().unwrap() {
            let value = match item["source"].as_str().unwrap() {
                "username" => json!("admin"),
                "password_hash" => json!(hash),
                "administrator_id" => json!("c3RhYmxlLWFkbWluaXN0cmF0b3I"),
                "event_id" => json!(event_id),
                "now_micros" => json!(now),
                "username_sha256" => {
                    query = query.bind(Sha256::digest(b"admin").to_vec());
                    continue;
                }
                "literal"
                | "schema.application"
                | "schema.application_version"
                | "schema.schema_revision"
                | "schema.schema_sha256" => item["value"].clone(),
                other => panic!("unexpected exported source {other}"),
            };
            query = match item["type"].as_str().unwrap() {
                "text" => query.bind(value.as_str().unwrap().to_owned()),
                "integer" => query.bind(value.as_i64().unwrap()),
                "boolean" => query.bind(value.as_bool().unwrap()),
                "null" => query.bind(Option::<String>::None),
                other => panic!("unexpected exported type {other}"),
            };
        }
        let changes = query.execute(&mut *connection).await?.rows_affected();
        if let Some(expected) = descriptor["expected_changes"].as_u64() {
            assert_eq!(changes, expected);
        }
    }
    Ok(())
}

#[tokio::test]
async fn generated_statements_initialize_reset_revoke_and_preserve_identity_atomically() {
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    let mut connection = pool.acquire().await.unwrap();
    sqlx::raw_sql(admin_sqlite::ADMIN_PERSISTENT_DDL)
        .execute(&mut *connection)
        .await
        .unwrap();
    sqlx::raw_sql(crate::schema_identity::PRODUCT_METADATA_DDL)
        .execute(&mut *connection)
        .await
        .unwrap();
    sqlx::raw_sql(platform_db::PLATFORM_METADATA_DDL)
        .execute(&mut *connection)
        .await
        .unwrap();
    let schema = SchemaIdentity::new(
        "fixture",
        "fixture-db-current",
        1,
        crate::sqlite::schema_fingerprint(&mut *connection)
            .await
            .unwrap(),
    )
    .unwrap();
    let identity = ReleaseIdentity {
        product: "fixture".into(),
        version: "1.0.0".into(),
        source_revision: "a".repeat(40),
        target: "x86_64-unknown-linux-gnu".into(),
        state_contract_sha256: "b".repeat(64),
    };
    let material = common(&identity, &schema, Some("server-control-plane")).unwrap();
    let original_password = "original fixture password";
    let original_hash = admin_auth::hash_password(original_password).unwrap();
    connection.execute("BEGIN IMMEDIATE").await.unwrap();
    execute(
        &mut connection,
        &material["database"]["initialize"],
        &original_hash,
        10,
    )
    .await
    .unwrap();
    connection.execute("COMMIT").await.unwrap();
    let actual = crate::sqlite::read_schema_identity(&mut connection)
        .await
        .unwrap();
    actual.require_exact(&schema).unwrap();
    drop(connection);
    let service = admin_core::AdministratorService::new(
        admin_sqlite::SqliteAdministratorStore::new(pool.clone()),
    );
    let session = service
        .login(
            "admin",
            original_password,
            &admin_core::LoginContext {
                request_id: None,
                now_micros: 20,
            },
        )
        .await
        .unwrap();
    let mut connection = pool.acquire().await.unwrap();
    let replacement = admin_auth::hash_password("replacement fixture password").unwrap();
    // A failing audit statement must permit the caller to roll back every
    // earlier password/session mutation in the exported transaction.
    let mut invalid = material["database"]["reset_administrator"].clone();
    invalid[2]["sql"] = json!("INSERT INTO absent_audit_table(value) VALUES(?)");
    connection.execute("BEGIN IMMEDIATE").await.unwrap();
    assert!(
        execute(&mut connection, &invalid, &replacement, 30)
            .await
            .is_err()
    );
    connection.execute("ROLLBACK").await.unwrap();
    let stored: String = sqlx::query_scalar("SELECT password_hash FROM _common_administrators")
        .fetch_one(&mut *connection)
        .await
        .unwrap();
    assert_eq!(stored, original_hash);
    connection.execute("BEGIN IMMEDIATE").await.unwrap();
    execute(
        &mut connection,
        &material["database"]["reset_administrator"],
        &replacement,
        30,
    )
    .await
    .unwrap();
    connection.execute("COMMIT").await.unwrap();
    let row = sqlx::query(admin_sqlite::ADMINISTRATORS_QUERY_SQL)
        .fetch_one(&mut *connection)
        .await
        .unwrap();
    assert_eq!(
        row.get::<String, _>("administrator_id"),
        "c3RhYmxlLWFkbWluaXN0cmF0b3I"
    );
    assert_eq!(row.get::<i64, _>("session_version"), 2);
    assert!(admin_auth::verify_password(
        "replacement fixture password",
        &row.get::<String, _>("password_hash")
    ));
    let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM _common_security_audit_events WHERE action IN ('administrator.password_changed','administrator.sessions_revoked')").fetch_one(&mut *connection).await.unwrap();
    assert_eq!(count, 2);
    drop(connection);
    assert!(
        service
            .authenticate_session(&session.session_token, 31)
            .await
            .is_err()
    );
    pool.close().await;
}

#[test]
fn unsupported_profile_and_unsafe_schema_integer_are_rejected() {
    let identity = ReleaseIdentity {
        product: "fixture".into(),
        version: "1.0.0".into(),
        source_revision: "a".repeat(40),
        target: "x86_64-unknown-linux-gnu".into(),
        state_contract_sha256: "b".repeat(64),
    };
    let mut schema = SchemaIdentity::new("fixture", "current", 1, "a".repeat(64)).unwrap();
    assert!(common(&identity, &schema, Some("retired-profile")).is_err());
    schema.schema_revision = crate::contracts::MAX_SAFE_JSON_INTEGER + 1;
    assert!(common(&identity, &schema, None).is_err());
}
