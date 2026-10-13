//! Offline product state operations linked only by the separately signed setup helper.
use crate::{db, runtime_lock::MaintenanceLock};
use std::path::Path;
use xcss::admin_core::AdministratorStore;
use xcss::contracts::SchemaIdentity;
use xcss::offline::{CheckReport, ConfigReport};
fn load(root: &Path, config: &Path) -> anyhow::Result<crate::ServeConfig> {
    crate::ServeConfig::from_root_sources(root, config)
}
fn private_runtime_directories(root: &Path) -> anyhow::Result<()> {
    xcss::state_file::PrivateStateDirectory::open(root.join("logs"))?;
    xcss::state_file::PrivateStateDirectory::open(root.join("run"))?;
    Ok(())
}
fn report(config: &crate::ServeConfig, path: &Path) -> anyhow::Result<ConfigReport> {
    let mut state_paths = vec![
        path.to_path_buf(),
        config.data_dir.clone(),
        config.root.join("logs"),
    ];
    state_paths.sort();
    state_paths.dedup();
    let mut writable_paths = vec![
        config.data_dir.clone(),
        config.root.join("logs"),
        config.root.join("run"),
    ];
    writable_paths.sort();
    writable_paths.dedup();
    Ok(ConfigReport {
        product: "xscs".into(),
        root: config.root.clone(),
        data_dir: config.data_dir.clone(),
        state_paths,
        schema_identity: crate::database_schema::current_schema_identity(),
        bind: config.bind,
        writable_paths,
        shared_paths: Vec::new(),
    })
}
pub async fn validate_config(root: &Path, config: &Path) -> anyhow::Result<ConfigReport> {
    report(&load(root, config)?, config)
}
pub async fn check(root: &Path, config: &Path) -> anyhow::Result<CheckReport> {
    let value = load(root, config)?;
    validate_state(&value).await?;
    Ok(CheckReport {
        config: report(&value, config)?,
        initialized: true,
    })
}
fn current_time_micros() -> anyhow::Result<u64> {
    Ok(u64::try_from(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)?
            .as_micros(),
    )?)
}
pub async fn migrate(
    root: &Path,
    config: &Path,
    data_dir: &Path,
    source_schema: &SchemaIdentity,
    target_schema: &SchemaIdentity,
) -> anyhow::Result<()> {
    let expected = crate::database_schema::current_schema_identity();
    anyhow::ensure!(
        source_schema == target_schema && target_schema == &expected,
        "no explicit migration exists for this schema transition"
    );
    anyhow::ensure!(
        load(root, config)?.data_dir == data_dir,
        "target data_dir differs from the verified upgrade plan"
    );
    check(root, config).await?;
    Ok(())
}
pub async fn initialize(
    root: &Path,
    config: &Path,
    username: &str,
    password: &str,
) -> anyhow::Result<()> {
    let username = xcss::admin_auth::normalize_administrator_username(username)?;
    xcss::admin_auth::validate_password(password)?;
    let config = load(root, config)?;
    private_runtime_directories(root)?;
    xcss::server_cli::create_empty_private_directory(&config.data_dir)
        .map_err(xcss::server_cli::CliError)?;
    let directory = xcss::state_file::PrivateStateDirectory::open(&config.data_dir)?;
    xcss::server_cli::runtime_allowed(&config.data_dir).map_err(xcss::server_cli::CliError)?;
    let _maintenance = directory.try_maintenance_lock()?;
    xcss::server_cli::runtime_allowed(&config.data_dir).map_err(xcss::server_cli::CliError)?;
    let database = crate::database_schema::database_path(&config.database_url)?;
    anyhow::ensure!(
        !database.try_exists()?,
        "initialization never overwrites an existing database"
    );
    let secrets = config.initialize_secrets()?;
    let pool = db::open_or_initialize(&config.database_url).await?;
    let result = async {
        let service = xcss::admin_core::AdministratorService::new(
            xcss::admin_sqlite::SqliteAdministratorStore::new(pool.clone()),
        );
        service
            .bootstrap_administrator(&username, password, current_time_micros()?)
            .await?;
        db::require_current_runtime_state(&pool, &secrets).await?;
        xcss::sqlite::checkpoint(&pool).await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    pool.close().await;
    result
}
pub async fn reset_password(
    root: &Path,
    config: &Path,
    username: &str,
    password: &str,
) -> anyhow::Result<()> {
    let username = xcss::admin_auth::normalize_administrator_username(username)?;
    xcss::admin_auth::validate_password(password)?;
    let config = load(root, config)?;
    let directory = xcss::state_file::PrivateStateDirectory::open(&config.data_dir)?;
    xcss::server_cli::runtime_allowed(&config.data_dir).map_err(xcss::server_cli::CliError)?;
    let _maintenance = directory.try_maintenance_lock()?;
    xcss::server_cli::runtime_allowed(&config.data_dir).map_err(xcss::server_cli::CliError)?;
    validate_state(&config).await?;
    let lock = MaintenanceLock::exclusive(&config.database_url)?;
    let pool = crate::database_schema::open_validated_location(&lock.database_url()).await?;
    let service = xcss::admin_core::AdministratorService::new(
        xcss::admin_sqlite::SqliteAdministratorStore::new(pool.clone()),
    );
    let result = service
        .change_administrator_password(&username, password, current_time_micros()?)
        .await;
    pool.close().await;
    result.map_err(anyhow::Error::from)
}

/// Validate only existing runtime inputs before acquiring process locks.
/// Database contents are checked once, after both locks are held.
#[doc(hidden)]
pub fn validate_runtime_inputs(config: &crate::ServeConfig) -> anyhow::Result<()> {
    private_runtime_directories(&config.root)?;
    xcss::state_file::PrivateStateDirectory::open(&config.data_dir)?;
    config.validate_runtime_assets()?;
    config.load_secrets()?;
    crate::database_schema::validate_existing_file(&crate::database_schema::database_path(
        &config.database_url,
    )?)?;
    Ok(())
}

#[doc(hidden)]
pub async fn validate_state(
    config: &crate::ServeConfig,
) -> anyhow::Result<crate::crypto::SecretBox> {
    let secrets = config.load_secrets()?;
    config.validate_runtime_assets()?;
    crate::web_assets::verify()?;
    private_runtime_directories(&config.root)?;
    xcss::state_file::PrivateStateDirectory::open(&config.data_dir)?;
    let snapshot = crate::database_schema::validation_snapshot(
        crate::database_schema::database_path(&config.database_url)?,
        config.root.join("run"),
    )
    .await?;
    let result = async {
        crate::database_schema::validate_pool(snapshot.pool()).await?;
        db::require_current_runtime_state(snapshot.pool(), &secrets).await?;
        let administrator =
            xcss::admin_sqlite::SqliteAdministratorStore::new(snapshot.pool().clone());
        anyhow::ensure!(
            administrator.administrator_count().await? > 0,
            "administrator initialization is required"
        );
        administrator.validate_all_administrators().await?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    snapshot.close().await;
    result?;
    Ok(secrets)
}
