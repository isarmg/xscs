use std::{
    fs::{self, File, OpenOptions},
    path::{Component, Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, ensure};
use sqlx::{Connection, SqliteConnection, SqlitePool, sqlite::SqliteConnectOptions};
use xcss::schema_identity::SchemaIdentity;
use xcss::sqlite::PoolOptions;

#[cfg(unix)]
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};

pub const APPLICATION: &str = "xscs";
pub const APPLICATION_VERSION: &str = env!("CARGO_PKG_VERSION");
// Persisted schema identity changes only with a data-format migration.
const SCHEMA_APPLICATION_VERSION: &str = "1.0.0";
pub const SCHEMA_REVISION: i64 = 1;
pub const SCHEMA_SHA256: &str = "b3fdff2217ea2ba4a384e3ade29cbf87a949d63392bdff3895916ff2377bd1ba";

const CURRENT_SCHEMA_SQL: &str = include_str!("../../../schema/generated/current_schema.sql");

pub fn current_schema_identity() -> SchemaIdentity {
    SchemaIdentity::new(
        APPLICATION,
        SCHEMA_APPLICATION_VERSION,
        u64::try_from(SCHEMA_REVISION).expect("current schema revision is non-negative"),
        SCHEMA_SHA256,
    )
    .expect("compiled xscs schema identity is valid")
}

pub async fn open_or_initialize(database_url: &str) -> anyhow::Result<SqlitePool> {
    let path = database_path(database_url)?;
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    match options.open(&path) {
        Ok(file) => initialize_created(&path, file).await,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            open_existing_path(&path).await
        }
        Err(error) => Err(error).context("create current xscs database"),
    }
}

pub async fn open_existing(database_url: &str) -> anyhow::Result<SqlitePool> {
    let path = database_path(database_url)?;
    open_existing_path(&path).await
}

/// Open a lock-anchored location after the caller inspected the source generation.
pub async fn open_validated_location(database_url: &str) -> anyhow::Result<SqlitePool> {
    let path = database_path(database_url)?;
    validate_existing_file(&path)?;
    let pool = open_pool(&path).await?;
    validate_pool(&pool).await?;
    Ok(pool)
}

async fn open_existing_path(path: &Path) -> anyhow::Result<SqlitePool> {
    validate_existing_file(path)?;
    let validation_path = path.to_path_buf();
    tokio::task::spawn_blocking(move || validate_read_only(&validation_path))
        .await
        .context("join read-only database schema validation")??;
    let pool = open_pool(path).await?;
    if let Err(error) = validate_pool(&pool).await {
        pool.close().await;
        return Err(error);
    }
    Ok(pool)
}

async fn initialize_created(path: &Path, file: File) -> anyhow::Result<SqlitePool> {
    if let Err(error) = file
        .sync_all()
        .context("synchronize new xscs database file")
    {
        drop(file);
        return fail_initialization(path, error);
    }
    if let Err(error) = sync_parent(path) {
        drop(file);
        return fail_initialization(path, error);
    }
    drop(file);
    let pool = match open_pool(path).await {
        Ok(pool) => pool,
        Err(error) => return fail_initialization(path, error),
    };
    if let Err(error) = initialize_empty(&pool).await {
        pool.close().await;
        return fail_initialization(path, error);
    }
    if let Err(error) = checkpoint_and_sync(&pool, path).await {
        pool.close().await;
        return fail_initialization(path, error);
    }
    Ok(pool)
}

fn fail_initialization<T>(path: &Path, error: anyhow::Error) -> anyhow::Result<T> {
    if let Err(cleanup_error) = cleanup_failed_initialization(path) {
        return Err(cleanup_error.context(format!(
            "current schema initialization failed and cleanup was incomplete; original error: {error:#}"
        )));
    }
    Err(error.context("initialize current xscs schema"))
}

