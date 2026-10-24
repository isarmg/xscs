//! Common CLI behavior, with product commands and business initialization
//! deliberately outside the crate. Machine output contains a single record.

pub mod http;
pub use http::{ContractJson, ContractPath, ContractQuery, request_context_middleware};

use axum::{
    extract::{Request, State},
    http::HeaderValue,
    middleware::Next,
    response::Response,
};
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Write},
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    path::Path,
    process::ExitCode,
    time::Duration,
};

pub use crate::error::{ErrorCode, ErrorEnvelope, HttpStatus};
pub use crate::sqlite::{
    SnapshotError, SnapshotLimits, ValidationSnapshot, ValidationSnapshotPool,
    open_validation_snapshot, open_validation_snapshot_with_connection_limits,
};
pub use crate::state_file::INSTANCE_LOCK_FILE;

pub const SERVICE_IDENTITY_HEADER: &str = "x-service";
pub const STATUS_TIMEOUT: Duration = Duration::from_secs(3);
pub const MAX_STATUS_BYTES: usize = 4096;

/// Preserve a public machine error while crossing a product's anyhow boundary.
/// Display intentionally excludes details and any internal diagnostic chain.
#[derive(Debug)]
pub struct CliError(pub ErrorEnvelope);

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}: {}", self.0.code, self.0.message)
    }
}
impl std::error::Error for CliError {}

fn failure(code: &'static str, message: &'static str) -> ErrorEnvelope {
    ErrorEnvelope::with_code(
        ErrorCode::new(code).expect("static public error code"),
        message,
    )
}

/// Preserve public state-file failure semantics without exposing paths, owners,
/// permission bits or the underlying operating-system diagnostic text.
pub fn state_error(error: &crate::state_file::Error) -> ErrorEnvelope {
    use crate::state_file::Error;
    match error {
        Error::LockUnavailable { source, .. } if source.kind() == io::ErrorKind::WouldBlock => {
            failure(
                "state.lock_busy",
                "The data directory is in use by another runtime.",
            )
            .retryable(true)
        }
        Error::StateDirectoryNotAbsolute { .. }
        | Error::InvalidChildName { .. }
        | Error::NotDirectory { .. }
        | Error::NotRegularFile { .. }
        | Error::UnexpectedHardLinkCount { .. } => failure(
            "state.unsafe_path",
            "The state path does not satisfy the private directory and file contract.",
        ),
        Error::UnexpectedOwner { .. } => failure(
            "state.owner_mismatch",
            "The state path is not owned by the current process user.",
        ),
        Error::UnexpectedMode { .. } => failure(
            "state.invalid_permissions",
            "The state path does not have the required private permissions.",
        ),
        Error::FileIdentityChanged { .. } => failure(
            "state.identity_changed",
            "The state path changed identity during this operation.",
        ),
        Error::InspectPath { source, .. }
        | Error::CreateDirectory { source, .. }
        | Error::OpenFile { source, .. }
        | Error::LockUnavailable { source, .. } => state_io_error(source),
    }
}

fn state_io_error(error: &io::Error) -> ErrorEnvelope {
    match error.kind() {
        io::ErrorKind::NotFound => {
            failure("state.not_found", "The required state path does not exist.")
        }
        io::ErrorKind::PermissionDenied => failure(
            "state.permission_denied",
            "The current process cannot access the required state path.",
        ),
        _ => failure(
            "state.io_failed",
            "The state operation failed while accessing the filesystem.",
        ),
    }
}

/// Public SQLite diagnostic failures do not include filenames, SQL text or
/// adapter error chains. Only transient capture failures are retryable.
pub fn snapshot_error(error: &SnapshotError) -> ErrorEnvelope {
    let (code, message, retryable) = match error {
        SnapshotError::InvalidLimits | SnapshotError::ConnectionLimits(_) => (
            "snapshot.invalid_limits",
            "The validation snapshot limits are invalid.",
            false,
        ),
        SnapshotError::UnsafeSource => (
            "snapshot.unsafe_source",
            "The validation source does not satisfy the private file contract.",
            false,
        ),
        SnapshotError::MissingDatabase => (
            "snapshot.not_found",
            "The validation database does not exist.",
            false,
        ),
        SnapshotError::InvalidDatabase | SnapshotError::Sqlx(_) => (
            "snapshot.invalid_database",
            "The validation database cannot satisfy the current database contract.",
            false,
        ),
        SnapshotError::SourceChanged => (
            "snapshot.source_changed",
            "The validation source changed; retry after writes stop.",
            true,
        ),
        SnapshotError::BudgetExceeded => (
            "snapshot.budget_exceeded",
            "The validation snapshot exceeds its byte budget.",
            false,
        ),
        SnapshotError::Timeout => (
            "snapshot.timeout",
            "The validation snapshot exceeded its time budget.",
            true,
        ),
        SnapshotError::Busy => (
            "snapshot.busy",
            "The validation source has an active writer, checkpoint or recovery.",
            true,
        ),
        SnapshotError::LockUnavailable(_) => (
            "snapshot.lock_unavailable",
            "The validation source does not support the required native locking.",
            false,
        ),
        SnapshotError::CaptureTaskFailed => (
            "snapshot.capture_failed",
            "The validation capture task failed.",
            false,
        ),
        SnapshotError::Io(_) => (
            "snapshot.io_failed",
            "The validation snapshot could not access its filesystem input.",
            false,
        ),
    };
    failure(code, message).retryable(retryable)
}

