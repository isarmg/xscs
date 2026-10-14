//! Checks of already installed runtime state; never initializes or repairs.
use crate::db;
use xcss::admin_core::AdministratorStore as _;
fn private_runtime_directories(root: &std::path::Path) -> anyhow::Result<()> {
    xcss::state_file::PrivateStateDirectory::open(root.join("logs"))?;
    xcss::state_file::PrivateStateDirectory::open(root.join("run"))?;
    Ok(())
}

/// Validate only existing runtime inputs before acquiring process locks.
/// Database contents are checked once, after both locks are held.
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