async fn checkpoint_and_sync(pool: &SqlitePool, path: &Path) -> anyhow::Result<()> {
    xcss::sqlite::checkpoint(pool)
        .await
        .context("checkpoint initialized xscs schema")?;
    sync_file_and_parent(path)
}

async fn open_pool(path: &Path) -> anyhow::Result<SqlitePool> {
    let options = PoolOptions::new(12)
        .with_min_connections(1)
        .with_acquire_timeout(Duration::from_secs(10))
        .with_connection_limits(connection_limits());
    Ok(xcss::sqlite::open_existing(path, options).await?)
}

/// Initialize one completely empty SQLite database with the exact current
/// schema. Existing objects are never altered.
pub async fn initialize_empty(pool: &SqlitePool) -> anyhow::Result<()> {
    let mut transaction = pool.begin().await?;
    let existing: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*'")
            .fetch_one(&mut *transaction)
            .await?;
    ensure!(
        existing == 0,
        "database is not empty; initialization requires an empty database"
    );
    sqlx::raw_sql(CURRENT_SCHEMA_SQL)
        .execute(&mut *transaction)
        .await?;
    sqlx::query("INSERT INTO manager_identity(singleton,manager_id) VALUES(1,?)")
        .bind(uuid::Uuid::new_v4().to_string())
        .execute(&mut *transaction)
        .await?;
    let created_at_micros = u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_micros(),
    )?;
    xcss::platform_db::initialize_current_platform_metadata(
        &mut transaction,
        "server-control-plane",
        created_at_micros,
    )
    .await?;
    let actual = xcss::sqlite::schema_fingerprint(&mut *transaction).await?;
    ensure!(
        actual == SCHEMA_SHA256,
        "compiled current schema fingerprint mismatch: expected {SCHEMA_SHA256}, computed {actual}"
    );
    sqlx::query(
        "INSERT INTO product_metadata(\
           singleton,application,application_version,schema_revision,schema_sha256\
         ) VALUES(1,?,?,?,?)",
    )
    .bind(APPLICATION)
    .bind(SCHEMA_APPLICATION_VERSION)
    .bind(SCHEMA_REVISION)
    .bind(SCHEMA_SHA256)
    .execute(&mut *transaction)
    .await?;
    transaction.commit().await?;
    validate_pool(pool).await
}

pub async fn validate_pool(pool: &SqlitePool) -> anyhow::Result<()> {
    xcss::sqlite::require_pool_current_schema(pool, &current_schema_identity())
        .await
        .context("database is not the exact current xscs schema")?;
    xcss::platform_db::require_current_platform_metadata(pool, "server-control-plane").await?;
    Ok(())
}

pub async fn is_current(pool: &SqlitePool) -> bool {
    validate_pool(pool).await.is_ok()
}

pub async fn actual_schema_sha256(pool: &SqlitePool) -> anyhow::Result<String> {
    Ok(xcss::sqlite::schema_fingerprint(pool).await?)
}

/// The physical database admission budget includes the main file and journals.
/// Validation uses the same byte boundary, without changing the source generation.
pub const DATABASE_BYTE_BUDGET: u64 = 8 * 1024 * 1024 * 1024;

pub fn snapshot_limits() -> xcss::sqlite::SnapshotLimits {
    xcss::sqlite::SnapshotLimits {
        max_total_bytes: DATABASE_BYTE_BUDGET,
        ..Default::default()
    }
}

pub fn connection_limits() -> xcss::sqlite::ConnectionLimits {
    xcss::sqlite::ConnectionLimits::new(2 * 1024 * 1024)
}

/// Capture before the runtime opens the original SQLite connection.
pub async fn validation_snapshot(
    path: PathBuf,
) -> anyhow::Result<xcss::sqlite::ValidationSnapshotPool> {
    let snapshot = tokio::task::spawn_blocking(move || {
        xcss::sqlite::ValidationSnapshot::capture_with_limits(path, snapshot_limits())
    })
    .await
    .context("join private database validation capture")??;
    Ok(snapshot
        .into_pool_with_connection_limits(connection_limits())
        .await?)
}

