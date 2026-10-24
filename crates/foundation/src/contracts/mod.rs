//! Strict Rust representations of the current Xcss JSON contracts.
//!
//! This crate intentionally models one wire version only. It contains no
//! aliases, fallback shapes or product-owned upgrade policy.
//! Unknown fields and values outside JavaScript's safe integer range fail
//! closed, keeping Rust consumers aligned with the TypeScript guards and the
//! published JSON Schemas in `web/contracts`.

use std::{collections::HashSet, fmt};

use crate::admin_auth::{
    ADMINISTRATOR_USERNAME_MAX_BYTES, is_token_shape, require_canonical_administrator_username,
};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{Error as _, Visitor},
    ser::{Error as _, SerializeStruct},
};
use thiserror::Error;

pub use crate::error::{
    ErrorCode, ErrorEnvelope, MAX_ERROR_CODE_BYTES, MAX_REQUEST_ID_BYTES, RequestId,
};
pub use crate::schema_identity::{Error as SchemaIdentityError, SchemaIdentity};

/// Largest integer represented exactly by every supported JSON consumer.
pub const MAX_SAFE_JSON_INTEGER: u64 = 9_007_199_254_740_991;

/// The only current product role. xcss contains no viewer/operator role.
pub const ADMINISTRATOR_ROLE: &str = "admin";

pub const ADMIN_LOGIN_PATH: &str = "/api/v1/auth/login";
pub const ADMIN_SESSION_PATH: &str = "/api/v1/auth/session";
pub const ADMIN_LOGOUT_PATH: &str = "/api/v1/auth/logout";

mod administrators;
pub use administrators::{ADMIN_ACCOUNT_PATH, ADMINISTRATORS_PATH, AdministratorAccountRequest};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum AdministratorRole {
    #[serde(rename = "admin")]
    Admin,
}

#[derive(Clone, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdministratorLoginRequest {
    // These are deliberately bounded untrusted candidates, not persisted
    // identities or credentials. `xcss::admin_auth` owns canonical username
    // normalization and the one current password/hash policy at admission.
    #[serde(deserialize_with = "deserialize_administrator_username_candidate")]
    pub username: String,
    #[serde(deserialize_with = "deserialize_secret_text")]
    pub password: String,
}

impl std::fmt::Debug for AdministratorLoginRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AdministratorLoginRequest")
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .finish()
    }
}

impl Serialize for AdministratorLoginRequest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.validate().map_err(S::Error::custom)?;
        let mut state = serializer.serialize_struct("AdministratorLoginRequest", 2)?;
        state.serialize_field("username", &self.username)?;
        state.serialize_field("password", &self.password)?;
        state.end()
    }
}

impl AdministratorLoginRequest {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_administrator_username_candidate(&self.username)?;
        validate_credential_text("password", &self.password, 1_024)
    }
}

#[derive(Clone, Eq, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AdministratorSession {
    #[serde(deserialize_with = "deserialize_authenticated_true")]
    pub authenticated: bool,
    #[serde(deserialize_with = "deserialize_identifier")]
    pub user_id: String,
    #[serde(deserialize_with = "deserialize_canonical_administrator_username")]
    pub username: String,
    pub role: AdministratorRole,
    #[serde(deserialize_with = "deserialize_authentication_token")]
    pub csrf_token: String,
}

impl std::fmt::Debug for AdministratorSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AdministratorSession")
            .field("authenticated", &self.authenticated)
            .field("user_id", &self.user_id)
            .field("username", &self.username)
            .field("role", &self.role)
            .field("csrf_token", &"[REDACTED]")
            .finish()
    }
}

impl Serialize for AdministratorSession {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.validate().map_err(S::Error::custom)?;
        let mut state = serializer.serialize_struct("AdministratorSession", 5)?;
        state.serialize_field("authenticated", &true)?;
        state.serialize_field("user_id", &self.user_id)?;
        state.serialize_field("username", &self.username)?;
        state.serialize_field("role", &self.role)?;
        state.serialize_field("csrf_token", &self.csrf_token)?;
        state.end()
    }
}

