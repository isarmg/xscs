use clap::Parser;
use std::path::PathBuf;
use xscs::{
    ServeConfig, db,
    http::{WorkerState, router},
    runtime_lock::ApplicationLock,
};

#[derive(Parser)]
#[command(name = "xscs", version, about = "Sunshine management service")]
struct Cli {
    #[arg(long, default_value = "/opt/isarmg/xscs")]
    root: PathBuf,
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let config_path = cli.root.join("config/config.json");
    let result = match ServeConfig::from_root_sources(&cli.root, &config_path) {
        Ok(config) => serve_with_config(config).await,
        Err(error) => Err(error),
    };
    match result {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("xscs could not start: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn serve_with_config(config: ServeConfig) -> anyhow::Result<()> {
    initialize_logging(&config.log_level)?;
    let signals = xcss::server_runtime::ProcessSignals::install()?;
    let listeners = xcss::server_runtime::BoundListeners::bind([config.bind])?;
    let transport = xcss::server_runtime::HttpServer::new(listeners, signals);
    // Hold both locks for the complete process lifetime. The instance lock
    // rejects a second worker, while the shared maintenance lock excludes
    // exclusive state and administrator maintenance.
    xcss::server_cli::runtime_allowed(&config.data_dir).map_err(xcss::server_cli::CliError)?;
    xscs::offline::validate_runtime_inputs(&config)?;
    let directory = xcss::state_file::PrivateStateDirectory::open(&config.data_dir)?;
    xcss::server_cli::runtime_allowed(directory.path()).map_err(xcss::server_cli::CliError)?;
    let _common_lock = directory.try_instance_lock()?;
    xcss::server_cli::runtime_allowed(directory.path()).map_err(xcss::server_cli::CliError)?;
    let application_lock = ApplicationLock::acquire(&config.database_url)?;
    let secrets = xscs::offline::validate_state(&config).await?;
    enable_runtime_logging(&config.root)?;
    tracing::info!(event = "common.config.loaded");
    let pool =
        xscs::database_schema::open_validated_location(&application_lock.database_url()).await?;
    let state = WorkerState::new(pool, secrets, config.production, config.static_dir)?;
    let recovered = state.operation_manager().recover_startup().await?;
    if let Err(error) = state.operation_manager().deliver_outbox().await {
        tracing::warn!(%error, "initial audit outbox delivery failed; background retry will continue");
    }
    let health_pool = state.pool.clone();
    let operation_manager = state.operation_manager().clone();
    let audit_pool = state.pool.clone();
    let operations_pool = state.pool.clone();
    let runtime =
        xcss::server_runtime::ServerRuntime::builder(xcss::server_runtime::ProductDescriptor {
            id: "xscs".to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            common_revision: env!("XCSS_REVISION").to_owned(),
            profile: "server-control-plane".to_owned(),
            capabilities: vec![
                "embedded-web".into(),
                "admin-persistent".to_owned(),
                "server-runtime".to_owned(),
                "server-health".to_owned(),
                "durable-operations".to_owned(),
                "secret-envelope".to_owned(),
            ],
        })
        .with_schema_identity(xscs::database_schema::current_schema_identity())
        .register_metric(
            xcss::server_runtime::DiagnosticMetric::AuditBacklog,
            move || {
                let store = xcss::operations::SqliteOperationStore::new(audit_pool.clone());
                async move { store.pending_audit_count().await.ok() }
            },
        )
        .register_metric(
            xcss::server_runtime::DiagnosticMetric::OperationBacklog,
            move || {
                let store = xcss::operations::SqliteOperationStore::new(operations_pool.clone());
                async move { store.active_operation_count().await.ok() }
            },
        )
        .register_health_check(
            "database",
            xcss::server_runtime::health_check(move || {
                let pool = health_pool.clone();
                async move { db::ready(&pool).await }
            }),
        )
        .register_background_task(
            "durable-operations",
            xcss::server_runtime::TaskCriticality::Critical,
            move |shutdown| operation_manager.run_until(shutdown),
        )
        .register_background_task(
            "shutdown-log",
            xcss::server_runtime::TaskCriticality::Degrading,
            |mut shutdown| async move {
                if !*shutdown.borrow() {
                    let _ = shutdown.changed().await;
                }
                tracing::info!(event = "common.runtime.shutdown_started");
                Ok(())
            },
        )
        .build()
        .await?;
    let runtime_handle = runtime.handle();
    tracing::info!(event = "common.runtime.started");
    tracing::info!(
        bind = %config.bind,
        schema = db::SCHEMA,
        recovered_running_operations = recovered,
        "xscs ready"
    );
    runtime
        .serve(
            transport,
            router(state, runtime_handle)?.layer(axum::middleware::from_fn_with_state(
                "xscs".to_owned(),
                xcss::server_cli::service_identity_middleware,
            )),
        )
        .await?;
    tracing::info!(event = "common.runtime.stopped");
    Ok(())
}

static LOG_LAYER: std::sync::OnceLock<xcss::log::XcssStructuredLayer> = std::sync::OnceLock::new();
fn initialize_logging(log_level: &str) -> anyhow::Result<()> {
    use tracing_subscriber::{layer::SubscriberExt as _, util::SubscriberInitExt as _};
    let layer = xcss::log::XcssStructuredLayer::new("xscs")?;
    LOG_LAYER
        .set(layer.clone())
        .map_err(|_| anyhow::anyhow!("logging already initialized"))?;
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_new(log_level)?)
        .with(layer)
        .try_init()?;
    Ok(())
}
fn enable_runtime_logging(root: &std::path::Path) -> anyhow::Result<()> {
    xcss::state_file::PrivateStateDirectory::open(root.join("logs"))?;
    let file = xcss::log::RotatingLogFile::open(
        root.join("logs"),
        "xscs",
        xcss::log::LogRetention::default(),
    )?;
    LOG_LAYER
        .get()
        .ok_or_else(|| anyhow::anyhow!("logging is unavailable"))?
        .set_rotating_file(file)?;
    Ok(())
}
