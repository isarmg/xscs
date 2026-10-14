//! Product-owned management protocol. No streaming, arbitrary command, path or URL messages.
use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub mod config;

/// Current Client session/capability contract.
pub const PROTOCOL: &str = "xscs-management/1";
/// Current business-task contract. Its shape stays stable when session capabilities change,
/// preserving durable operation fingerprints across transport-only updates.
pub const TASK_PROTOCOL: &str = "xscs-task/1";
/// A slash is not legal in an RFC 6455 WebSocket subprotocol header token.
pub const WEBSOCKET_SUBPROTOCOL: &str = "xscs-management.v1";
pub const SUNSHINE_VERSION: &str = "2026.914.233613";
pub const SUPPORTED_SUNSHINE_VERSIONS: &[&str] = &[SUNSHINE_VERSION];
pub const LIVE_HEALTH_PATH: &str = "/healthz";
pub const READY_HEALTH_PATH: &str = "/readyz";
pub const CLIENT_NAMESPACE: &str = "/xscc";
pub const ENROLL_PATH: &str = "/xscc/v1/enroll";
pub const PAIRING_PATH: &str = "/xscc/v1/pairing";
pub const IDENTITY_PATH: &str = "/xscc/v1/identity";
pub const CONNECT_PATH: &str = "/xscc/v1/connect";
pub const MAX_MESSAGE_BYTES: usize = 64 * 1024;
/// Reserve room for the Result envelope and operation ID, including JSON escaping.
pub const MAX_REPORT_BYTES: usize = MAX_MESSAGE_BYTES - 1024;
pub const MAX_EXECUTION_SECONDS: u64 = 90;

pub fn report_digest(report: &Report) -> Result<String, InvalidTask> {
    let bytes = serde_json::to_vec(report).map_err(|_| InvalidTask)?;
    Ok(hex::encode(Sha256::digest(bytes)))
}
pub const AUTHORIZATION_CODE_LENGTH: usize = 36;

/// Return whether a Sunshine instance authorization code has the exact
/// Manager-issued wire format. Keep this in the shared protocol crate so the
/// Manager and native Clients cannot silently drift to different lengths.
pub fn is_valid_authorization_code(value: &str) -> bool {
    value.len() == AUTHORIZATION_CODE_LENGTH
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte.is_ascii_lowercase())
}