impl AdministratorSession {
    pub fn new(
        user_id: impl Into<String>,
        username: impl Into<String>,
        csrf_token: impl Into<String>,
    ) -> Result<Self, ValidationError> {
        let value = Self {
            authenticated: true,
            user_id: user_id.into(),
            username: username.into(),
            role: AdministratorRole::Admin,
            csrf_token: csrf_token.into(),
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        if !self.authenticated {
            return Err(ValidationError::NotAuthenticated);
        }
        validate_identifier("user_id", &self.user_id)?;
        require_canonical_administrator_username(&self.username)
            .map_err(|_| ValidationError::InvalidAdministratorUsername)?;
        if !is_token_shape(&self.csrf_token) {
            return Err(ValidationError::InvalidAuthenticationToken {
                field: "csrf_token",
            });
        }
        Ok(())
    }
}

/// Current state-contract wire version.
pub const STATE_CONTRACT_VERSION: u8 = 1;

/// Current backup-manifest wire version.
pub const BACKUP_MANIFEST_VERSION: u8 = 1;

/// The complete metadata identity used by backup manifests.
///
/// This is deliberately the canonical type from `xcss::schema_identity`, not
/// a second copy of the same four fields.
pub type BackupSchemaIdentity = SchemaIdentity;

/// The smaller schema reference embedded in a state contract.
///
/// Unlike [`SchemaIdentity`], the owning application's name and version are
/// already carried by [`StateContract`], so the wire object has two fields.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateSchemaIdentity {
    #[serde(deserialize_with = "deserialize_non_negative_safe_integer")]
    pub revision: u64,
    #[serde(deserialize_with = "deserialize_sha256")]
    pub sha256: String,
}

impl StateSchemaIdentity {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_non_negative_safe_integer("schema.revision", self.revision)?;
        validate_sha256("schema.sha256", &self.sha256)
    }
}

/// Kinds of state that may participate in backup and restore.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum StateResourceKind {
    #[serde(rename = "sqlite")]
    Sqlite,
    #[serde(rename = "configuration")]
    Configuration,
    #[serde(rename = "data-tree")]
    DataTree,
    #[serde(rename = "recordings")]
    Recordings,
    #[serde(rename = "companion-contract")]
    CompanionContract,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateResource {
    #[serde(deserialize_with = "deserialize_identifier")]
    pub name: String,
    pub kind: StateResourceKind,
    pub required: bool,
}

