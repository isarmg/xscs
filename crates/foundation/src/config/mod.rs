//! Current JSON configuration with explicit, typed sources. Product Serde
//! definitions own fields, defaults and business validation; this crate owns
//! strict parsing, precedence, provenance and safe contract diagnostics.

mod managed;
mod strict;
pub use managed::read_managed_file;
#[cfg(test)]
mod tests;

use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{collections::BTreeMap, fmt};

pub const MAX_CONFIG_BYTES: usize = 1024 * 1024;
pub const MAX_OVERRIDES: usize = 256;

/// Read one existing, private configuration file without creating directories,
/// locks or files. The directory must be mode 0700 and the single-linked file
/// mode 0600, both owned by this process user. Path replacement or mutation
/// during the bounded read is rejected; file contents never enter diagnostics.
#[cfg(unix)]
pub fn read_private_file(path: &std::path::Path) -> Result<Vec<u8>, ConfigError> {
    use std::{
        fs,
        io::Read,
        os::unix::fs::{MetadataExt, OpenOptionsExt},
    };
    let invalid = || ConfigError::new(Reason::UnsafeFile, "", ConfigSource::File);
    let read_failed = || ConfigError::new(Reason::ReadFailed, "", ConfigSource::File);
    let parent = path.parent().ok_or_else(invalid)?;
    let directory =
        crate::state_file::PrivateStateDirectory::open(parent).map_err(|_| invalid())?;
    let mut file = fs::OpenOptions::new()
        .read(true)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open(path)
        .map_err(|error| {
            ConfigError::new(
                if error.kind() == std::io::ErrorKind::NotFound {
                    Reason::MissingFile
                } else if error.raw_os_error() == Some(rustix::io::Errno::LOOP.raw_os_error()) {
                    Reason::UnsafeFile
                } else {
                    Reason::ReadFailed
                },
                "",
                ConfigSource::File,
            )
        })?;
    let before = file.metadata().map_err(|_| read_failed())?;
    if !before.is_file()
        || before.nlink() != 1
        || before.mode() & 0o777 != 0o600
        || before.uid() != directory.owner_uid()
    {
        return Err(invalid());
    }
    if before.len() > MAX_CONFIG_BYTES as u64 {
        return Err(ConfigError::new(
            Reason::LimitExceeded,
            "",
            ConfigSource::File,
        ));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_CONFIG_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| read_failed())?;
    if bytes.len() > MAX_CONFIG_BYTES {
        return Err(ConfigError::new(
            Reason::LimitExceeded,
            "",
            ConfigSource::File,
        ));
    }
    let stamp = |metadata: &fs::Metadata| {
        (
            metadata.dev(),
            metadata.ino(),
            metadata.len(),
            metadata.mtime(),
            metadata.mtime_nsec(),
            metadata.ctime(),
            metadata.ctime_nsec(),
            metadata.uid(),
            metadata.mode(),
            metadata.nlink(),
        )
    };
    let after = file.metadata().map_err(|_| read_failed())?;
    let named = fs::symlink_metadata(path).map_err(|_| read_failed())?;
    if stamp(&before) != stamp(&after)
        || stamp(&before) != stamp(&named)
        || bytes.len() as u64 != before.len()
    {
        return Err(ConfigError::new(
            Reason::SourceChanged,
            "",
            ConfigSource::File,
        ));
    }
    directory.verify_identity().map_err(|_| invalid())?;
    Ok(bytes)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    Default,
    File,
    Environment,
    CommandLine,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Reason {
    InvalidSyntax,
    DuplicateField,
    UnknownField,
    MissingField,
    TypeMismatch,
    InvalidValue,
    InvalidMapping,
    LimitExceeded,
    MissingFile,
    UnsafeFile,
    ReadFailed,
    SourceChanged,
}

/// Safe to emit to a terminal or HTTP client: input values and parser text are
/// deliberately excluded, including invalid enum values and duplicate keys.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ConfigError {
    pub reason: Reason,
    pub path: String,
    pub source: ConfigSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub column: Option<usize>,
}

impl ConfigError {
    pub fn new(reason: Reason, path: impl Into<String>, source: ConfigSource) -> Self {
        Self {
            reason,
            path: path.into(),
            source,
            line: None,
            column: None,
        }
    }

    pub fn envelope(&self) -> crate::error::ErrorEnvelope {
        let (code, message) = match self.reason {
            Reason::MissingFile => (
                "config_file_not_found",
                "The selected configuration file does not exist.",
            ),
            Reason::UnsafeFile => (
                "unsafe_config_file",
                "The configuration path does not satisfy its ownership and permission contract.",
            ),
            Reason::ReadFailed => (
                "config_read_failed",
                "The selected configuration file could not be read.",
            ),
            Reason::SourceChanged => (
                "config_source_changed",
                "The configuration source changed during inspection.",
            ),
            _ => (
                "contract_violation",
                "Configuration does not satisfy the current contract.",
            ),
        };
        crate::error::ErrorEnvelope::with_code(
            crate::error::ErrorCode::new(code).expect("static public error code is valid"),
            message,
        )
        .with_detail("violations", json!([self]))
    }
}