fn validate_read_only(path: &Path) -> anyhow::Result<()> {
    validate_existing_file(path)?;
    let snapshot = xcss::sqlite::ValidationSnapshot::capture_with_limits(path, snapshot_limits())?;
    xcss::sqlite::block_on_sqlite_connection(async {
        let mut connection = SqliteConnection::connect_with(
            &SqliteConnectOptions::new()
                .filename(snapshot.database_path())
                .create_if_missing(false)
                .busy_timeout(Duration::from_secs(2)),
        )
        .await
        .context("open private xscs schema-validation snapshot")?;
        let result = async {
            xcss::sqlite::apply_connection_limits(&mut connection, connection_limits()).await?;
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            connection
                .lock_handle()
                .await?
                .set_progress_handler(1000, move || std::time::Instant::now() < deadline);
            sqlx::raw_sql("PRAGMA query_only=ON; PRAGMA trusted_schema=OFF;")
                .execute(&mut connection)
                .await?;
            xcss::sqlite::require_current_schema(&mut connection, &current_schema_identity())
                .await
                .context("database is not the exact current xscs schema")?;
            Ok::<_, anyhow::Error>(())
        }
        .await;
        // The private snapshot must outlive the native SQLite worker on every exit.
        let closed = connection.close().await;
        result?;
        closed.context("close private xscs validation connection")
    })
}

pub fn validate_configuration_database(database_url: &str) -> anyhow::Result<()> {
    validate_read_only(&database_path(database_url)?)
}

pub fn database_path(database_url: &str) -> anyhow::Result<PathBuf> {
    let value = database_url
        .strip_prefix("sqlite://")
        .or_else(|| database_url.strip_prefix("sqlite:"))
        .context("database URL must use the sqlite scheme")?;
    ensure!(!value.is_empty(), "SQLite database path must not be empty");
    ensure!(
        value != ":memory:",
        "in-memory database files are unsupported"
    );
    // The product derives this value from data_dir and opens it with filename(),
    // so URL punctuation is literal filename content, never query syntax.
    ensure!(!value.contains('\0'), "database path must not contain NUL");
    let path = PathBuf::from(value);
    let absolute = if path.is_absolute() {
        path
    } else {
        std::env::current_dir()
            .context("resolve database working directory")?
            .join(path)
    };
    let mut normalized = PathBuf::from("/");
    for component in absolute.components() {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(value) => normalized.push(value),
            Component::ParentDir => {
                anyhow::bail!("database path must not contain parent traversal")
            }
            Component::Prefix(_) => anyhow::bail!("database path has an unsupported prefix"),
        }
    }
    ensure!(
        normalized.file_name().is_some(),
        "database path must name a file"
    );
    Ok(normalized)
}

fn validate_existing_file(path: &Path) -> anyhow::Result<()> {
    let metadata = fs::symlink_metadata(path)
        .with_context(|| format!("database does not exist: {}", path.display()))?;
    ensure!(
        metadata.is_file() && !metadata.file_type().is_symlink(),
        "database must be a regular file"
    );
    #[cfg(unix)]
    ensure!(
        metadata.nlink() == 1,
        "database must have exactly one hard link"
    );
    Ok(())
}

fn cleanup_failed_initialization(path: &Path) -> anyhow::Result<()> {
    for suffix in ["-wal", "-shm", "-journal", ""] {
        let mut value = path.as_os_str().to_os_string();
        value.push(suffix);
        match fs::remove_file(PathBuf::from(value)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("remove failed schema initialization file"),
        }
    }
    sync_parent(path)
}

fn sync_file_and_parent(path: &Path) -> anyhow::Result<()> {
    File::open(path)?.sync_all()?;
    sync_parent(path)
}

fn sync_parent(path: &Path) -> anyhow::Result<()> {
    File::open(path.parent().unwrap_or_else(|| Path::new(".")))?.sync_all()?;
    Ok(())
}