impl StateResource {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_identifier("resources[].name", &self.name)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateExternalRequirement {
    #[serde(deserialize_with = "deserialize_identifier")]
    pub kind: String,
    #[serde(deserialize_with = "deserialize_identifier")]
    pub kid: String,
    #[serde(deserialize_with = "deserialize_identifier")]
    pub algorithm: String,
    #[serde(deserialize_with = "deserialize_positive_safe_integer")]
    pub envelope_version: u64,
}

impl StateExternalRequirement {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_identifier("external_requirements[].kind", &self.kind)?;
        validate_identifier("external_requirements[].kid", &self.kid)?;
        validate_identifier("external_requirements[].algorithm", &self.algorithm)?;
        validate_positive_safe_integer(
            "external_requirements[].envelope_version",
            self.envelope_version,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompanionContract {
    #[serde(deserialize_with = "deserialize_identifier")]
    pub name: String,
    #[serde(deserialize_with = "deserialize_identifier")]
    pub version: String,
    #[serde(deserialize_with = "deserialize_identifier")]
    pub platform: String,
    #[serde(deserialize_with = "deserialize_sha256")]
    pub sha256: String,
}

impl CompanionContract {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_identifier("companion_contracts[].name", &self.name)?;
        validate_identifier("companion_contracts[].version", &self.version)?;
        validate_identifier("companion_contracts[].platform", &self.platform)?;
        validate_sha256("companion_contracts[].sha256", &self.sha256)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StateContract {
    #[serde(deserialize_with = "deserialize_state_contract_version")]
    pub contract_version: u8,
    #[serde(deserialize_with = "deserialize_identifier")]
    pub application: String,
    #[serde(deserialize_with = "deserialize_identifier")]
    pub application_version: String,
    #[serde(deserialize_with = "deserialize_source_revision")]
    pub source_revision: String,
    #[serde(deserialize_with = "deserialize_required_state_schema")]
    pub schema: Option<StateSchemaIdentity>,
    #[serde(deserialize_with = "deserialize_maintenance_locks")]
    pub maintenance_locks: Vec<String>,
    pub resources: Vec<StateResource>,
    pub external_requirements: Vec<StateExternalRequirement>,
    pub companion_contracts: Vec<CompanionContract>,
}

impl StateContract {
    /// Parse and validate one current state-contract JSON document.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, ParseError> {
        let value: Self = serde_json::from_slice(bytes)?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_exact_version(
            "contract_version",
            u64::from(self.contract_version),
            u64::from(STATE_CONTRACT_VERSION),
        )?;
        validate_identifier("application", &self.application)?;
        validate_identifier("application_version", &self.application_version)?;
        validate_source_revision("source_revision", &self.source_revision)?;
        if let Some(schema) = &self.schema {
            schema.validate()?;
        }
        validate_unique_identifiers("maintenance_locks", &self.maintenance_locks)?;
        for resource in &self.resources {
            resource.validate()?;
        }
        for requirement in &self.external_requirements {
            requirement.validate()?;
        }
        for companion in &self.companion_contracts {
            companion.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReleaseIdentity {
    #[serde(deserialize_with = "deserialize_identifier")]
    pub product: String,
    #[serde(deserialize_with = "deserialize_identifier")]
    pub version: String,
    #[serde(deserialize_with = "deserialize_source_revision")]
    pub source_revision: String,
    #[serde(deserialize_with = "deserialize_identifier")]
    pub target: String,
    #[serde(deserialize_with = "deserialize_sha256")]
    pub state_contract_sha256: String,
}

impl ReleaseIdentity {
    /// Parse and validate one current release-identity JSON document.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, ParseError> {
        let value: Self = serde_json::from_slice(bytes)?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_identifier("product", &self.product)?;
        validate_identifier("version", &self.version)?;
        validate_source_revision("source_revision", &self.source_revision)?;
        validate_identifier("target", &self.target)?;
        validate_sha256("state_contract_sha256", &self.state_contract_sha256)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupExternalRequirement {
    #[serde(deserialize_with = "deserialize_identifier")]
    pub kind: String,
    #[serde(deserialize_with = "deserialize_identifier")]
    pub kid: String,
    #[serde(deserialize_with = "deserialize_sha256")]
    pub sha256: String,
    #[serde(deserialize_with = "deserialize_identifier")]
    pub algorithm: String,
    #[serde(deserialize_with = "deserialize_positive_safe_integer")]
    pub envelope_version: u64,
}

impl BackupExternalRequirement {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_identifier("external_requirements[].kind", &self.kind)?;
        validate_identifier("external_requirements[].kid", &self.kid)?;
        validate_sha256("external_requirements[].sha256", &self.sha256)?;
        validate_identifier("external_requirements[].algorithm", &self.algorithm)?;
        validate_positive_safe_integer(
            "external_requirements[].envelope_version",
            self.envelope_version,
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupResource {
    #[serde(deserialize_with = "deserialize_identifier")]
    pub name: String,
    pub kind: StateResourceKind,
    #[serde(deserialize_with = "deserialize_non_empty_path")]
    pub path: String,
    #[serde(deserialize_with = "deserialize_non_negative_safe_integer")]
    pub bytes: u64,
    #[serde(deserialize_with = "deserialize_positive_safe_integer")]
    pub files: u64,
    #[serde(deserialize_with = "deserialize_sha256")]
    pub sha256: String,
}

impl BackupResource {
    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_identifier("resources[].name", &self.name)?;
        validate_non_empty_path("resources[].path", &self.path)?;
        validate_non_negative_safe_integer("resources[].bytes", self.bytes)?;
        validate_positive_safe_integer("resources[].files", self.files)?;
        validate_sha256("resources[].sha256", &self.sha256)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackupManifest {
    #[serde(deserialize_with = "deserialize_backup_manifest_version")]
    pub manifest_version: u8,
    #[serde(deserialize_with = "deserialize_identifier")]
    pub tool_version: String,
    #[serde(deserialize_with = "deserialize_identifier")]
    pub product: String,
    #[serde(deserialize_with = "deserialize_identifier")]
    pub application_version: String,
    #[serde(deserialize_with = "deserialize_required_schema_identity")]
    pub schema_identity: Option<SchemaIdentity>,
    #[serde(deserialize_with = "deserialize_non_negative_safe_integer")]
    pub created_at_epoch_seconds: u64,
    pub external_requirements: Vec<BackupExternalRequirement>,
    #[serde(deserialize_with = "deserialize_backup_resources")]
    pub resources: Vec<BackupResource>,
}

impl BackupManifest {
    /// Parse and validate one current backup-manifest JSON document.
    pub fn from_slice(bytes: &[u8]) -> Result<Self, ParseError> {
        let value: Self = serde_json::from_slice(bytes)?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), ValidationError> {
        validate_exact_version(
            "manifest_version",
            u64::from(self.manifest_version),
            u64::from(BACKUP_MANIFEST_VERSION),
        )?;
        validate_identifier("tool_version", &self.tool_version)?;
        validate_identifier("product", &self.product)?;
        validate_identifier("application_version", &self.application_version)?;
        if let Some(identity) = &self.schema_identity {
            identity.validate()?;
            validate_non_negative_safe_integer(
                "schema_identity.schema_revision",
                identity.schema_revision,
            )?;
        }
        validate_non_negative_safe_integer(
            "created_at_epoch_seconds",
            self.created_at_epoch_seconds,
        )?;
        for requirement in &self.external_requirements {
            requirement.validate()?;
        }
        if self.resources.is_empty() {
            return Err(ValidationError::EmptyArray { field: "resources" });
        }
        for resource in &self.resources {
            resource.validate()?;
        }
        Ok(())
    }
}

/// Parse the shared error envelope through the canonical `xcss::error` type.
pub fn error_envelope_from_slice(bytes: &[u8]) -> Result<ErrorEnvelope, ParseError> {
    Ok(serde_json::from_slice(bytes)?)
}

#[derive(Debug, Error)]
pub enum ParseError {
    #[error("invalid contract JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Validation(#[from] ValidationError),
}

#[derive(Clone, Debug, Eq, PartialEq, Error)]
pub enum ValidationError {
    #[error("{field} must contain 1 to 128 ASCII letters, digits, '.', '_', ':' or '-'")]
    InvalidIdentifier { field: &'static str },
    #[error("{field} must be exactly 40 lowercase hexadecimal characters")]
    InvalidSourceRevision { field: &'static str },
    #[error("{field} must be exactly 64 lowercase hexadecimal characters")]
    InvalidSha256 { field: &'static str },
    #[error("{field} exceeds the maximum safe JSON integer ({MAX_SAFE_JSON_INTEGER}): {value}")]
    UnsafeInteger { field: &'static str, value: u64 },
    #[error("{field} must be a positive safe JSON integer")]
    NonPositiveInteger { field: &'static str },
    #[error("{field} must equal {expected}, found {actual}")]
    UnsupportedVersion {
        field: &'static str,
        expected: u64,
        actual: u64,
    },
    #[error("{field} contains duplicate value {value:?}")]
    DuplicateValue { field: &'static str, value: String },
    #[error("{field} must contain at least one entry")]
    EmptyArray { field: &'static str },
    #[error("{field} must not be empty")]
    EmptyPath { field: &'static str },
    #[error("authenticated must be true in an administrator session")]
    NotAuthenticated,
    #[error("administrator session username must use the canonical current administrator identity")]
    InvalidAdministratorUsername,
    #[error(
        "administrator login username must contain 1 to {ADMINISTRATOR_USERNAME_MAX_BYTES} non-control ASCII bytes"
    )]
    InvalidAdministratorUsernameCandidate,
    #[error("{field} must be one current 256-bit unpadded base64url authentication token")]
    InvalidAuthenticationToken { field: &'static str },
    #[error("{field} must contain 1 to {maximum} non-control Unicode code points")]
    InvalidCredentialText { field: &'static str, maximum: usize },
    #[error("invalid schema identity: {0}")]
    SchemaIdentity(#[from] SchemaIdentityError),
}

fn validate_identifier(field: &'static str, value: &str) -> Result<(), ValidationError> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-'))
    {
        return Err(ValidationError::InvalidIdentifier { field });
    }
    Ok(())
}

fn validate_credential_text(
    field: &'static str,
    value: &str,
    maximum: usize,
) -> Result<(), ValidationError> {
    let mut length = 0usize;
    for character in value.chars() {
        if character.is_ascii_control() {
            return Err(ValidationError::InvalidCredentialText { field, maximum });
        }
        length += 1;
        if length > maximum {
            return Err(ValidationError::InvalidCredentialText { field, maximum });
        }
    }
    if length == 0 {
        return Err(ValidationError::InvalidCredentialText { field, maximum });
    }
    Ok(())
}

fn validate_administrator_username_candidate(value: &str) -> Result<(), ValidationError> {
    if value.is_empty()
        || value.len() > ADMINISTRATOR_USERNAME_MAX_BYTES
        || !value.is_ascii()
        || value.bytes().any(|byte| byte.is_ascii_control())
    {
        return Err(ValidationError::InvalidAdministratorUsernameCandidate);
    }
    Ok(())
}

fn validate_source_revision(field: &'static str, value: &str) -> Result<(), ValidationError> {
    if value.len() != 40 || !value.bytes().all(is_lower_hex) {
        return Err(ValidationError::InvalidSourceRevision { field });
    }
    Ok(())
}

fn validate_sha256(field: &'static str, value: &str) -> Result<(), ValidationError> {
    if value.len() != 64 || !value.bytes().all(is_lower_hex) {
        return Err(ValidationError::InvalidSha256 { field });
    }
    Ok(())
}

fn is_lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
}

fn validate_non_negative_safe_integer(
    field: &'static str,
    value: u64,
) -> Result<(), ValidationError> {
    if value > MAX_SAFE_JSON_INTEGER {
        return Err(ValidationError::UnsafeInteger { field, value });
    }
    Ok(())
}

fn validate_positive_safe_integer(field: &'static str, value: u64) -> Result<(), ValidationError> {
    if value == 0 {
        return Err(ValidationError::NonPositiveInteger { field });
    }
    validate_non_negative_safe_integer(field, value)
}

fn validate_exact_version(
    field: &'static str,
    actual: u64,
    expected: u64,
) -> Result<(), ValidationError> {
    if actual != expected {
        return Err(ValidationError::UnsupportedVersion {
            field,
            expected,
            actual,
        });
    }
    Ok(())
}

fn validate_non_empty_path(field: &'static str, value: &str) -> Result<(), ValidationError> {
    if value.is_empty() {
        return Err(ValidationError::EmptyPath { field });
    }
    Ok(())
}

fn validate_unique_identifiers(
    field: &'static str,
    values: &[String],
) -> Result<(), ValidationError> {
    let mut unique = HashSet::with_capacity(values.len());
    for value in values {
        validate_identifier(field, value)?;
        if !unique.insert(value.as_str()) {
            return Err(ValidationError::DuplicateValue {
                field,
                value: value.clone(),
            });
        }
    }
    Ok(())
}

fn deserialize_identifier<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    validate_identifier("identifier", &value).map_err(D::Error::custom)?;
    Ok(value)
}

fn deserialize_authenticated_true<'de, D>(deserializer: D) -> Result<bool, D::Error>
where
    D: Deserializer<'de>,
{
    let value = bool::deserialize(deserializer)?;
    if !value {
        return Err(D::Error::custom(ValidationError::NotAuthenticated));
    }
    Ok(value)
}

fn deserialize_administrator_username_candidate<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    validate_administrator_username_candidate(&value).map_err(D::Error::custom)?;
    Ok(value)
}

fn deserialize_canonical_administrator_username<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    require_canonical_administrator_username(&value)
        .map_err(|_| D::Error::custom(ValidationError::InvalidAdministratorUsername))?;
    Ok(value)
}

fn deserialize_authentication_token<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if !is_token_shape(&value) {
        return Err(D::Error::custom(
            ValidationError::InvalidAuthenticationToken {
                field: "authentication token",
            },
        ));
    }
    Ok(value)
}

fn deserialize_secret_text<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    validate_credential_text("secret", &value, 1_024).map_err(D::Error::custom)?;
    Ok(value)
}

fn deserialize_source_revision<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    validate_source_revision("source_revision", &value).map_err(D::Error::custom)?;
    Ok(value)
}

fn deserialize_sha256<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    validate_sha256("sha256", &value).map_err(D::Error::custom)?;
    Ok(value)
}

fn deserialize_non_negative_safe_integer<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(SafeIntegerVisitor { minimum: 0 })
}

fn deserialize_positive_safe_integer<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(SafeIntegerVisitor { minimum: 1 })
}

struct SafeIntegerVisitor {
    minimum: u64,
}

impl SafeIntegerVisitor {
    fn accept<E>(self, value: u64) -> Result<u64, E>
    where
        E: serde::de::Error,
    {
        if value < self.minimum || value > MAX_SAFE_JSON_INTEGER {
            return Err(E::custom(self.expectation()));
        }
        Ok(value)
    }

    fn expectation(&self) -> String {
        if self.minimum == 0 {
            format!("an integer from 0 through {MAX_SAFE_JSON_INTEGER}")
        } else {
            format!("an integer from 1 through {MAX_SAFE_JSON_INTEGER}")
        }
    }
}

impl<'de> Visitor<'de> for SafeIntegerVisitor {
    type Value = u64;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.expectation())
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        self.accept(value)
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        let value = u64::try_from(value).map_err(|_| E::custom(self.expectation()))?;
        self.accept(value)
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: serde::de::Error,
    {
        if !value.is_finite()
            || value.fract() != 0.0
            || value < self.minimum as f64
            || value > MAX_SAFE_JSON_INTEGER as f64
        {
            return Err(E::custom(self.expectation()));
        }
        Ok(value as u64)
    }
}

fn deserialize_state_contract_version<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    let value = deserialize_non_negative_safe_integer(deserializer)?;
    validate_exact_version("contract_version", value, u64::from(STATE_CONTRACT_VERSION))
        .map_err(D::Error::custom)?;
    Ok(STATE_CONTRACT_VERSION)
}

fn deserialize_backup_manifest_version<'de, D>(deserializer: D) -> Result<u8, D::Error>
where
    D: Deserializer<'de>,
{
    let value = deserialize_non_negative_safe_integer(deserializer)?;
    validate_exact_version(
        "manifest_version",
        value,
        u64::from(BACKUP_MANIFEST_VERSION),
    )
    .map_err(D::Error::custom)?;
    Ok(BACKUP_MANIFEST_VERSION)
}

fn deserialize_required_state_schema<'de, D>(
    deserializer: D,
) -> Result<Option<StateSchemaIdentity>, D::Error>
where
    D: Deserializer<'de>,
{
    Option::<StateSchemaIdentity>::deserialize(deserializer)
}

fn deserialize_required_schema_identity<'de, D>(
    deserializer: D,
) -> Result<Option<SchemaIdentity>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct WireSchemaIdentity {
        #[serde(deserialize_with = "deserialize_identifier")]
        application: String,
        #[serde(deserialize_with = "deserialize_identifier")]
        application_version: String,
        #[serde(deserialize_with = "deserialize_non_negative_safe_integer")]
        schema_revision: u64,
        #[serde(deserialize_with = "deserialize_sha256")]
        schema_sha256: String,
    }

    Option::<WireSchemaIdentity>::deserialize(deserializer)?
        .map(|wire| {
            SchemaIdentity::new(
                wire.application,
                wire.application_version,
                wire.schema_revision,
                wire.schema_sha256,
            )
            .map_err(D::Error::custom)
        })
        .transpose()
}

fn deserialize_maintenance_locks<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let values = Vec::<String>::deserialize(deserializer)?;
    validate_unique_identifiers("maintenance_locks", &values).map_err(D::Error::custom)?;
    Ok(values)
}

fn deserialize_non_empty_path<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    validate_non_empty_path("path", &value).map_err(D::Error::custom)?;
    Ok(value)
}