pub fn is_supported_sunshine_version(version: &str) -> bool {
    SUPPORTED_SUNSHINE_VERSIONS.contains(&version)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub manager_id: Uuid,
    pub device_id: Uuid,
    pub installation_id: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Permission {
    ReadConfig,
    WriteConfig,
    Restart,
    ReadApplications,
    ManageApplications,
    ManagePairing,
    ReadDiagnostics,
    MaintainSunshine,
    ControlService,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RestartPolicy {
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplicationRef {
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreparationCommand {
    #[serde(rename = "do")]
    pub execute: String,
    pub undo: String,
    #[serde(default)]
    pub elevated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplicationSpec {
    pub name: String,
    #[serde(default)]
    pub output: String,
    #[serde(default)]
    pub cmd: String,
    #[serde(default, rename = "working-dir")]
    pub working_dir: String,
    #[serde(default, rename = "exclude-global-prep-cmd")]
    pub exclude_global_prep_cmd: bool,
    #[serde(default)]
    pub elevated: bool,
    #[serde(default, rename = "auto-detach")]
    pub auto_detach: bool,
    #[serde(default, rename = "wait-all")]
    pub wait_all: bool,
    #[serde(default = "default_exit_timeout", rename = "exit-timeout")]
    pub exit_timeout: u16,
    #[serde(default, rename = "prep-cmd")]
    pub prep_cmd: Vec<PreparationCommand>,
    #[serde(default)]
    pub detached: Vec<String>,
    #[serde(default, rename = "image-path")]
    pub image_path: String,
}

const fn default_exit_timeout() -> u16 {
    5
}

impl ApplicationSpec {
    pub fn has_host_commands(&self) -> bool {
        !self.output.is_empty()
            || !self.cmd.is_empty()
            || !self.working_dir.is_empty()
            || self.exclude_global_prep_cmd
            || self.elevated
            || !self.prep_cmd.is_empty()
            || !self.detached.is_empty()
    }

    pub fn validate(&self, host_commands_allowed: bool) -> Result<(), InvalidTask> {
        validate_text(&self.name, 1, 128)?;
        for value in [&self.output, &self.cmd, &self.working_dir, &self.image_path] {
            validate_optional_text(value, 4096)?;
        }
        if self.exit_timeout > 3600 || self.prep_cmd.len() > 16 || self.detached.len() > 16 {
            return Err(InvalidTask);
        }
        for prep in &self.prep_cmd {
            validate_optional_text(&prep.execute, 4096)?;
            validate_optional_text(&prep.undo, 4096)?;
            if prep.execute.is_empty() && prep.undo.is_empty() {
                return Err(InvalidTask);
            }
        }
        for command in &self.detached {
            validate_text(command, 1, 4096)?;
        }
        if self.has_host_commands() && !host_commands_allowed {
            return Err(InvalidTask);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplicationView {
    pub reference: ApplicationRef,
    pub specification: ApplicationSpec,
}

pub fn application_reference(application: &ApplicationSpec) -> Result<ApplicationRef, InvalidTask> {
    application.validate(true)?;
    let bytes = serde_json::to_vec(application).map_err(|_| InvalidTask)?;
    let mut digest = Sha256::new();
    digest.update(b"sunshine-application-v1\0");
    digest.update((bytes.len() as u64).to_be_bytes());
    digest.update(bytes);
    Ok(ApplicationRef {
        fingerprint: hex::encode(digest.finalize()),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApplicationsSnapshot {
    pub revision: String,
    pub applications: Vec<ApplicationView>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PairedClient {
    pub uuid: String,
    pub name: String,
    pub enabled: bool,
}

/// A pending Moonlight handshake, obtained from Sunshine rather than entered by the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PendingPairing {
    pub id: String,
    pub name: String,
    pub address: String,
}

pub fn validate_pending_pairings(pairings: &[PendingPairing]) -> Result<(), InvalidTask> {
    if pairings.len() > 64 {
        return Err(InvalidTask);
    }
    let mut ids = BTreeSet::new();
    for pairing in pairings {
        if pairing.id.len() != 32
            || !pairing.id.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !ids.insert(pairing.id.to_ascii_lowercase())
        {
            return Err(InvalidTask);
        }
        validate_optional_text(&pairing.name, 128)?;
        validate_text(&pairing.address, 1, 128)?;
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PairedClientsSnapshot {
    pub revision: String,
    pub clients: Vec<PairedClient>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogCursor {
    pub revision: String,
    pub before_offset: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogPage {
    pub revision: String,
    pub text: String,
    pub start_offset: u64,
    pub end_offset: u64,
    pub total_bytes: u64,
    pub previous: Option<LogCursor>,
    pub redacted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DriverStatus {
    pub installed: bool,
    pub version: Option<String>,
    pub required_version: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VirtualInputStatus {
    pub virtualhid: DriverStatus,
    pub vigembus: DriverStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServiceState {
    Running,
    Stopped,
    Starting,
    Stopping,
    Unavailable,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ServiceAction {
    Start,
    Stop,
    Restart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticSnapshot {
    pub sunshine_version: Option<String>,
    pub platform: Option<String>,
    pub api_reachable: bool,
    pub authentication_accepted: bool,
    pub service_state: ServiceState,
    pub configuration_revision: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MaintenanceAction {
    ResetDisplayPersistence,
    ResetPortalToken,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Command {
    ReadConfig {},
    PatchConfig {
        expected_revision: String,
        set: BTreeMap<String, config::FieldValue>,
        remove: BTreeSet<String>,
        restart_policy: RestartPolicy,
    },
    /// Replace the complete managed configuration snapshot, preserving local-only fields.
    /// Every version/OS-applicable allowlisted field occurs once in `set` or `remove`.
    SaveConfig {
        set: BTreeMap<String, config::FieldValue>,
        remove: BTreeSet<String>,
        restart_policy: RestartPolicy,
    },
    Restart {
        expected_revision: String,
        /// An explicit administrator decision; the Client must ALSO allow restart locally.
        administrator_confirmed: bool,
    },
    ListApplications {},
    SaveApplication {
        expected_revision: String,
        target: Option<ApplicationRef>,
        application: ApplicationSpec,
        administrator_confirmed_host_commands: bool,
    },
    DeleteApplication {
        expected_revision: String,
        target: ApplicationRef,
        administrator_confirmed: bool,
    },
    CloseApplication {
        administrator_confirmed: bool,
    },
    UploadCover {
        key: String,
        png_base64: String,
        administrator_confirmed: bool,
    },
    SubmitPairingPin {
        pairing_id: String,
        pin: String,
        name: String,
    },
    ListPendingPairings {},
    ListPairedClients {},
    SetPairedClientEnabled {
        uuid: String,
        enabled: bool,
        administrator_confirmed: bool,
    },
    UnpairClient {
        uuid: String,
        administrator_confirmed: bool,
    },
    UnpairAllClients {
        administrator_confirmed: bool,
    },
    ReadLogs {
        cursor: Option<LogCursor>,
        limit_bytes: u32,
    },
    ReadDiagnostics {},
    ReadVirtualInputStatus {},
    RunMaintenance {
        action: MaintenanceAction,
        administrator_confirmed: bool,
    },
    ReadServiceStatus {},
    ControlService {
        action: ServiceAction,
        administrator_confirmed: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Task {
    pub protocol: String,
    pub operation_id: String,
    pub binding: Binding,
    pub permission: Permission,
    pub command: Command,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    Execute,
    /// May inspect durable facts/actual state, but MUST NOT perform a side effect.
    InspectOnly,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ManagerMessage {
    Task {
        mode: DeliveryMode,
        task: Box<Task>,
        /// Immutable server deadline plus receipt-relative budget; neither is task identity.
        expires_at_unix_ms: u64,
        remaining_ms: u64,
    },
    ResultAccepted {
        operation_id: String,
        fingerprint: String,
        report_digest: String,
        /// xcss has a final outcome, including an explicit human resolution.
        finalized: bool,
    },
    Revoked {},
}

/// These are execution observations, NOT another persistent task lifecycle.
/// Manager maps observations into the existing xcss operation transitions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Report {
    ConfigRead {
        snapshot: ConfigSnapshot,
    },
    ConfigSaved {
        snapshot: ConfigSnapshot,
    },
    RestartAcknowledged {
        snapshot: ConfigSnapshot,
        process_generation: String,
    },
    ApplicationsRead {
        snapshot: ApplicationsSnapshot,
    },
    ApplicationSaved {
        snapshot: ApplicationsSnapshot,
    },
    ApplicationDeleted {
        snapshot: ApplicationsSnapshot,
    },
    ApplicationClosed {},
    CoverUploaded {
        path: String,
    },
    PairingPinSubmitted {},
    PendingPairingsRead {
        pairings: Vec<PendingPairing>,
    },
    PairedClientsRead {
        snapshot: PairedClientsSnapshot,
    },
    PairedClientUpdated {
        snapshot: PairedClientsSnapshot,
    },
    LogsRead {
        page: LogPage,
    },
    DiagnosticsRead {
        snapshot: DiagnosticSnapshot,
    },
    VirtualInputStatusRead {
        status: VirtualInputStatus,
    },
    MaintenanceCompleted {
        action: MaintenanceAction,
    },
    ServiceStatusRead {
        state: ServiceState,
    },
    ServiceControlled {
        action: ServiceAction,
        state: ServiceState,
    },
    Conflict {
        actual_revision: String,
    },
    Rejected {
        reason: Rejection,
    },
    Unknown {
        reason: Uncertainty,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Rejection {
    InvalidTask,
    BindingMismatch,
    PermissionDenied,
    OperationIdReused,
    UnsupportedVersion,
    UnsafeConfiguration,
    SunshineUnavailable,
    JournalFull,
    ResourceConflict,
    UnsupportedCapability,
    ExecutionExpired,
    ResultTooLarge,
    OperationAlreadyCompleted,
    PairingFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Uncertainty {
    NoExecutionRecord,
    EffectNotConfirmed,
    RestartNotConfirmed,
    PersistenceFailure,
    SideEffectNotConfirmed,
    ServiceTransitionNotConfirmed,
    ResultTooLarge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Effectiveness {
    /// Reading a file does not prove that its settings are used by the running process.
    PendingVerification,
    AwaitingRestart,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ConfigSnapshot {
    pub revision: String,
    pub sunshine_version: String,
    /// Only allowlisted fields; never send the entire local configuration to Manager.
    pub fields: BTreeMap<String, String>,
    pub effectiveness: Effectiveness,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    pub protocol: String,
    pub client_version: String,
    pub os: ClientOs,
    pub sunshine_version: String,
    pub restart_allowed: bool,
    pub managed_fields: Vec<String>,
    /// Supports complete managed-configuration replacement.
    pub configuration_overwrite: bool,
    pub application_management: bool,
    pub application_host_commands_allowed: bool,
    pub moonlight_pairing_management: bool,
    /// Supports enumerating pending Moonlight pairing requests.
    pub pending_pairing_listing: bool,
    pub diagnostics: bool,
    pub maintenance: bool,
    pub service_control: bool,
}

pub const MAX_CLIENT_VERSION_BYTES: usize = 64;
pub const MAX_STORED_CONFIG_VALUE_BYTES: usize = 512;
pub const REVISION_HEX_BYTES: usize = 64;

/// Validation used for persistent Manager observations, without Client-report effectiveness checks.
pub fn validate_stored_snapshot(snapshot: &ConfigSnapshot) -> Result<(), InvalidTask> {
    validate_revision(&snapshot.revision)?;
    if !is_supported_sunshine_version(&snapshot.sunshine_version)
        || snapshot.fields.iter().any(|(key, value)| {
            !config::contains_field(key) || value.len() > MAX_STORED_CONFIG_VALUE_BYTES
        })
    {
        return Err(InvalidTask);
    }
    Ok(())
}

impl Capabilities {
    /// The current contract defines capabilities; a release label records provenance only.
    pub fn validate(&self) -> Result<(), InvalidTask> {
        validate_text(&self.client_version, 1, MAX_CLIENT_VERSION_BYTES)?;
        if self.protocol != PROTOCOL
            || !is_supported_sunshine_version(&self.sunshine_version)
            || self.managed_fields.len() != config::FIELD_DEFINITIONS.len()
            || config::FIELD_DEFINITIONS
                .iter()
                .any(|field| !self.managed_fields.iter().any(|key| key == field.key))
        {
            return Err(InvalidTask);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClientOs {
    LinuxX86_64,
    WindowsX86_64,
    MacosX86_64,
    MacosAarch64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ActiveOperation {
    pub operation_id: String,
    pub fingerprint: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClientMessage {
    Progress {
        operation_id: String,
        fingerprint: String,
    },
    Hello {
        binding: Binding,
        capabilities: Capabilities,
        #[serde(default)]
        active_operation: Option<ActiveOperation>,
    },
    Heartbeat {
        sunshine_reachable: Option<bool>,
        configuration: Option<ConfigSnapshot>,
    },
    Result {
        operation_id: String,
        report: Report,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("invalid sunshine management task")]
pub struct InvalidTask;

impl Task {
    pub fn validate(
        &self,
        binding: &Binding,
        capabilities: &Capabilities,
    ) -> Result<(), Rejection> {
        if &self.binding != binding {
            return Err(Rejection::BindingMismatch);
        }
        if self.protocol != TASK_PROTOCOL
            || capabilities.validate().is_err()
            || self.binding.manager_id.is_nil()
            || self.binding.device_id.is_nil()
            || self.binding.installation_id.is_nil()
            || self.operation_id.strip_prefix("op_").is_none_or(|id| {
                !Uuid::parse_str(id).is_ok_and(|uuid| !uuid.is_nil() && uuid.to_string() == id)
            })
        {
            return Err(Rejection::InvalidTask);
        }
        let permission = match &self.command {
            Command::ReadConfig {} => Permission::ReadConfig,
            Command::PatchConfig {
                expected_revision,
                set,
                remove,
                ..
            } => {
                validate_revision(expected_revision).map_err(|_| Rejection::InvalidTask)?;
                if set.is_empty() && remove.is_empty()
                    || set.len() + remove.len() > config::FIELD_DEFINITIONS.len()
                {
                    return Err(Rejection::InvalidTask);
                }
                for (key, value) in set {
                    if remove.contains(key) || config::validate_field(key, value).is_err() {
                        return Err(Rejection::InvalidTask);
                    }
                }
                if remove.iter().any(|key| !config::contains_field(key)) {
                    return Err(Rejection::InvalidTask);
                }
                Permission::WriteConfig
            }
            Command::SaveConfig { set, remove, .. } => {
                require(
                    capabilities.configuration_overwrite,
                    Permission::WriteConfig,
                )?;
                let os = match capabilities.os {
                    ClientOs::LinuxX86_64 => "linux_x86_64",
                    ClientOs::WindowsX86_64 => "windows_x86_64",
                    ClientOs::MacosX86_64 => "macos_x86_64",
                    ClientOs::MacosAarch64 => "macos_aarch64",
                };
                let applicable: BTreeSet<&str> = config::FIELD_DEFINITIONS
                    .iter()
                    .filter(|field| {
                        field.operating_systems.contains(&os)
                            && field
                                .supported_sunshine_versions
                                .contains(&capabilities.sunshine_version.as_str())
                    })
                    .map(|field| field.key)
                    .collect();
                if set.len() + remove.len() != applicable.len() {
                    return Err(Rejection::InvalidTask);
                }
                for (key, value) in set {
                    if !applicable.contains(key.as_str())
                        || remove.contains(key)
                        || config::validate_field(key, value).is_err()
                    {
                        return Err(Rejection::InvalidTask);
                    }
                }
                if remove.iter().any(|key| !applicable.contains(key.as_str())) {
                    return Err(Rejection::InvalidTask);
                }
                Permission::WriteConfig
            }
            Command::Restart {
                expected_revision,
                administrator_confirmed,
            } => {
                validate_revision(expected_revision).map_err(|_| Rejection::InvalidTask)?;
                if !capabilities.restart_allowed || !administrator_confirmed {
                    return Err(Rejection::PermissionDenied);
                }
                Permission::Restart
            }
            Command::ListApplications {} => require(
                capabilities.application_management,
                Permission::ReadApplications,
            )?,
            Command::SaveApplication {
                expected_revision,
                target,
                application,
                administrator_confirmed_host_commands,
            } => {
                require(
                    capabilities.application_management,
                    Permission::ManageApplications,
                )?;
                validate_revision(expected_revision).map_err(|_| Rejection::InvalidTask)?;
                if let Some(target) = target {
                    validate_revision(&target.fingerprint).map_err(|_| Rejection::InvalidTask)?;
                }
                application
                    .validate(capabilities.application_host_commands_allowed)
                    .map_err(|_| {
                        if application.has_host_commands() {
                            Rejection::UnsupportedCapability
                        } else {
                            Rejection::InvalidTask
                        }
                    })?;
                if application.has_host_commands() && !administrator_confirmed_host_commands {
                    return Err(Rejection::PermissionDenied);
                }
                Permission::ManageApplications
            }
            Command::DeleteApplication {
                expected_revision,
                target,
                administrator_confirmed,
            } => {
                require(
                    capabilities.application_management,
                    Permission::ManageApplications,
                )?;
                if !administrator_confirmed {
                    return Err(Rejection::PermissionDenied);
                }
                validate_revision(expected_revision).map_err(|_| Rejection::InvalidTask)?;
                validate_revision(&target.fingerprint).map_err(|_| Rejection::InvalidTask)?;
                Permission::ManageApplications
            }
            Command::CloseApplication {
                administrator_confirmed,
            } => {
                require(
                    capabilities.application_management,
                    Permission::ManageApplications,
                )?;
                if !administrator_confirmed {
                    return Err(Rejection::PermissionDenied);
                }
                Permission::ManageApplications
            }
            Command::UploadCover {
                key,
                png_base64,
                administrator_confirmed,
            } => {
                require(
                    capabilities.application_management,
                    Permission::ManageApplications,
                )?;
                if !administrator_confirmed {
                    return Err(Rejection::PermissionDenied);
                }
                validate_identifier(key, 64).map_err(|_| Rejection::InvalidTask)?;
                if png_base64.is_empty()
                    || png_base64.len() > 40 * 1024
                    || !png_base64
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
                {
                    return Err(Rejection::InvalidTask);
                }
                Permission::ManageApplications
            }
            Command::SubmitPairingPin {
                pairing_id,
                pin,
                name,
            } => {
                require(
                    capabilities.moonlight_pairing_management,
                    Permission::ManagePairing,
                )?;
                if pairing_id.len() != 32
                    || !pairing_id.bytes().all(|b| b.is_ascii_hexdigit())
                    || pin.len() != 4
                    || !pin.bytes().all(|b| b.is_ascii_digit())
                    || validate_text(name, 1, 128).is_err()
                {
                    return Err(Rejection::InvalidTask);
                }
                Permission::ManagePairing
            }
            Command::ListPendingPairings {} => {
                require(
                    capabilities.pending_pairing_listing,
                    Permission::ManagePairing,
                )?;
                require(
                    capabilities.moonlight_pairing_management,
                    Permission::ManagePairing,
                )?
            }
            Command::ListPairedClients {} => require(
                capabilities.moonlight_pairing_management,
                Permission::ManagePairing,
            )?,
            Command::SetPairedClientEnabled {
                uuid,
                enabled,
                administrator_confirmed,
            } => {
                require(
                    capabilities.moonlight_pairing_management,
                    Permission::ManagePairing,
                )?;
                if !enabled && !administrator_confirmed {
                    return Err(Rejection::PermissionDenied);
                }
                validate_uuid(uuid).map_err(|_| Rejection::InvalidTask)?;
                Permission::ManagePairing
            }
            Command::UnpairClient {
                uuid,
                administrator_confirmed,
            } => {
                require(
                    capabilities.moonlight_pairing_management,
                    Permission::ManagePairing,
                )?;
                if !administrator_confirmed {
                    return Err(Rejection::PermissionDenied);
                }
                validate_uuid(uuid).map_err(|_| Rejection::InvalidTask)?;
                Permission::ManagePairing
            }
            Command::UnpairAllClients {
                administrator_confirmed,
            } => {
                require(
                    capabilities.moonlight_pairing_management,
                    Permission::ManagePairing,
                )?;
                if !administrator_confirmed {
                    return Err(Rejection::PermissionDenied);
                }
                Permission::ManagePairing
            }
            Command::ReadLogs {
                cursor,
                limit_bytes,
            } => {
                require(capabilities.diagnostics, Permission::ReadDiagnostics)?;
                if !(1024..=24 * 1024).contains(limit_bytes) {
                    return Err(Rejection::InvalidTask);
                }
                if let Some(cursor) = cursor {
                    validate_revision(&cursor.revision).map_err(|_| Rejection::InvalidTask)?;
                }
                Permission::ReadDiagnostics
            }
            Command::ReadDiagnostics {} | Command::ReadVirtualInputStatus {} => {
                require(capabilities.diagnostics, Permission::ReadDiagnostics)?
            }
            Command::RunMaintenance {
                administrator_confirmed,
                ..
            } => {
                require(capabilities.maintenance, Permission::MaintainSunshine)?;
                if !administrator_confirmed {
                    return Err(Rejection::PermissionDenied);
                }
                Permission::MaintainSunshine
            }
            Command::ReadServiceStatus {} => {
                require(capabilities.service_control, Permission::ControlService)?
            }
            Command::ControlService {
                administrator_confirmed,
                ..
            } => {
                require(capabilities.service_control, Permission::ControlService)?;
                if !administrator_confirmed {
                    return Err(Rejection::PermissionDenied);
                }
                Permission::ControlService
            }
        };
        self.fingerprint().map_err(|_| Rejection::InvalidTask)?;
        if self.permission != permission {
            return Err(Rejection::PermissionDenied);
        }
        Ok(())
    }

    /// Stable across object/set ordering and retransmission mode. Binding and permission are covered.
    pub fn fingerprint(&self) -> Result<String, InvalidTask> {
        let bytes = serde_json::to_vec(self).map_err(|_| InvalidTask)?;
        if bytes.len() > MAX_MESSAGE_BYTES - 1024 {
            return Err(InvalidTask);
        }
        Ok(hex::encode(Sha256::digest(bytes)))
    }
}

fn require(condition: bool, permission: Permission) -> Result<Permission, Rejection> {
    if condition {
        Ok(permission)
    } else {
        Err(Rejection::UnsupportedCapability)
    }
}

fn validate_text(value: &str, minimum: usize, maximum: usize) -> Result<(), InvalidTask> {
    let length = value.len();
    if length < minimum
        || length > maximum
        || value != value.trim()
        || value.chars().any(|c| c == '\0' || c.is_control())
    {
        Err(InvalidTask)
    } else {
        Ok(())
    }
}
fn validate_optional_text(value: &str, maximum: usize) -> Result<(), InvalidTask> {
    if value.is_empty() {
        Ok(())
    } else {
        validate_text(value, 1, maximum)
    }
}
fn validate_identifier(value: &str, maximum: usize) -> Result<(), InvalidTask> {
    if value.is_empty()
        || value.len() > maximum
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        Err(InvalidTask)
    } else {
        Ok(())
    }
}
fn validate_uuid(value: &str) -> Result<(), InvalidTask> {
    let parsed = Uuid::parse_str(value).map_err(|_| InvalidTask)?;
    if parsed.is_nil() || parsed.to_string() != value.to_ascii_lowercase() {
        Err(InvalidTask)
    } else {
        Ok(())
    }
}

pub fn validate_revision(revision: &str) -> Result<(), InvalidTask> {
    if revision.len() == REVISION_HEX_BYTES
        && revision
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(InvalidTask)
    }
}

pub fn decode_manager_message(bytes: &[u8]) -> Result<ManagerMessage, InvalidTask> {
    if bytes.len() > MAX_MESSAGE_BYTES {
        return Err(InvalidTask);
    }
    serde_json::from_slice(bytes).map_err(|_| InvalidTask)
}

pub fn validate_report(command: &Command, report: &Report) -> Result<(), InvalidTask> {
    let valid = match (command, report) {
        (Command::ReadConfig {}, Report::ConfigRead { snapshot }) => {
            validate_config_snapshot(snapshot, Effectiveness::PendingVerification).is_ok()
        }
        (
            Command::PatchConfig { .. } | Command::SaveConfig { .. },
            Report::ConfigSaved { snapshot },
        ) => validate_config_snapshot(snapshot, Effectiveness::AwaitingRestart).is_ok(),
        (
            Command::Restart { .. },
            Report::RestartAcknowledged {
                snapshot,
                process_generation,
            },
        ) => {
            validate_config_snapshot(snapshot, Effectiveness::PendingVerification).is_ok()
                && validate_text(process_generation, 1, 256).is_ok()
        }
        (Command::ListApplications {}, Report::ApplicationsRead { snapshot })
        | (Command::SaveApplication { .. }, Report::ApplicationSaved { snapshot })
        | (Command::DeleteApplication { .. }, Report::ApplicationDeleted { snapshot }) => {
            validate_applications(snapshot).is_ok()
        }
        (Command::CloseApplication { .. }, Report::ApplicationClosed {}) => true,
        (Command::UploadCover { .. }, Report::CoverUploaded { path }) => {
            validate_text(path, 1, 4096).is_ok()
        }
        (Command::SubmitPairingPin { .. }, Report::PairingPinSubmitted {}) => true,
        (Command::ListPendingPairings {}, Report::PendingPairingsRead { pairings }) => {
            validate_pending_pairings(pairings).is_ok()
        }
        (Command::ListPairedClients {}, Report::PairedClientsRead { snapshot })
        | (Command::SetPairedClientEnabled { .. }, Report::PairedClientUpdated { snapshot })
        | (Command::UnpairClient { .. }, Report::PairedClientUpdated { snapshot })
        | (Command::UnpairAllClients { .. }, Report::PairedClientUpdated { snapshot }) => {
            validate_paired_clients(snapshot).is_ok()
        }
        (Command::ReadLogs { .. }, Report::LogsRead { page }) => validate_log_page(page).is_ok(),
        (Command::ReadDiagnostics {}, Report::DiagnosticsRead { snapshot }) => {
            validate_diagnostics(snapshot).is_ok()
        }
        (Command::ReadVirtualInputStatus {}, Report::VirtualInputStatusRead { status }) => {
            validate_virtual_input(status).is_ok()
        }
        (
            Command::RunMaintenance { action, .. },
            Report::MaintenanceCompleted { action: observed },
        ) => action == observed,
        (Command::ReadServiceStatus {}, Report::ServiceStatusRead { .. }) => true,
        (
            Command::ControlService { action, .. },
            Report::ServiceControlled {
                action: observed, ..
            },
        ) => action == observed,
        (_, Report::Conflict { actual_revision }) => validate_revision(actual_revision).is_ok(),
        (_, Report::Rejected { .. } | Report::Unknown { .. }) => true,
        _ => false,
    };
    if !valid || serde_json::to_vec(report).map_err(|_| InvalidTask)?.len() > MAX_REPORT_BYTES {
        Err(InvalidTask)
    } else {
        Ok(())
    }
}

fn validate_config_snapshot(
    snapshot: &ConfigSnapshot,
    effectiveness: Effectiveness,
) -> Result<(), InvalidTask> {
    validate_revision(&snapshot.revision)?;
    if snapshot.effectiveness != effectiveness
        || !is_supported_sunshine_version(&snapshot.sunshine_version)
        || snapshot.fields.iter().any(|(key, value)| {
            value.len() > 512 || config::validate_snapshot_field(key, value).is_err()
        })
    {
        Err(InvalidTask)
    } else {
        Ok(())
    }
}

fn validate_applications(snapshot: &ApplicationsSnapshot) -> Result<(), InvalidTask> {
    validate_revision(&snapshot.revision)?;
    if snapshot.applications.len() > 256 {
        return Err(InvalidTask);
    }
    let mut references = BTreeSet::new();
    for app in &snapshot.applications {
        validate_revision(&app.reference.fingerprint)?;
        app.specification.validate(true)?;
        if !references.insert(&app.reference.fingerprint) {
            return Err(InvalidTask);
        }
    }
    Ok(())
}

fn validate_paired_clients(snapshot: &PairedClientsSnapshot) -> Result<(), InvalidTask> {
    validate_revision(&snapshot.revision)?;
    if snapshot.clients.len() > 512 {
        return Err(InvalidTask);
    }
    let mut ids = BTreeSet::new();
    for client in &snapshot.clients {
        validate_uuid(&client.uuid)?;
        // Sunshine preserves paired-client display names, including surrounding spaces.
        // Actions address the UUID; keep the original name and snapshot revision intact.
        if client.name.is_empty()
            || client.name.len() > 128
            || client.name.chars().any(char::is_control)
        {
            return Err(InvalidTask);
        }
        if !ids.insert(&client.uuid) {
            return Err(InvalidTask);
        }
    }
    Ok(())
}

fn validate_log_page(page: &LogPage) -> Result<(), InvalidTask> {
    validate_revision(&page.revision)?;
    if page.text.len() > 24 * 1024
        || page.start_offset > page.end_offset
        || page.end_offset > page.total_bytes
        || page.end_offset - page.start_offset != page.text.len() as u64
    {
        return Err(InvalidTask);
    }
    if let Some(cursor) = &page.previous {
        validate_revision(&cursor.revision)?;
        if cursor.revision != page.revision || cursor.before_offset != page.start_offset {
            return Err(InvalidTask);
        }
    }
    Ok(())
}

fn validate_diagnostics(value: &DiagnosticSnapshot) -> Result<(), InvalidTask> {
    if let Some(version) = &value.sunshine_version
        && !is_supported_sunshine_version(version)
    {
        return Err(InvalidTask);
    }
    if let Some(platform) = &value.platform {
        validate_text(platform, 1, 64)?;
    }
    if let Some(revision) = &value.configuration_revision {
        validate_revision(revision)?;
    }
    Ok(())
}

fn validate_virtual_input(value: &VirtualInputStatus) -> Result<(), InvalidTask> {
    for driver in [&value.virtualhid, &value.vigembus] {
        for text in [&driver.version, &driver.required_version, &driver.error]
            .into_iter()
            .flatten()
        {
            validate_text(text, 1, 256)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod macos_platform_tests {
    use super::*;
    #[test]
    fn additive_platform_values_preserve_old_clients() {
        for os in [
            ClientOs::LinuxX86_64,
            ClientOs::WindowsX86_64,
            ClientOs::MacosX86_64,
            ClientOs::MacosAarch64,
        ] {
            let bytes = serde_json::to_vec(&os).unwrap();
            assert_eq!(serde_json::from_slice::<ClientOs>(&bytes).unwrap(), os);
        }
        assert_eq!(
            serde_json::to_string(&ClientOs::MacosAarch64).unwrap(),
            "\"macos_aarch64\""
        );
        assert_eq!(
            serde_json::to_string(&ClientOs::MacosX86_64).unwrap(),
            "\"macos_x86_64\""
        );
    }

    #[test]
    fn only_verified_stable_sunshine_versions_are_admitted() {
        assert!(is_supported_sunshine_version("2026.914.233613"));
        assert!(!is_supported_sunshine_version("2026.906.222525"));
        assert!(!is_supported_sunshine_version("2026.910.221003"));
        assert!(!is_supported_sunshine_version("invalid"));
    }
}

#[cfg(test)]
mod protocol_v1_tests {
    use super::*;

    #[test]
    fn authorization_code_format_is_shared_and_exact() {
        assert!(is_valid_authorization_code(&"a0".repeat(18)));
        assert!(!is_valid_authorization_code(&"a".repeat(35)));
        assert!(!is_valid_authorization_code(&"a".repeat(37)));
        assert!(!is_valid_authorization_code(&"A".repeat(36)));
        assert!(!is_valid_authorization_code(&format!(
            "{}-",
            "a".repeat(35)
        )));
    }

    fn binding() -> Binding {
        Binding {
            manager_id: Uuid::from_u128(1),
            device_id: Uuid::from_u128(2),
            installation_id: Uuid::from_u128(3),
        }
    }

    fn capabilities() -> Capabilities {
        Capabilities {
            protocol: PROTOCOL.into(),
            client_version: "0.2.0".into(),
            os: ClientOs::LinuxX86_64,
            sunshine_version: SUNSHINE_VERSION.into(),
            restart_allowed: true,
            managed_fields: config::FIELD_DEFINITIONS
                .iter()
                .map(|field| field.key.to_owned())
                .collect(),
            configuration_overwrite: true,
            application_management: true,
            application_host_commands_allowed: true,
            moonlight_pairing_management: true,
            pending_pairing_listing: true,
            diagnostics: true,
            maintenance: true,
            service_control: true,
        }
    }

    fn application() -> ApplicationSpec {
        ApplicationSpec {
            name: "Steam".into(),
            output: String::new(),
            cmd: String::new(),
            working_dir: String::new(),
            exclude_global_prep_cmd: false,
            elevated: false,
            auto_detach: false,
            wait_all: false,
            exit_timeout: 5,
            prep_cmd: vec![],
            detached: vec![],
            image_path: String::new(),
        }
    }

    #[test]
    fn old_capability_shape_and_protocol_are_rejected() {
        let old = serde_json::json!({"protocol":"xscs-management/1","client_version":"0.1.4","os":"linux_x86_64","sunshine_version":SUNSHINE_VERSION,"restart_allowed":true,"managed_fields":[]});
        assert!(serde_json::from_value::<Capabilities>(old).is_err());
        assert!(
            decode_manager_message(
                br#"{"type":"task","mode":"execute","task":{"protocol":"xscs-management/1"}}"#
            )
            .is_err()
        );
    }

    #[test]
    fn overwrite_requires_a_complete_managed_snapshot_without_a_revision_guard() {
        let binding = binding();
        let set = BTreeMap::from([("qp".into(), config::FieldValue::Integer(29))]);
        let remove = config::FIELD_DEFINITIONS
            .iter()
            .filter(|field| {
                field.operating_systems.contains(&"linux_x86_64") && !set.contains_key(field.key)
            })
            .map(|field| field.key.to_owned())
            .collect();
        let mut task = Task {
            protocol: TASK_PROTOCOL.into(),
            operation_id: format!("op_{}", Uuid::from_u128(4)),
            binding: binding.clone(),
            permission: Permission::WriteConfig,
            command: Command::SaveConfig {
                set,
                remove,
                restart_policy: RestartPolicy::Manual,
            },
        };
        let mut capabilities = capabilities();
        capabilities.configuration_overwrite = false;
        assert_eq!(
            task.validate(&binding, &capabilities),
            Err(Rejection::UnsupportedCapability)
        );
        capabilities.configuration_overwrite = true;
        assert!(task.validate(&binding, &capabilities).is_ok());
        let encoded = serde_json::to_value(&task.command).unwrap();
        assert_eq!(encoded["kind"], "save_config");
        assert!(encoded.get("expected_revision").is_none());
        let report = Report::ConfigSaved {
            snapshot: ConfigSnapshot {
                revision: "a".repeat(64),
                sunshine_version: SUNSHINE_VERSION.into(),
                fields: BTreeMap::from([("qp".into(), "29".into())]),
                effectiveness: Effectiveness::AwaitingRestart,
            },
        };
        assert!(validate_report(&task.command, &report).is_ok());
        if let Command::SaveConfig { remove, .. } = &mut task.command {
            remove.remove("sunshine_name");
        }
        assert_eq!(
            task.validate(&binding, &capabilities),
            Err(Rejection::InvalidTask)
        );
        if let Command::SaveConfig { remove, .. } = &mut task.command {
            remove.insert("global_prep_cmd".into());
        }
        assert_eq!(
            task.validate(&binding, &capabilities),
            Err(Rejection::InvalidTask)
        );
    }

    #[test]
    fn release_labels_do_not_decide_current_capabilities() {
        let mut capabilities = capabilities();
        let task = Task {
            protocol: TASK_PROTOCOL.into(),
            operation_id: format!("op_{}", Uuid::from_u128(4)),
            binding: binding(),
            permission: Permission::ManagePairing,
            command: Command::ListPendingPairings {},
        };
        for version in ["0.1.0", "0.3.0", "0.4.0", "1.0.0-beta", "test"] {
            capabilities.client_version = version.into();
            assert!(task.validate(&task.binding, &capabilities).is_ok());
        }
        capabilities.pending_pairing_listing = false;
        assert_eq!(
            task.validate(&task.binding, &capabilities),
            Err(Rejection::UnsupportedCapability)
        );
        let mut missing = serde_json::to_value(&capabilities).unwrap();
        missing
            .as_object_mut()
            .unwrap()
            .remove("pending_pairing_listing");
        assert!(serde_json::from_value::<Capabilities>(missing).is_err());
    }

    #[test]
    fn host_commands_require_the_reported_capability_and_explicit_confirmation() {
        let binding = binding();
        let mut application = application();
        application.cmd = "steam".into();
        let mut task = Task {
            protocol: TASK_PROTOCOL.into(),
            operation_id: format!("op_{}", Uuid::from_u128(4)),
            binding: binding.clone(),
            permission: Permission::ManageApplications,
            command: Command::SaveApplication {
                expected_revision: "a".repeat(64),
                target: None,
                application,
                administrator_confirmed_host_commands: true,
            },
        };
        assert!(task.validate(&binding, &capabilities()).is_ok());
        let mut unavailable = capabilities();
        unavailable.application_host_commands_allowed = false;
        assert_eq!(
            task.validate(&binding, &unavailable),
            Err(Rejection::UnsupportedCapability)
        );
        if let Command::SaveApplication {
            administrator_confirmed_host_commands,
            ..
        } = &mut task.command
        {
            *administrator_confirmed_host_commands = false;
        }
        assert_eq!(
            task.validate(&binding, &capabilities()),
            Err(Rejection::PermissionDenied)
        );
    }

    #[test]
    fn reports_are_command_specific_and_bounded() {
        let command = Command::ReadLogs {
            cursor: None,
            limit_bytes: 24 * 1024,
        };
        let valid = Report::LogsRead {
            page: LogPage {
                revision: "b".repeat(64),
                text: "ready\n".into(),
                start_offset: 0,
                end_offset: 6,
                total_bytes: 6,
                previous: None,
                redacted: false,
            },
        };
        assert!(validate_report(&command, &valid).is_ok());
        assert!(validate_report(&Command::ReadDiagnostics {}, &valid).is_err());
        let oversized = Report::LogsRead {
            page: LogPage {
                revision: "b".repeat(64),
                text: "x".repeat(24 * 1024 + 1),
                start_offset: 0,
                end_offset: (24 * 1024 + 1) as u64,
                total_bytes: (24 * 1024 + 1) as u64,
                previous: None,
                redacted: false,
            },
        };
        assert!(validate_report(&command, &oversized).is_err());
    }

    #[test]
    fn application_references_are_content_bound() {
        let first = application_reference(&application()).unwrap();
        let mut renamed = application();
        renamed.name = "Steam Remote".into();
        assert_ne!(first, application_reference(&renamed).unwrap());
        assert_eq!(first, application_reference(&application()).unwrap());
    }
}

/// The strict plaintext stored inside the current operation envelope.
#[derive(Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StoredRequest {
    pub binding: Binding,
    pub command: Command,
}

#[derive(Serialize)]
pub struct CommandStorageContract {
    pub kind: &'static str,
    pub action: &'static str,
    pub resource: &'static str,
}

pub const COMMAND_STORAGE_CONTRACTS: &[CommandStorageContract] = &[
    CommandStorageContract {
        kind: "read_config",
        action: "sunshine.config.read",
        resource: "read",
    },
    CommandStorageContract {
        kind: "patch_config",
        action: "sunshine.config.patch",
        resource: "write",
    },
    CommandStorageContract {
        kind: "save_config",
        action: "sunshine.config.save",
        resource: "write",
    },
    CommandStorageContract {
        kind: "restart",
        action: "sunshine.restart",
        resource: "write",
    },
    CommandStorageContract {
        kind: "list_applications",
        action: "sunshine.applications.list",
        resource: "read",
    },
    CommandStorageContract {
        kind: "save_application",
        action: "sunshine.applications.save",
        resource: "write",
    },
    CommandStorageContract {
        kind: "delete_application",
        action: "sunshine.applications.delete",
        resource: "write",
    },
    CommandStorageContract {
        kind: "close_application",
        action: "sunshine.applications.close",
        resource: "write",
    },
    CommandStorageContract {
        kind: "upload_cover",
        action: "sunshine.applications.cover.upload",
        resource: "write",
    },
    CommandStorageContract {
        kind: "submit_pairing_pin",
        action: "sunshine.pairing.pin.submit",
        resource: "write",
    },
    CommandStorageContract {
        kind: "list_pending_pairings",
        action: "sunshine.pairing.pending.list",
        resource: "read",
    },
    CommandStorageContract {
        kind: "list_paired_clients",
        action: "sunshine.pairing.clients.list",
        resource: "read",
    },
    CommandStorageContract {
        kind: "set_paired_client_enabled",
        action: "sunshine.pairing.clients.update",
        resource: "write",
    },
    CommandStorageContract {
        kind: "unpair_client",
        action: "sunshine.pairing.clients.unpair",
        resource: "write",
    },
    CommandStorageContract {
        kind: "unpair_all_clients",
        action: "sunshine.pairing.clients.unpair_all",
        resource: "write",
    },
    CommandStorageContract {
        kind: "read_logs",
        action: "sunshine.logs.read",
        resource: "read",
    },
    CommandStorageContract {
        kind: "read_diagnostics",
        action: "sunshine.diagnostics.read",
        resource: "read",
    },
    CommandStorageContract {
        kind: "read_virtual_input_status",
        action: "sunshine.virtual_input.read",
        resource: "read",
    },
    CommandStorageContract {
        kind: "run_maintenance",
        action: "sunshine.maintenance.run",
        resource: "write",
    },
    CommandStorageContract {
        kind: "read_service_status",
        action: "sunshine.service.read",
        resource: "read",
    },
    CommandStorageContract {
        kind: "control_service",
        action: "sunshine.service.control",
        resource: "write",
    },
];

pub fn command_storage_contract(command: &Command) -> &'static CommandStorageContract {
    match command {
        Command::ReadConfig { .. } => &COMMAND_STORAGE_CONTRACTS[0],
        Command::PatchConfig { .. } => &COMMAND_STORAGE_CONTRACTS[1],
        Command::SaveConfig { .. } => &COMMAND_STORAGE_CONTRACTS[2],
        Command::Restart { .. } => &COMMAND_STORAGE_CONTRACTS[3],
        Command::ListApplications { .. } => &COMMAND_STORAGE_CONTRACTS[4],
        Command::SaveApplication { .. } => &COMMAND_STORAGE_CONTRACTS[5],
        Command::DeleteApplication { .. } => &COMMAND_STORAGE_CONTRACTS[6],
        Command::CloseApplication { .. } => &COMMAND_STORAGE_CONTRACTS[7],
        Command::UploadCover { .. } => &COMMAND_STORAGE_CONTRACTS[8],
        Command::SubmitPairingPin { .. } => &COMMAND_STORAGE_CONTRACTS[9],
        Command::ListPendingPairings { .. } => &COMMAND_STORAGE_CONTRACTS[10],
        Command::ListPairedClients { .. } => &COMMAND_STORAGE_CONTRACTS[11],
        Command::SetPairedClientEnabled { .. } => &COMMAND_STORAGE_CONTRACTS[12],
        Command::UnpairClient { .. } => &COMMAND_STORAGE_CONTRACTS[13],
        Command::UnpairAllClients { .. } => &COMMAND_STORAGE_CONTRACTS[14],
        Command::ReadLogs { .. } => &COMMAND_STORAGE_CONTRACTS[15],
        Command::ReadDiagnostics { .. } => &COMMAND_STORAGE_CONTRACTS[16],
        Command::ReadVirtualInputStatus { .. } => &COMMAND_STORAGE_CONTRACTS[17],
        Command::RunMaintenance { .. } => &COMMAND_STORAGE_CONTRACTS[18],
        Command::ReadServiceStatus { .. } => &COMMAND_STORAGE_CONTRACTS[19],
        Command::ControlService { .. } => &COMMAND_STORAGE_CONTRACTS[20],
    }
}

/// Build-time data representation of the same persistent JSON semantics.
/// Consumers explicitly export this data; the protocol does not inspect their source.
pub fn persistent_json_definitions() -> serde_json::Value {
    use serde_json::{Value, json};
    let fields: Vec<&str> = config::FIELD_DEFINITIONS
        .iter()
        .map(|field| field.key)
        .collect();
    let mut snapshot_schema = serde_json::to_value(schemars::schema_for!(ConfigSnapshot)).unwrap();
    snapshot_schema["properties"]["revision"] = json!({"type":"string","minLength":REVISION_HEX_BYTES,"maxLength":REVISION_HEX_BYTES,"pattern":"^[0-9a-f]+$"});
    snapshot_schema["properties"]["sunshine_version"] =
        json!({"type":"string","enum":SUPPORTED_SUNSHINE_VERSIONS});
    let properties: serde_json::Map<String, Value> = fields
        .iter()
        .map(|key| ((*key).to_owned(), json!({"type":"string"})))
        .collect();
    snapshot_schema["properties"]["fields"] =
        json!({"type":"object","additionalProperties":false,"properties":properties});
    let snapshot_rules = json!([{"kind":"object-value-string-bytes","fields":["/fields"],"arguments":{"minimum":0,"maximum":MAX_STORED_CONFIG_VALUE_BYTES}}]);
    let mut capabilities_schema =
        serde_json::to_value(schemars::schema_for!(Capabilities)).unwrap();
    capabilities_schema["properties"]["protocol"] = json!({"type":"string","const":PROTOCOL});
    capabilities_schema["properties"]["sunshine_version"] =
        json!({"type":"string","enum":SUPPORTED_SUNSHINE_VERSIONS});
    let capabilities_rules = json!([
        {"kind":"string-bytes","fields":["/client_version"],"arguments":{"minimum":1,"maximum":MAX_CLIENT_VERSION_BYTES}},
        {"kind":"string-trim-no-control","fields":["/client_version"],"arguments":{}},
        {"kind":"complete-string-set","fields":["/managed_fields"],"arguments":{"values":fields}}]);
    let snapshot = json!({"revision":"a".repeat(REVISION_HEX_BYTES),"sunshine_version":SUNSHINE_VERSION,"fields":{},"effectiveness":"pending_verification"});
    let capabilities = json!({"protocol":PROTOCOL,"client_version":"1.0.0","os":"linux_x86_64","sunshine_version":SUNSHINE_VERSION,"restart_allowed":false,"managed_fields":fields,"configuration_overwrite":false,"application_management":false,"application_host_commands_allowed":false,"moonlight_pairing_management":false,"pending_pairing_listing":false,"diagnostics":false,"maintenance":false,"service_control":false});
    let mut snapshot_cases = vec![json!({"name":"current","input":snapshot,"valid":true})];
    for (name, field, value) in [
        (
            "invalid-revision",
            "revision",
            json!("A".repeat(REVISION_HEX_BYTES)),
        ),
        (
            "unsupported-version",
            "sunshine_version",
            json!("unsupported"),
        ),
        ("unknown-field", "obsolete", json!(true)),
        ("unknown-managed-field", "fields", json!({"obsolete":"x"})),
        (
            "value-byte-bound",
            "fields",
            json!({(fields[0]):"é".repeat(MAX_STORED_CONFIG_VALUE_BYTES/2+1)}),
        ),
    ] {
        let mut value_case = snapshot.clone();
        value_case[field] = value;
        snapshot_cases.push(json!({"name":name,"input":value_case,"valid":false}));
    }
    let mut capabilities_cases = vec![json!({"name":"current","input":capabilities,"valid":true})];
    for (name, field, value) in [
        ("protocol", "protocol", json!("obsolete")),
        ("version-space", "client_version", json!(" bad")),
        ("version-control", "client_version", json!("bad\n")),
        (
            "version-bytes",
            "client_version",
            json!("é".repeat(MAX_CLIENT_VERSION_BYTES / 2 + 1)),
        ),
        ("managed-set", "managed_fields", json!([])),
        ("unknown-field", "obsolete", json!(true)),
    ] {
        let mut value_case = capabilities.clone();
        value_case[field] = value;
        capabilities_cases.push(json!({"name":name,"input":value_case,"valid":false}));
    }
    for case in &snapshot_cases {
        assert_eq!(
            serde_json::from_value::<ConfigSnapshot>(case["input"].clone())
                .is_ok_and(|value| validate_stored_snapshot(&value).is_ok()),
            case["valid"].as_bool().unwrap()
        );
    }
    for case in &capabilities_cases {
        assert_eq!(
            serde_json::from_value::<Capabilities>(case["input"].clone())
                .is_ok_and(|value| value.validate().is_ok()),
            case["valid"].as_bool().unwrap()
        );
    }
    json!({"snapshot":{"schema":snapshot_schema,"rules":snapshot_rules,"cases":snapshot_cases},"capabilities":{"schema":capabilities_schema,"rules":capabilities_rules,"cases":capabilities_cases}})
}
