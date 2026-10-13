use std::{
    collections::BTreeMap,
    env, fs,
    net::SocketAddr,
    path::{Path, PathBuf},
};

use anyhow::Context;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
#[cfg(test)]
use xcss::config::Override;
use xcss::config::{ConfigSource, EnvMapping, EnvValueKind};

use crate::crypto::SecretBox;

#[derive(Clone)]
pub struct ServeConfig {
    pub root: PathBuf,
    pub bind: SocketAddr,
    pub database_url: String,
    pub production: bool,
    pub log_level: String,
    pub static_dir: Option<PathBuf>,
    pub data_dir: PathBuf,
    pub sources: BTreeMap<String, ConfigSource>,
}

impl ServeConfig {
    pub fn from_root_sources(root: &Path, config: &Path) -> anyhow::Result<Self> {
        anyhow::ensure!(
            root.is_absolute()
                && root != Path::new("/")
                && root.components().all(|part| matches!(
                    part,
                    std::path::Component::RootDir | std::path::Component::Normal(_)
                )),
            "root must be a normalized absolute directory"
        );
        let defaults = Settings {
            data_dir: root.join("state"),
            ..Settings::default()
        };

        let config = normalize_config_path(config)?;
        let file = xcss::config::read_private_file(&config)?;
        let environment = xcss::config::read_environment(ENVIRONMENT, |name| env::var(name).ok())?;
        let loaded = xcss::config::resolve_validated(
            &defaults,
            Some(&file),
            &environment,
            &[],
            validate_intrinsic,
        )?;
        let settings = loaded.value;
        let data_dir = settings.data_dir;
        validate_state_location(root, &data_dir)?;
        let database_url = format!("sqlite://{}", data_dir.join("xscs.sqlite3").display());
        let bind = settings.bind;
        let production = !settings.development;
        let static_dir = settings.development_web_dir.map(PathBuf::from);
        anyhow::ensure!(
            static_dir.is_none()
                || (!production && crate::release_contract::SOURCE_REVISION == "unbound"),
            "directory Web assets require an unbound development build"
        );
        validate_bind(bind, production)?;

        Ok(Self {
            root: root.to_path_buf(),
            bind,
            database_url,
            production,
            log_level: settings.log_level,
            static_dir,
            data_dir,
            sources: loaded.sources,
        })
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    data_dir: PathBuf,
    bind: SocketAddr,
    development: bool,
    log_level: String,
    development_web_dir: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            data_dir: PathBuf::from("/opt/isarmg/xscs/state"),
            bind: "127.0.0.1:18104".parse().expect("valid default bind"),
            development: false,
            log_level: "info".into(),
            development_web_dir: None,
        }
    }
}

const ENVIRONMENT: &[EnvMapping<'static>] = &[
    EnvMapping {
        variable: "XSCS_DATA_DIR",
        path: "/data_dir",
        kind: EnvValueKind::String,
    },
    EnvMapping {
        variable: "XSCS_BIND",
        path: "/bind",
        kind: EnvValueKind::String,
    },
    EnvMapping {
        variable: "XSCS_DEVELOPMENT",
        path: "/development",
        kind: EnvValueKind::Boolean,
    },
    EnvMapping {
        variable: "XSCS_LOG_LEVEL",
        path: "/log_level",
        kind: EnvValueKind::String,
    },
    EnvMapping {
        variable: "XSCS_DEV_WEB_DIR",
        path: "/development_web_dir",
        kind: EnvValueKind::String,
    },
];

fn development_static_dir(
    value: Option<&str>,
    development: bool,
) -> anyhow::Result<Option<PathBuf>> {
    let Some(value) = value else { return Ok(None) };
    anyhow::ensure!(
        development && crate::release_contract::SOURCE_REVISION == "unbound",
        "directory Web assets are allowed only in an unbound development build"
    );
    Ok(Some(validate_static_dir(value, false)?))
}

fn validate_static_dir(value: &str, production: bool) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        !production,
        "production Web assets are embedded in the binary"
    );
    let root = Path::new(value);
    anyhow::ensure!(
        root.is_absolute(),
        "development Web directory must be absolute"
    );
    xcss::web_assets::DirectoryAssets::new(root)?;
    anyhow::ensure!(
        root.join("index.html").is_file() && root.join("assets").is_dir(),
        "development Web directory must contain index.html and assets"
    );
    let mut entries = fs::read_dir(root)?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<Result<Vec<_>, _>>()?;
    entries.sort();
    anyhow::ensure!(
        entries
            == [
                std::ffi::OsString::from("assets"),
                std::ffi::OsString::from("index.html")
            ],
        "development Web directory must contain only the declared browser entry and assets"
    );
    Ok(fs::canonicalize(root)?)
}

fn decode_key(value: &str) -> anyhow::Result<[u8; 32]> {
    let decoded = STANDARD
        .decode(value.trim())
        .context("credential key must be base64")?;
    decoded
        .try_into()
        .map_err(|_| anyhow::anyhow!("credential key must decode to exactly 32 bytes"))
}