fn deserialize_backup_resources<'de, D>(deserializer: D) -> Result<Vec<BackupResource>, D::Error>
where
    D: Deserializer<'de>,
{
    let resources = Vec::<BackupResource>::deserialize(deserializer)?;
    if resources.is_empty() {
        return Err(D::Error::custom(ValidationError::EmptyArray {
            field: "resources",
        }));
    }
    Ok(resources)
}

#[cfg(test)]
mod tests {
    use serde::Deserialize;
    use serde_json::{Value, json};

    use super::*;

    #[test]
    fn administrator_debug_does_not_log_password_or_csrf_credentials() {
        let login = AdministratorLoginRequest {
            username: "admin".into(),
            password: "DO_NOT_LOG_PASSWORD".into(),
        };
        assert!(!format!("{login:?}").contains("DO_NOT_LOG_PASSWORD"));
        let token = "A".repeat(43);
        let session = AdministratorSession::new("admin-1", "admin", &token).unwrap();
        assert!(!format!("{session:?}").contains(&token));
        assert!(format!("{session:?}").contains("[REDACTED]"));
    }

    const ERROR_FIXTURES: &str =
        include_str!("../../web/contracts/fixtures/error-envelope.fixtures.json");
    const ADMIN_AUTH_FIXTURES: &str =
        include_str!("../../web/contracts/fixtures/administrator-auth.fixtures.json");
    const STATE_FIXTURES: &str =
        include_str!("../../web/contracts/fixtures/state-contract.fixtures.json");
    const RELEASE_FIXTURES: &str =
        include_str!("../../web/contracts/fixtures/release.fixtures.json");
    const BACKUP_FIXTURES: &str =
        include_str!("../../web/contracts/fixtures/backup-manifest.fixtures.json");

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct FixtureSet {
        valid: Vec<Value>,
        invalid: Vec<Value>,
    }

