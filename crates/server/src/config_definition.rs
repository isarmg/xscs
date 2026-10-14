//! Current configuration data shared by runtime parsing and build-time contract generation.
use serde::{Deserialize, Serialize};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Settings {
    pub(crate) data_dir: PathBuf,
    pub(crate) bind: SocketAddr,
    pub(crate) development: bool,
    pub(crate) log_level: String,
    pub(crate) development_web_dir: Option<String>,
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

pub(crate) fn data_path_spec() -> serde_json::Value {
    serde_json::json!({"base":"config","value":"/data_dir","fallback":null,"suffix":"","encoding":"path"})
}
pub(crate) fn intrinsic_rules() -> serde_json::Value {
    use serde_json::json;
    let rules = vec![
        json!({"kind":"enum","fields":["/log_level"],"arguments":{"values":["off","error","warn","info","debug","trace"]}}),
        json!({"kind":"socket-address","fields":["/bind"],"arguments":{"loopback_when":"/development"}}),
        json!({"kind":"path","fields":["/data_dir"],"arguments":{"absolute":true}}),
        json!({"kind":"path","fields":["/development_web_dir"],"arguments":{"absolute":true,"optional":true}}),
    ];
    json!(rules)
}

pub(crate) fn current_rules(source_bound: bool) -> serde_json::Value {
    use serde_json::json;
    let mut rules = intrinsic_rules().as_array().unwrap().clone();
    rules.extend([
        json!({"kind":"root-state-paths","fields":[],"arguments":{"paths":[data_path_spec()],"protected":xcss::server_layout::ROOT_RESERVED_DIRECTORIES}}),
        json!({"kind":"external-web-override","fields":["/development_web_dir"],"arguments":{"development_pointer":"/development","source_bound":source_bound}}),
    ]);
    json!(rules)
}

#[cfg(test)]
pub(crate) fn validate_intrinsic(
    settings: &Settings,
    source: xcss::config::ConfigSource,
) -> Result<(), xcss::config::ConfigError> {
    let value = serde_json::to_value(settings).map_err(|_| {
        xcss::config::ConfigError::new(xcss::config::Reason::InvalidMapping, "", source)
    })?;
    // Intrinsic rules do not refer to an installation root. Runtime additionally
    // validates all root-dependent rules with its explicit physical ROOT.
    xcss::configuration_contract::validate_rules(
        &intrinsic_rules(),
        &value,
        Path::new("/configuration-validation"),
        source,
    )
}

pub(crate) fn validate(
    settings: &Settings,
    root: &Path,
    source_bound: bool,
    source: xcss::config::ConfigSource,
) -> Result<(), xcss::config::ConfigError> {
    let value = serde_json::to_value(settings).map_err(|_| {
        xcss::config::ConfigError::new(xcss::config::Reason::InvalidMapping, "", source)
    })?;
    xcss::configuration_contract::validate_rules(&current_rules(source_bound), &value, root, source)
}