fn validate_bind(bind: SocketAddr, production: bool) -> anyhow::Result<()> {
    anyhow::ensure!(
        production || bind.ip().is_loopback(),
        "XSCS_DEVELOPMENT=true requires a loopback XSCS_BIND"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn production_accepts_configured_backend_addresses_and_preserves_the_default() {
        assert_eq!(
            Settings::default().bind,
            "127.0.0.1:18104".parse::<SocketAddr>().unwrap()
        );
        for bind in ["10.20.0.12:18104", "0.0.0.0:18104", "[::]:18104"] {
            let file = serde_json::to_vec(&serde_json::json!({"bind": bind})).unwrap();
            let loaded = xcss::config::resolve_validated(
                &Settings::default(),
                Some(&file),
                &[],
                &[],
                validate_intrinsic,
            )
            .unwrap();
            assert_eq!(loaded.value.bind, bind.parse::<SocketAddr>().unwrap());
            validate_bind(loaded.value.bind, !loaded.value.development).unwrap();
        }
    }

    #[test]
    fn development_requires_a_loopback_backend_address() {
        for bind in ["127.0.0.1:18104", "[::1]:18104"] {
            validate_bind(bind.parse().unwrap(), false).unwrap();
        }
        for bind in ["10.20.0.12:18104", "0.0.0.0:18104", "[::]:18104"] {
            assert!(validate_bind(bind.parse().unwrap(), false).is_err());
        }
    }

    #[test]
    fn embedded_assets_need_no_external_directory_and_production_rejects_overrides() {
        assert!(development_static_dir(None, false).unwrap().is_none());
        assert!(development_static_dir(Some("/not-needed"), false).is_err());
    }

    #[test]
    fn credential_key_decoding_requires_exactly_32_bytes() {
        assert!(decode_key(&STANDARD.encode([7_u8; 32])).is_ok());
        assert!(decode_key(&STANDARD.encode([7_u8; 31])).is_err());
    }

    #[test]
    fn static_directory_is_absolute_complete_and_contains_no_link_aliases() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("index.html"), "current").unwrap();
        fs::create_dir(directory.path().join("assets")).unwrap();
        fs::write(directory.path().join("assets/app.js"), "current").unwrap();

        assert_eq!(
            validate_static_dir(directory.path().to_str().unwrap(), false).unwrap(),
            directory.path().canonicalize().unwrap()
        );
        assert!(validate_static_dir(directory.path().to_str().unwrap(), true).is_err());
        assert!(validate_static_dir("web/dist", false).is_err());

        fs::remove_file(directory.path().join("index.html")).unwrap();
        assert!(validate_static_dir(directory.path().to_str().unwrap(), false).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn static_directory_rejects_symbolic_and_hard_linked_assets() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let index = directory.path().join("index.html");
        fs::write(&index, "current").unwrap();
        fs::create_dir(directory.path().join("assets")).unwrap();
        fs::write(directory.path().join("assets/app.js"), "current").unwrap();
        let alias = directory.path().join("alias.html");
        fs::hard_link(&index, &alias).unwrap();
        assert!(validate_static_dir(directory.path().to_str().unwrap(), false).is_err());

        fs::remove_file(&alias).unwrap();
        let outside = tempfile::NamedTempFile::new().unwrap();
        symlink(outside.path(), directory.path().join("linked.js")).unwrap();
        assert!(validate_static_dir(directory.path().to_str().unwrap(), false).is_err());
    }
}

fn validate_intrinsic(
    settings: &Settings,
    source: ConfigSource,
) -> Result<(), xcss::config::ConfigError> {
    let invalid =
        |path| xcss::config::ConfigError::new(xcss::config::Reason::InvalidValue, path, source);
    if !settings.data_dir.is_absolute()
        || settings.data_dir.components().any(|part| {
            matches!(
                part,
                std::path::Component::ParentDir | std::path::Component::CurDir
            )
        })
    {
        return Err(invalid("/data_dir"));
    }
    if !["error", "warn", "info", "debug", "trace", "off"].contains(&settings.log_level.as_str()) {
        return Err(invalid("/log_level"));
    }
    if settings
        .development_web_dir
        .as_ref()
        .is_some_and(|path| !Path::new(path).is_absolute())
    {
        return Err(invalid("/development_web_dir"));
    }
    Ok(())
}

#[cfg(test)]
mod precedence_contract_tests {
    use super::*;
    #[test]
    fn a_higher_priority_override_cannot_hide_an_invalid_file_value() {
        let file =
            serde_json::to_vec(&serde_json::json!({"log_level":"private-invalid-secret"})).unwrap();
        let error = xcss::config::resolve_validated(
            &Settings::default(),
            Some(&file),
            &[Override::new("/bind", serde_json::json!("127.0.0.1:18104"))],
            &[],
            validate_intrinsic,
        )
        .err()
        .expect("invalid lower layer must fail");
        assert_eq!(error.path, "/log_level");
        assert_eq!(error.source, ConfigSource::File);
        assert!(
            !serde_json::to_string(&error.envelope())
                .unwrap()
                .contains("private-invalid-secret")
        );
    }
}