    #[test]
    fn error_envelope_matches_every_shared_fixture() {
        assert_fixtures(ERROR_FIXTURES, error_envelope_from_slice);
    }

    #[test]
    fn administrator_auth_matches_every_shared_fixture() {
        let fixtures: FixtureSet =
            serde_json::from_str(ADMIN_AUTH_FIXTURES).expect("fixture JSON is valid");
        for (index, value) in fixtures.valid.into_iter().enumerate() {
            let accepted = serde_json::from_value::<AdministratorLoginRequest>(value.clone())
                .is_ok()
                || serde_json::from_value::<AdministratorSession>(value).is_ok();
            assert!(accepted, "rejected valid administrator fixture {index}");
        }
        for (index, value) in fixtures.invalid.into_iter().enumerate() {
            let accepted = serde_json::from_value::<AdministratorLoginRequest>(value.clone())
                .is_ok()
                || serde_json::from_value::<AdministratorSession>(value).is_ok();
            assert!(!accepted, "accepted invalid administrator fixture {index}");
        }
    }

    #[test]
    fn administrator_session_cannot_serialize_an_invalid_public_value() {
        let mut session = AdministratorSession::new(
            "018f1f4b-7a5d-7b5f-8d31-123456789abc",
            "admin",
            "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(&session).unwrap()["role"],
            json!(ADMINISTRATOR_ROLE)
        );

        session.authenticated = false;
        assert!(serde_json::to_value(&session).is_err());

        session.authenticated = true;
        session.username = "Admin".to_owned();
        assert!(serde_json::to_value(&session).is_err());

        session.username = "admin".to_owned();
        session.csrf_token = "noncanonical-token-shape".to_owned();
        assert!(serde_json::to_value(&session).is_err());
    }

