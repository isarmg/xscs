//! Private test fixtures built from current published DDL and credential primitives.
#![allow(dead_code)]
use sqlx::SqlitePool;
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::Path,
};
#[path = "administrator.rs"]
mod administrator;
const PRODUCT: &str = "xscs";
pub async fn initialize_empty(pool: &SqlitePool) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    let objects: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*'")
            .fetch_one(&mut *tx)
            .await?;
    anyhow::ensure!(objects == 0, "test fixture requires an empty database");
    sqlx::raw_sql(include_str!(
        "../../../../schema/generated/current_schema.sql"
    ))
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO manager_identity(singleton,manager_id) VALUES(1,?)")
        .bind(uuid::Uuid::new_v4().to_string())
        .execute(&mut *tx)
        .await?;
    let now = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_micros(),
    )?;
    xcss::platform_db::initialize_current_platform_metadata(&mut tx, "server-control-plane", now)
        .await?;
    let fingerprint = xcss::sqlite::schema_fingerprint(&mut *tx).await?;
    sqlx::query(xcss::schema_identity::PRODUCT_METADATA_INSERT_SQL)
        .bind(PRODUCT)
        .bind("v2")
        .bind(2i64)
        .bind(fingerprint)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}
pub async fn open_or_initialize(url: &str) -> anyhow::Result<SqlitePool> {
    let path = Path::new(
        url.strip_prefix("sqlite://")
            .ok_or_else(|| anyhow::anyhow!("test fixture URI must be absolute sqlite://"))?,
    );
    let created = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
    {
        Ok(file) => {
            file.sync_all()?;
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(error) => return Err(error.into()),
    };
    if !created {
        return xscs::database_schema::open_existing(url).await;
    }
    let pool = xcss::sqlite::open_existing(path, xcss::sqlite::PoolOptions::new(16)).await?;
    if let Err(error) = initialize_empty(&pool).await {
        pool.close().await;
        return Err(error);
    }
    Ok(pool)
}
pub async fn initialize_root(root: &Path) -> anyhow::Result<()> {
    let state = root.join("state");
    fs::create_dir(&state)?;
    fs::set_permissions(&state, fs::Permissions::from_mode(0o700))?;

    let path = state.join(format!("{PRODUCT}.sqlite3"));
    let pool = open_or_initialize(&format!("sqlite://{}", path.display())).await?;
    let service = xcss::admin_core::AdministratorService::new(
        xcss::admin_sqlite::SqliteAdministratorStore::new(pool.clone()),
    );
    administrator::seed_administrator(&service, "fixture-admin", "fixture-password-2026", 1)
        .await?;
    xcss::sqlite::checkpoint(&pool).await?;
    pool.close().await;
    xcss::server_secrets::initialize(
        &state,
        &serde_json::json!({"version":1,"credential_key_id":"primary","credential_key":base64::Engine::encode(&base64::engine::general_purpose::STANDARD,[9u8;32])}),
    )?;
    Ok(())
}
