#[path = "support/database.rs"]
mod fixture_database;
use std::{
    fs,
    path::Path,
    thread,
    time::{Duration, Instant},
};

use sha2::Digest;
use xscs::{database_schema, db};

async fn close_sqlite_workers(pool: &sqlx::SqlitePool) {
    let mut connections = Vec::new();
    for _ in 0..pool.size() {
        connections.push(pool.acquire().await.expect("own fixture SQLite worker"));
    }
    let closed = pool.close();
    tokio::pin!(closed);
    assert!(futures_util::poll!(&mut closed).is_pending());
    for connection in connections {
        connection
            .close()
            .await
            .expect("stop fixture SQLite worker");
    }
    closed.await;
}

fn database_url(path: &Path) -> String {
    format!("sqlite://{}", path.display())
}

fn try_directory_snapshot(directory: &Path) -> std::io::Result<Vec<(String, Vec<u8>)>> {
    let mut files = fs::read_dir(directory)?
        .map(|entry| {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let bytes = if entry.file_type()?.is_file() {
                fs::read(entry.path())?
            } else {
                Vec::new()
            };
            Ok((name, bytes))
        })
        .collect::<std::io::Result<Vec<_>>>()?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(files)
}

fn stable_directory_snapshot(directory: &Path) -> Vec<(String, Vec<u8>)> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut previous = None;
    loop {
        match try_directory_snapshot(directory) {
            Ok(current) if previous.as_ref() == Some(&current) => return current,
            Ok(current) => previous = Some(current),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => previous = None,
            Err(error) => panic!("snapshot {}: {error}", directory.display()),
        }
        assert!(
            Instant::now() < deadline,
            "SQLite directory did not reach a stable state before snapshot"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn assert_snapshot_unchanged(before: &[(String, Vec<u8>)], after: &[(String, Vec<u8>)]) {
    let summarize = |snapshot: &[(String, Vec<u8>)]| {
        snapshot
            .iter()
            .map(|(name, bytes)| {
                let digest = hex::encode(sha2::Sha256::digest(bytes));
                format!("{name}:{}:{digest}", bytes.len())
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(summarize(after), summarize(before));
}

fn assert_live_wal(snapshot: &[(String, Vec<u8>)], database: &Path) {
    let name = database.file_name().unwrap().to_string_lossy();
    for suffix in ["-wal", "-shm"] {
        let sidecar = format!("{name}{suffix}");
        assert!(
            snapshot
                .iter()
                .any(|(entry, bytes)| entry == &sidecar && !bytes.is_empty()),
            "fixture did not retain live SQLite sidecar {sidecar}"
        );
    }
}

async fn mutate_while_holding_connection(path: &Path, sql: &str) -> sqlx::SqliteConnection {
    use sqlx::Connection as _;
    let mut connection = sqlx::SqliteConnection::connect_with(
        &sqlx::sqlite::SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true),
    )
    .await
    .unwrap();
    sqlx::raw_sql("PRAGMA journal_mode=WAL; PRAGMA wal_autocheckpoint=0;")
        .execute(&mut connection)
        .await
        .unwrap();
    // Only fixture-authored SQL reaches this helper.
    sqlx::raw_sql(sqlx::AssertSqlSafe(sql.to_owned()))
        .execute(&mut connection)
        .await
        .unwrap();
    // Retain the only writer while asserting that validation leaves live WAL/SHM unchanged.
    connection
}

#[tokio::test]
async fn exact_current_schema_is_durable_and_self_identifying() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("xscs.sqlite3");
    let url = database_url(&path);

    let pool = fixture_database::open_or_initialize(&url).await.unwrap();
    assert!(db::ready(&pool).await);
    assert_eq!(
        database_schema::actual_schema_sha256(&pool).await.unwrap(),
        database_schema::SCHEMA_SHA256
    );
    let identity = xcss::sqlite::require_pool_current_schema(
        &pool,
        &database_schema::current_schema_identity(),
    )
    .await
    .unwrap();
    assert_eq!(identity.application, database_schema::APPLICATION);
    let metadata: (i64, String, String, i64, String) = sqlx::query_as(
        "SELECT singleton,application,application_version,schema_revision,schema_sha256 \
         FROM product_metadata",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        metadata,
        (
            1,
            database_schema::APPLICATION.to_string(),
            "v2".to_string(),
            database_schema::SCHEMA_REVISION,
            database_schema::SCHEMA_SHA256.to_string(),
        )
    );

    sqlx::query(
        "INSERT INTO devices(device_id,name,authorization_code_enc,created_at_micros,updated_at_micros) \
         VALUES('11111111-1111-4111-8111-111111111111','Persistent',?,1,1)",
    )
    .bind("x".repeat(80))
    .execute(&pool)
    .await
    .unwrap();
    xcss::sqlite::integrity_check(&pool).await.unwrap();
    xcss::sqlite::foreign_key_check(&pool).await.unwrap();
    close_sqlite_workers(&pool).await;

    let reopened = db::open_existing(&url).await.unwrap();
    let name: String = sqlx::query_scalar(
        "SELECT name FROM devices WHERE device_id='11111111-1111-4111-8111-111111111111'",
    )
    .fetch_one(&reopened)
    .await
    .unwrap();
    assert_eq!(name, "Persistent");
    assert!(db::ready(&reopened).await);
    close_sqlite_workers(&reopened).await;
}

#[tokio::test]
async fn empty_unknown_schema_noncurrent_version_and_drift_are_read_only_rejections() {
    {
        let directory = tempfile::tempdir().unwrap();
        let empty = directory.path().join("empty.sqlite3");
        fs::File::create(&empty).unwrap();
        let before = stable_directory_snapshot(directory.path());
        assert!(
            fixture_database::open_or_initialize(&database_url(&empty))
                .await
                .is_err()
        );
        assert_snapshot_unchanged(&before, &stable_directory_snapshot(directory.path()));
    }

    {
        let directory = tempfile::tempdir().unwrap();
        let unknown_schema = directory.path().join("unknown-schema.sqlite3");
        let connection = mutate_while_holding_connection(
            &unknown_schema,
            "CREATE TABLE unknown_data(id INTEGER PRIMARY KEY)",
        )
        .await;
        let before = stable_directory_snapshot(directory.path());
        assert_live_wal(&before, &unknown_schema);
        assert!(
            fixture_database::open_or_initialize(&database_url(&unknown_schema))
                .await
                .is_err()
        );
        assert_snapshot_unchanged(&before, &stable_directory_snapshot(directory.path()));
        sqlx::Connection::close(connection).await.unwrap();
    }

    {
        let directory = tempfile::tempdir().unwrap();
        let wrong_version = directory.path().join("wrong-version.sqlite3");
        let pool = fixture_database::open_or_initialize(&database_url(&wrong_version))
            .await
            .unwrap();
        close_sqlite_workers(&pool).await;
        drop(pool);
        let connection = mutate_while_holding_connection(
            &wrong_version,
            "UPDATE product_metadata SET application_version='0.0.0'",
        )
        .await;
        let before = stable_directory_snapshot(directory.path());
        assert_live_wal(&before, &wrong_version);
        assert!(
            fixture_database::open_or_initialize(&database_url(&wrong_version))
                .await
                .is_err()
        );
        assert_snapshot_unchanged(&before, &stable_directory_snapshot(directory.path()));
        sqlx::Connection::close(connection).await.unwrap();
    }

    {
        let directory = tempfile::tempdir().unwrap();
        let drifted = directory.path().join("drifted.sqlite3");
        let pool = fixture_database::open_or_initialize(&database_url(&drifted))
            .await
            .unwrap();
        close_sqlite_workers(&pool).await;
        drop(pool);
        let connection =
            mutate_while_holding_connection(&drifted, "CREATE TABLE unexpected(id INTEGER)").await;
        let before = stable_directory_snapshot(directory.path());
        assert_live_wal(&before, &drifted);
        assert!(
            fixture_database::open_or_initialize(&database_url(&drifted))
                .await
                .is_err()
        );
        assert_snapshot_unchanged(&before, &stable_directory_snapshot(directory.path()));
        sqlx::Connection::close(connection).await.unwrap();
    }
}

#[cfg(unix)]
#[tokio::test]
async fn database_symlinks_and_hardlinks_are_rejected_without_mutation() {
    use std::os::unix::fs::symlink;

    let directory = tempfile::tempdir().unwrap();
    let original = directory.path().join("original.sqlite3");
    let pool = fixture_database::open_or_initialize(&database_url(&original))
        .await
        .unwrap();
    close_sqlite_workers(&pool).await;

    let symbolic = directory.path().join("symbolic.sqlite3");
    symlink(&original, &symbolic).unwrap();
    let before = stable_directory_snapshot(directory.path());
    assert!(
        fixture_database::open_or_initialize(&database_url(&symbolic))
            .await
            .is_err()
    );
    assert_snapshot_unchanged(&before, &stable_directory_snapshot(directory.path()));

    fs::remove_file(&symbolic).unwrap();
    let hard = directory.path().join("hard.sqlite3");
    fs::hard_link(&original, &hard).unwrap();
    let before = stable_directory_snapshot(directory.path());
    assert!(
        fixture_database::open_or_initialize(&database_url(&hard))
            .await
            .is_err()
    );
    assert_snapshot_unchanged(&before, &stable_directory_snapshot(directory.path()));
}