    #[test]
    fn administrator_login_cannot_serialize_an_invalid_public_value() {
        let mut login = AdministratorLoginRequest {
            username: " Admin ".to_owned(),
            password: "bounded wire candidate".to_owned(),
        };
        assert_eq!(
            serde_json::to_value(&login).unwrap(),
            json!({
                "username": " Admin ",
                "password": "bounded wire candidate"
            })
        );

        login.username = "管理员".to_owned();
        assert!(serde_json::to_value(&login).is_err());

        login.username = "admin".to_owned();
        login.password.clear();
        assert!(serde_json::to_value(&login).is_err());
    }

    #[test]
    fn state_contract_matches_every_shared_fixture() {
        assert_fixtures(STATE_FIXTURES, StateContract::from_slice);
    }

    #[test]
    fn release_identity_matches_every_shared_fixture() {
        assert_fixtures(RELEASE_FIXTURES, ReleaseIdentity::from_slice);
    }

    #[test]
    fn backup_manifest_matches_every_shared_fixture() {
        assert_fixtures(BACKUP_FIXTURES, BackupManifest::from_slice);
    }

    fn assert_fixtures<T, E>(fixtures: &str, parse: impl Fn(&[u8]) -> Result<T, E>)
    where
        E: std::fmt::Display,
    {
        let fixtures: FixtureSet = serde_json::from_str(fixtures).expect("fixture JSON is valid");
        for (index, value) in fixtures.valid.into_iter().enumerate() {
            let bytes = serde_json::to_vec(&value).expect("fixture value serializes");
            if let Err(error) = parse(&bytes) {
                panic!("rejected valid shared fixture {index}: {error}");
            }
        }
        for (index, value) in fixtures.invalid.into_iter().enumerate() {
            let bytes = serde_json::to_vec(&value).expect("fixture value serializes");
            assert!(
                parse(&bytes).is_err(),
                "accepted invalid shared fixture {index}"
            );
        }
    }