/// Normalize the required configuration path before private reads and reporting.
pub fn normalize_config_path(path: &Path) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(
        !path
            .components()
            .any(|part| part == std::path::Component::ParentDir),
        "config path cannot contain parent traversal"
    );
    Ok(std::path::absolute(path)?)
}

#[cfg(test)]
mod deployment_defaults_tests {
    use super::*;
    #[test]
    fn production_is_default_and_old_configuration_names_are_rejected() {
        assert!(!Settings::default().development);
        assert_eq!(
            Settings::default().data_dir,
            PathBuf::from("/opt/isarmg/xscs/state")
        );
        let mut json = serde_json::to_value(Settings::default()).unwrap();
        json["production"] = serde_json::json!(true);
        assert!(serde_json::from_value::<Settings>(json).is_err());
    }
    #[test]
    fn environment_mappings_are_product_specific_and_have_no_bootstrap_secrets() {
        assert!(
            ENVIRONMENT
                .iter()
                .all(|mapping| mapping.variable.starts_with("XSCS_"))
        );
        assert!(
            !ENVIRONMENT
                .iter()
                .any(|mapping| mapping.variable.contains("BOOTSTRAP")
                    || mapping.variable.contains("PRODUCTION"))
        );
    }
    #[test]
    fn explicit_missing_config_is_not_silently_dropped() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing.json");
        assert!(ServeConfig::from_root_sources(directory.path(), &path).is_err());
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SecretsDocument {
    version: u32,
    credential_key_id: String,
    credential_key: String,
}
impl SecretsDocument {
    fn secret_box(self) -> anyhow::Result<SecretBox> {
        anyhow::ensure!(
            self.version == 1 && self.credential_key_id == "primary",
            "invalid secrets document identity"
        );
        let key = decode_key(&self.credential_key)?;
        anyhow::ensure!(
            STANDARD.encode(key) == self.credential_key,
            "credential key must use canonical base64"
        );
        SecretBox::new(self.credential_key_id, key)
    }
}
impl ServeConfig {
    pub fn initialize_secrets(&self) -> anyhow::Result<SecretBox> {
        let document = SecretsDocument {
            version: 1,
            credential_key_id: "primary".into(),
            credential_key: STANDARD.encode(xcss::server_secrets::generate_key()?),
        };
        xcss::server_secrets::initialize(&self.data_dir, &document)?;
        document.secret_box()
    }
    pub fn load_secrets(&self) -> anyhow::Result<SecretBox> {
        xcss::server_secrets::load::<SecretsDocument>(&self.data_dir)?.secret_box()
    }
    pub fn validate_runtime_assets(&self) -> anyhow::Result<()> {
        development_static_dir(
            self.static_dir.as_ref().and_then(|p| p.to_str()),
            !self.production,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod secrets_contract_tests {
    use super::*;
    #[test]
    fn private_document_requires_current_identity_and_canonical_key() {
        for (version, id, key) in [
            (2, "primary", STANDARD.encode([7; 32])),
            (1, "other", STANDARD.encode([7; 32])),
            (1, "primary", STANDARD.encode([7; 31])),
            (1, "primary", format!(" {}", STANDARD.encode([7; 32]))),
        ] {
            assert!(
                SecretsDocument {
                    version,
                    credential_key_id: id.into(),
                    credential_key: key
                }
                .secret_box()
                .is_err()
            );
        }
        assert!(
            SecretsDocument {
                version: 1,
                credential_key_id: "primary".into(),
                credential_key: STANDARD.encode([7; 32])
            }
            .secret_box()
            .is_ok()
        );
    }
    #[test]
    fn configuration_rejects_state_secrets_and_database_path_inputs() {
        for field in [
            "credential_key",
            "credential_key_id",
            "database_url",
            "bootstrap_admin_password",
        ] {
            let mut value = serde_json::to_value(Settings::default()).unwrap();
            value[field] = serde_json::json!("not-a-setting");
            assert!(serde_json::from_value::<Settings>(value).is_err());
        }
    }
}

fn validate_state_location(root: &Path, state: &Path) -> anyhow::Result<()> {
    anyhow::ensure!(
        state.is_absolute()
            && state != Path::new("/")
            && state.components().all(|part| matches!(
                part,
                std::path::Component::RootDir | std::path::Component::Normal(_)
            )),
        "state paths must be normalized absolute directories"
    );
    anyhow::ensure!(
        !root.starts_with(state),
        "state directory cannot contain the installation root"
    );
    for name in [
        "config",
        "logs",
        "run",
        "bin",
        "libexec",
        "share",
        "maintenance",
    ] {
        let protected = root.join(name);
        anyhow::ensure!(
            !state.starts_with(&protected) && !protected.starts_with(state),
            "state directory overlaps a protected installation path"
        );
    }
    Ok(())
}