/// Response identity comes from the product that owns this router. Validate
/// the service name before installing the middleware; invalid names fail
/// closed because a status caller requires the header to match exactly.
pub async fn service_identity_middleware(
    State(service): State<String>,
    request: Request,
    next: Next,
) -> Response {
    let mut response = next.run(request).await;
    if let Ok(value) = HeaderValue::from_str(&service) {
        response
            .headers_mut()
            .insert(SERVICE_IDENTITY_HEADER, value);
    }
    response
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessState {
    Ready,
    NotReady,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StatusReport {
    pub service: String,
    pub state: ReadinessState,
    pub ready: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReadinessResponse {
    ready: bool,
}

/// Probe actual business readiness with a bounded, proxy-free request. An
/// unspecified listening address means loopback for the local diagnostics
/// request. Redirects and another service's response are rejected.
pub async fn query_status(
    mut bind: SocketAddr,
    expected_service: &str,
) -> Result<StatusReport, ErrorEnvelope> {
    if expected_service.is_empty()
        || !expected_service
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(failure(
            "invalid_input",
            "A current service identity is required.",
        ));
    }
    if bind.ip().is_unspecified() {
        bind.set_ip(match bind.ip() {
            IpAddr::V4(_) => IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V6(_) => IpAddr::V6(Ipv6Addr::LOCALHOST),
        });
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(STATUS_TIMEOUT)
        .build()
        .map_err(|_| {
            failure(
                "dependency_unavailable",
                "Cannot create the readiness client.",
            )
        })?;
    let mut response = client
        .get(format!("http://{bind}/readyz"))
        .send()
        .await
        .map_err(|_| {
            failure(
                "dependency_unavailable",
                "The configured service did not answer the readiness query.",
            )
            .retryable(true)
        })?;
    let status = response.status();
    if response
        .headers()
        .get(SERVICE_IDENTITY_HEADER)
        .and_then(|value| value.to_str().ok())
        != Some(expected_service)
    {
        return Err(failure(
            "service_identity_mismatch",
            "The endpoint did not confirm the configured service identity.",
        ));
    }
    if !matches!(status.as_u16(), 200 | 503)
        || response
            .content_length()
            .is_some_and(|length| length > MAX_STATUS_BYTES as u64)
    {
        return Err(failure(
            "invalid_status_response",
            "The service returned an invalid readiness response.",
        ));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        failure(
            "dependency_unavailable",
            "The readiness response was interrupted.",
        )
        .retryable(true)
    })? {
        if body.len().saturating_add(chunk.len()) > MAX_STATUS_BYTES {
            return Err(failure(
                "invalid_status_response",
                "The readiness response exceeded its size limit.",
            ));
        }
        body.extend_from_slice(&chunk);
    }
    let readiness: ReadinessResponse = serde_json::from_slice(&body).map_err(|_| {
        failure(
            "invalid_status_response",
            "The service returned an invalid readiness response.",
        )
    })?;
    if readiness.ready != (status.as_u16() == 200) {
        return Err(failure(
            "invalid_status_response",
            "Readiness HTTP status and response body disagree.",
        ));
    }
    Ok(StatusReport {
        service: expected_service.into(),
        ready: readiness.ready,
        state: if readiness.ready {
            ReadinessState::Ready
        } else {
            ReadinessState::NotReady
        },
    })
}

pub fn print_report(report: &StatusReport, json: bool) -> io::Result<()> {
    let mut output = io::stdout().lock();
    if json {
        serde_json::to_writer(&mut output, report)?;
        writeln!(output)?;
    } else {
        writeln!(
            output,
            "{}: {}",
            report.service,
            if report.ready { "ready" } else { "not ready" }
        )?;
    }
    output.flush()
}

/// Print a public error contract; diagnostics and internal error chains must
/// be mapped before calling this boundary. Success exit codes are rejected.
pub fn report_error(error: &ErrorEnvelope, json: bool, exit: u8) -> ExitCode {
    let emitted = write_error(
        error,
        json,
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    );
    if emitted.is_err() {
        let _ = writeln!(
            io::stderr().lock(),
            "Could not write the command error response."
        );
    }
    ExitCode::from(if exit == 0 { 1 } else { exit })
}

fn write_error(
    error: &ErrorEnvelope,
    json: bool,
    output: &mut impl Write,
    diagnostics: &mut impl Write,
) -> io::Result<()> {
    if json {
        serde_json::to_writer(&mut *output, error)?;
        writeln!(output)?;
        output.flush()
    } else {
        writeln!(diagnostics, "{}: {}", error.code, error.message)?;
        diagnostics.flush()
    }
}

pub fn validate_runtime_log_directory(data_dir: &Path) -> Result<(), ErrorEnvelope> {
    crate::fs_safety::PrivateDirectory::open_existing(data_dir).map_err(|_| {
        failure(
            "unsafe_state_directory",
            "The data directory must be private and current.",
        )
    })?;
    crate::fs_safety::PrivateDirectory::open_existing(data_dir.join("logs"))
        .map(|_| ())
        .map_err(|_| {
            failure(
                "unsafe_log_directory",
                "An existing private runtime log directory is required.",
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, middleware, routing::get};

    #[test]
    fn filesystem_failures_keep_machine_semantics_and_hide_diagnostics() {
        use crate::state_file::{Error, LockKind};
        let private = tempfile::tempdir().unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(private.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let directory = crate::state_file::PrivateStateDirectory::open(private.path()).unwrap();
        let missing = directory.open_existing("SECRET-path").err().unwrap();
        let busy = Error::LockUnavailable {
            kind: LockKind::Instance,
            source: io::Error::new(io::ErrorKind::WouldBlock, "SECRET-owner"),
        };
        for (error, code, retryable) in [
            (missing, "state.not_found", false),
            (busy, "state.lock_busy", true),
            (
                Error::OpenFile {
                    path: "SECRET-path".into(),
                    source: io::Error::new(io::ErrorKind::PermissionDenied, "SECRET-source"),
                },
                "state.permission_denied",
                false,
            ),
        ] {
            let error = state_error(&error);
            assert_eq!(error.code.as_str(), code);
            assert_eq!(error.retryable, retryable);
            assert!(!serde_json::to_string(&error).unwrap().contains("SECRET"));
        }
        let io = SnapshotError::Io(io::Error::other("SECRET-database"));
        assert_eq!(snapshot_error(&io).code.as_str(), "snapshot.io_failed");
        assert!(
            !serde_json::to_string(&snapshot_error(&io))
                .unwrap()
                .contains("SECRET")
        );
        assert!(snapshot_error(&SnapshotError::Busy).retryable);
        assert!(!snapshot_error(&SnapshotError::InvalidDatabase).retryable);
    }

    #[test]
    fn machine_errors_emit_one_contract_record_and_no_human_output() {
        let error = failure("contract_violation", "Configuration is invalid.");
        let mut output = Vec::new();
        let mut diagnostics = Vec::new();
        write_error(&error, true, &mut output, &mut diagnostics).unwrap();
        assert_eq!(
            serde_json::from_slice::<ErrorEnvelope>(&output).unwrap(),
            error
        );
        assert_eq!(output.iter().filter(|byte| **byte == b'\n').count(), 1);
        assert!(diagnostics.is_empty());
        output.clear();
        write_error(&error, false, &mut output, &mut diagnostics).unwrap();
        assert!(output.is_empty());
        assert!(
            String::from_utf8(diagnostics)
                .unwrap()
                .contains("contract_violation")
        );
    }

    #[tokio::test]
    async fn status_verifies_service_identity_http_semantics_and_actual_readiness() {
        for (ready, status) in [(true, 200), (false, 503), (true, 503)] {
            let app = Router::new()
                .route(
                    "/readyz",
                    get(move || async move {
                        (
                            axum::http::StatusCode::from_u16(status).unwrap(),
                            Json(serde_json::json!({"ready":ready})),
                        )
                    }),
                )
                .layer(middleware::from_fn_with_state(
                    "example".to_string(),
                    service_identity_middleware,
                ));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let result = query_status(address, "example").await;
            if ready == (status == 200) {
                assert_eq!(result.unwrap().ready, ready);
            } else {
                assert_eq!(result.unwrap_err().code.as_str(), "invalid_status_response");
            }
            assert_eq!(
                query_status(address, "another")
                    .await
                    .unwrap_err()
                    .code
                    .as_str(),
                "service_identity_mismatch"
            );
            server.abort();
            let _ = server.await;
        }
    }

    #[test]
    fn runtime_log_validation_never_creates_or_repairs_a_directory() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        let parent = tempfile::tempdir().unwrap();
        let state = parent.path().join("state");
        std::fs::create_dir(&state).unwrap();
        std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(validate_runtime_log_directory(&state).is_err());
        assert!(!state.join("logs").exists());
        std::fs::create_dir(state.join("logs")).unwrap();
        std::fs::set_permissions(state.join("logs"), std::fs::Permissions::from_mode(0o700))
            .unwrap();
        validate_runtime_log_directory(&state).unwrap();
        std::fs::set_permissions(state.join("logs"), std::fs::Permissions::from_mode(0o755))
            .unwrap();
        assert!(validate_runtime_log_directory(&state).is_err());
        assert_eq!(
            std::fs::metadata(state.join("logs"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755
        );
        std::fs::remove_dir(state.join("logs")).unwrap();
        symlink(parent.path(), state.join("logs")).unwrap();
        assert!(validate_runtime_log_directory(&state).is_err());
        assert!(validate_runtime_log_directory(&state).is_err());
    }
}