    #[test]
    fn required_nullable_fields_cannot_be_omitted() {
        let state = json!({
            "contract_version": 1,
            "application": "sample-product",
            "application_version": "0.3.0",
            "source_revision": "a".repeat(40),
            "maintenance_locks": [],
            "resources": [],
            "external_requirements": [],
            "companion_contracts": []
        });
        assert!(StateContract::from_slice(&serde_json::to_vec(&state).unwrap()).is_err());

        let backup = json!({
            "manifest_version": 1,
            "tool_version": "0.3.0",
            "product": "sample-product",
            "application_version": "0.3.0",
            "created_at_epoch_seconds": 0,
            "external_requirements": [],
            "resources": [{
                "name": "database",
                "kind": "sqlite",
                "path": "database.sqlite3",
                "bytes": 0,
                "files": 1,
                "sha256": "a".repeat(64)
            }]
        });
        assert!(BackupManifest::from_slice(&serde_json::to_vec(&backup).unwrap()).is_err());
    }

    #[test]
    fn semantic_boundaries_match_the_json_schemas() {
        let release = ReleaseIdentity {
            product: "sample-product".to_owned(),
            version: "0.3.0".to_owned(),
            source_revision: "a".repeat(40),
            target: "x86_64-unknown-linux-gnu".to_owned(),
            state_contract_sha256: "b".repeat(64),
        };
        release.validate().unwrap();

        let mut invalid_revision = release.clone();
        invalid_revision.source_revision = "A".repeat(40);
        assert!(matches!(
            invalid_revision.validate(),
            Err(ValidationError::InvalidSourceRevision { .. })
        ));

        let schema = StateSchemaIdentity {
            revision: MAX_SAFE_JSON_INTEGER,
            sha256: "c".repeat(64),
        };
        schema.validate().unwrap();
        let too_large = StateSchemaIdentity {
            revision: MAX_SAFE_JSON_INTEGER + 1,
            sha256: "c".repeat(64),
        };
        assert!(matches!(
            too_large.validate(),
            Err(ValidationError::UnsafeInteger { .. })
        ));
    }