impl fmt::Display for ConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "configuration {:?} at {} ({:?})",
            self.reason, self.path, self.source
        )
    }
}
impl std::error::Error for ConfigError {}

/// A JSON pointer names a current field. Arrays are replaced as a complete
/// field, so overrides cannot partially patch indexed business records.
#[derive(Clone)]
pub struct Override {
    pub path: String,
    pub value: Value,
}

impl Override {
    pub fn new(path: impl Into<String>, value: impl Into<Value>) -> Self {
        Self {
            path: path.into(),
            value: value.into(),
        }
    }
}

// Never accidentally expose a password supplied by CLI or environment to logs.
impl fmt::Debug for Override {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Override")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnvValueKind {
    String,
    Boolean,
    UnsignedInteger,
    SignedInteger,
    Json,
}

#[derive(Clone, Copy, Debug)]
pub struct EnvMapping<'a> {
    pub variable: &'a str,
    pub path: &'a str,
    pub kind: EnvValueKind,
}

/// Read only declared names. The caller supplies lookup, so tests and embedded
/// applications never have to mutate process-global environment variables.
pub fn read_environment(
    mappings: &[EnvMapping<'_>],
    mut lookup: impl FnMut(&str) -> Option<String>,
) -> Result<Vec<Override>, ConfigError> {
    if mappings.len() > MAX_OVERRIDES {
        return Err(ConfigError::new(
            Reason::LimitExceeded,
            "",
            ConfigSource::Environment,
        ));
    }
    let mut names = std::collections::BTreeSet::new();
    let mut paths = std::collections::BTreeSet::new();
    let mut overrides = Vec::new();
    for mapping in mappings {
        if mapping.variable.is_empty()
            || !mapping
                .variable
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
            || !names.insert(mapping.variable)
            || !paths.insert(mapping.path)
            || pointer_segments(mapping.path).is_none()
        {
            return Err(ConfigError::new(
                Reason::InvalidMapping,
                mapping.path,
                ConfigSource::Environment,
            ));
        }
        let Some(raw) = lookup(mapping.variable) else {
            continue;
        };
        if raw.len() > MAX_CONFIG_BYTES {
            return Err(ConfigError::new(
                Reason::LimitExceeded,
                mapping.path,
                ConfigSource::Environment,
            ));
        }
        let invalid = || {
            ConfigError::new(
                Reason::TypeMismatch,
                mapping.path,
                ConfigSource::Environment,
            )
        };
        let value = match mapping.kind {
            EnvValueKind::String => Value::String(raw),
            EnvValueKind::Boolean => match raw.as_str() {
                "true" => Value::Bool(true),
                "false" => Value::Bool(false),
                _ => return Err(invalid()),
            },
            EnvValueKind::UnsignedInteger => {
                Value::from(raw.parse::<u64>().map_err(|_| invalid())?)
            }
            EnvValueKind::SignedInteger => Value::from(raw.parse::<i64>().map_err(|_| invalid())?),
            EnvValueKind::Json => strict::parse_json(raw.as_bytes(), ConfigSource::Environment)
                .map_err(|mut error| {
                    error.path = format!("{}{}", mapping.path, error.path);
                    error
                })?,
        };
        overrides.push(Override::new(mapping.path, value));
    }
    Ok(overrides)
}

pub struct Loaded<T> {
    pub value: T,
    /// JSON pointers to effective leaves. Values and environment contents are
    /// absent: this map can be safely included in configuration diagnostics.
    pub sources: BTreeMap<String, ConfigSource>,
}

/// Defaults < current JSON file < explicitly mapped environment < CLI.
///
/// Validate each layer before applying the next: a higher-priority value must
/// not hide an unknown field, incorrect type or invalid lower-priority value.
/// No historical aliases or coercion are supported. Semantic checks which
/// require multiple fields or external resources remain product-owned.
pub fn resolve<T: Serialize + DeserializeOwned>(
    defaults: &T,
    file_json: Option<&[u8]>,
    environment: &[Override],
    command_line: &[Override],
) -> Result<Loaded<T>, ConfigError> {
    resolve_validated(
        defaults,
        file_json,
        environment,
        command_line,
        |_, _| Ok(()),
    )
}

/// Validate product-owned field constraints at each source boundary. The hook
/// checks each complete source layer (including related field constraints); it
/// must defer absent required inputs and filesystem/dependency checks until
/// the caller validates the final effective configuration. Errors retain the
/// actual source even if a later source would override the offending value.
pub fn resolve_validated<T: Serialize + DeserializeOwned>(
    defaults: &T,
    file_json: Option<&[u8]>,
    environment: &[Override],
    command_line: &[Override],
    validate: impl Fn(&T, ConfigSource) -> Result<(), ConfigError>,
) -> Result<Loaded<T>, ConfigError> {
    let mut effective = serde_json::to_value(defaults)
        .map_err(|_| ConfigError::new(Reason::InvalidValue, "", ConfigSource::Default))?;
    if !effective.is_object() {
        return Err(ConfigError::new(
            Reason::TypeMismatch,
            "",
            ConfigSource::Default,
        ));
    }
    let check = |value: Value, source| {
        let typed = strict::deserialize::<T>(value, source)?;
        validate(&typed, source).map_err(|mut error| {
            error.source = source;
            error
        })?;
        Ok::<T, ConfigError>(typed)
    };
    check(effective.clone(), ConfigSource::Default)?;
    let mut sources = BTreeMap::new();
    record_sources(&effective, "", ConfigSource::Default, &mut sources);
    if let Some(bytes) = file_json {
        let file = strict::parse_json(bytes, ConfigSource::File)?;
        if !file.is_object() {
            return Err(ConfigError::new(
                Reason::TypeMismatch,
                "",
                ConfigSource::File,
            ));
        }
        merge(&mut effective, file, "", &mut sources);
        check(effective.clone(), ConfigSource::File)?;
    }
    for (overrides, source) in [
        (environment, ConfigSource::Environment),
        (command_line, ConfigSource::CommandLine),
    ] {
        if overrides.len() > MAX_OVERRIDES {
            return Err(ConfigError::new(Reason::LimitExceeded, "", source));
        }
        let mut seen = std::collections::BTreeSet::new();
        for item in overrides {
            if !seen.insert(&item.path) {
                return Err(ConfigError::new(Reason::DuplicateField, &item.path, source));
            }
            let segments = pointer_segments(&item.path)
                .ok_or_else(|| ConfigError::new(Reason::InvalidMapping, &item.path, source))?;
            let mut parent = &mut effective;
            for segment in &segments[..segments.len() - 1] {
                parent = parent
                    .as_object_mut()
                    .and_then(|map| map.get_mut(segment))
                    .ok_or_else(|| ConfigError::new(Reason::UnknownField, &item.path, source))?;
            }
            let map = parent
                .as_object_mut()
                .ok_or_else(|| ConfigError::new(Reason::TypeMismatch, &item.path, source))?;
            map.insert(
                segments.last().expect("nonempty pointer").clone(),
                item.value.clone(),
            );
            // Reject structural errors immediately so later entries cannot hide
            // unknown fields or invalid types. Product constraints are checked
            // after the complete source, not against a partly applied layer.
            strict::deserialize::<T>(effective.clone(), source)?;
            remove_sources(&item.path, &mut sources);
            record_sources(&item.value, &item.path, source, &mut sources);
        }
        if !overrides.is_empty() {
            check(effective.clone(), source)?;
        }
    }
    // Every intermediate layer has already been checked. Do not label a final
    // default/file value as CLI merely because there were no explicit args.
    let value = strict::deserialize(effective, ConfigSource::Default)?;
    Ok(Loaded { value, sources })
}

fn pointer_segments(pointer: &str) -> Option<Vec<String>> {
    if !pointer.starts_with('/') {
        return None;
    }
    pointer[1..]
        .split('/')
        .map(|part| {
            let mut decoded = String::new();
            let mut chars = part.chars();
            while let Some(c) = chars.next() {
                if c == '~' {
                    decoded.push(match chars.next()? {
                        '0' => '~',
                        '1' => '/',
                        _ => return None,
                    });
                } else {
                    decoded.push(c);
                }
            }
            Some(decoded)
        })
        .collect()
}

fn child_path(parent: &str, key: &str) -> String {
    format!("{parent}/{}", key.replace('~', "~0").replace('/', "~1"))
}

fn remove_sources(path: &str, sources: &mut BTreeMap<String, ConfigSource>) {
    let prefix = format!("{path}/");
    sources.retain(|key, _| key != path && !key.starts_with(&prefix));
}

fn record_sources(
    value: &Value,
    path: &str,
    source: ConfigSource,
    sources: &mut BTreeMap<String, ConfigSource>,
) {
    if let Value::Object(map) = value
        && !map.is_empty()
    {
        for (key, child) in map {
            record_sources(child, &child_path(path, key), source, sources);
        }
    } else {
        sources.insert(path.to_string(), source);
    }
}

fn merge(
    current: &mut Value,
    overlay: Value,
    path: &str,
    sources: &mut BTreeMap<String, ConfigSource>,
) {
    match (current, overlay) {
        (Value::Object(current), Value::Object(overlay)) => {
            for (key, value) in overlay {
                let child = child_path(path, &key);
                if let Some(existing) = current.get_mut(&key) {
                    merge(existing, value, &child, sources);
                } else {
                    record_sources(&value, &child, ConfigSource::File, sources);
                    current.insert(key, value);
                }
            }
        }
        (current, overlay) => {
            remove_sources(path, sources);
            record_sources(&overlay, path, ConfigSource::File, sources);
            *current = overlay;
        }
    }
}