    #[test]
    fn mathematically_integral_json_numbers_match_javascript_guards() {
        let state = format!(
            r#"{{
                "contract_version": 1.0,
                "application": "sample-product",
                "application_version": "0.3.0",
                "source_revision": "{}",
                "schema": {{"revision": 3e0, "sha256": "{}"}},
                "maintenance_locks": [],
                "resources": [],
                "external_requirements": [],
                "companion_contracts": []
            }}"#,
            "a".repeat(40),
            "b".repeat(64)
        );
        StateContract::from_slice(state.as_bytes()).unwrap();

        let backup = format!(
            r#"{{
                "manifest_version": 1.0,
                "tool_version": "0.3.0",
                "product": "sample-product",
                "application_version": "0.3.0",
                "schema_identity": {{
                    "application": "sample-product",
                    "application_version": "0.3.0",
                    "schema_revision": 4.0,
                    "schema_sha256": "{}"
                }},
                "created_at_epoch_seconds": 0e0,
                "external_requirements": [],
                "resources": [{{
                    "name": "database",
                    "kind": "sqlite",
                    "path": "database.sqlite3",
                    "bytes": 0.0,
                    "files": 1e0,
                    "sha256": "{}"
                }}]
            }}"#,
            "a".repeat(64),
            "b".repeat(64)
        );
        BackupManifest::from_slice(backup.as_bytes()).unwrap();

        let fractional = state.replace("\"revision\": 3e0", "\"revision\": 3.5");
        assert!(StateContract::from_slice(fractional.as_bytes()).is_err());
    }

    #[test]
    fn current_contract_does_not_invent_product_owned_array_or_path_rules() {
        let manifest = json!({
            "manifest_version": 1,
            "tool_version": "0.3.0",
            "product": "sample-product",
            "application_version": "0.3.0",
            "schema_identity": null,
            "created_at_epoch_seconds": 0,
            "external_requirements": [],
            "resources": [
                {
                    "name": "z-resource",
                    "kind": "data-tree",
                    "path": "../product-owned-path",
                    "bytes": 0,
                    "files": 1,
                    "sha256": "a".repeat(64)
                },
                {
                    "name": "z-resource",
                    "kind": "data-tree",
                    "path": "../product-owned-path",
                    "bytes": 0,
                    "files": 1,
                    "sha256": "a".repeat(64)
                }
            ]
        });
        BackupManifest::from_slice(&serde_json::to_vec(&manifest).unwrap()).unwrap();
    }

    #[test]
    fn only_the_current_versions_are_accepted() {
        let state = serde_json::from_str::<FixtureSet>(STATE_FIXTURES)
            .unwrap()
            .valid
            .remove(0);
        for version in [0, 2] {
            let mut value = state.clone();
            value["contract_version"] = json!(version);
            assert!(StateContract::from_slice(&serde_json::to_vec(&value).unwrap()).is_err());
        }

        let release = serde_json::from_str::<FixtureSet>(RELEASE_FIXTURES)
            .unwrap()
            .valid
            .remove(0);
        let mut unknown = release;
        unknown["upgrade_tool"] = json!("unexpected");
        assert!(ReleaseIdentity::from_slice(&serde_json::to_vec(&unknown).unwrap()).is_err());
    }
}
