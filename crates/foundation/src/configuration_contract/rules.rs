//! Current declarative semantic rules. The JSON shape is release material;
//! independently implemented interpreters consume the same finite definitions.

use crate::config::{ConfigError, ConfigSource, Reason};
use serde::Deserialize;
use serde_json::{Map, Value};
use std::{
    net::SocketAddr,
    path::{Component, Path, PathBuf},
};

const MAX_RULES: usize = 256;
const MAX_FIELDS: usize = 16;
const MAX_TEXT_BYTES: usize = 65_536;
const MAX_PATH_DEPTH: usize = 16;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rule {
    kind: String,
    fields: Vec<String>,
    arguments: Map<String, Value>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PathSpec {
    base: String,
    value: String,
    fallback: Option<Box<PathSpec>>,
    suffix: String,
    encoding: String,
}

/// Validate one current rule array without filesystem access. Unknown rules,
/// arguments, missing referenced values and malformed definitions fail closed.
/// Diagnostics contain field pointers, never candidate values or secrets.
pub fn validate_rules(
    rules: &Value,
    value: &Value,
    root: &Path,
    source: ConfigSource,
) -> std::result::Result<(), ConfigError> {
    let invalid_mapping = || ConfigError::new(Reason::InvalidMapping, "", source);
    let raw = rules.as_array().ok_or_else(invalid_mapping)?;
    if raw.len() > MAX_RULES || crate::server_layout::Layout::new(root).is_err() {
        return Err(invalid_mapping());
    }
    for raw_rule in raw {
        let rule: Rule = serde_json::from_value(raw_rule.clone()).map_err(|_| invalid_mapping())?;
        if rule.fields.len() > MAX_FIELDS || rule.fields.iter().any(|field| !valid_pointer(field)) {
            return Err(invalid_mapping());
        }
        let path = rule.fields.first().cloned().unwrap_or_default();
        let result = apply(&rule, value, root);
        if let Err(reason) = result {
            return Err(ConfigError::new(reason, path, source));
        }
    }
    Ok(())
}

fn valid_pointer(pointer: &str) -> bool {
    !pointer.is_empty()
        && pointer.starts_with('/')
        && pointer.len() <= MAX_TEXT_BYTES
        && !pointer.chars().any(char::is_control)
        && pointer
            .split('~')
            .skip(1)
            .all(|part| part.starts_with(['0', '1']))
}

type Result<T> = std::result::Result<T, Reason>;
fn mapping<T>() -> Result<T> {
    Err(Reason::InvalidMapping)
}
fn candidate<'a>(value: &'a Value, pointer: &str) -> Result<&'a Value> {
    value.pointer(pointer).ok_or(Reason::MissingField)
}
fn flag(arguments: &Map<String, Value>, name: &str, default: bool) -> Result<bool> {
    arguments
        .get(name)
        .map_or(Ok(default), |v| v.as_bool().ok_or(Reason::InvalidMapping))
}
fn text<'a>(arguments: &'a Map<String, Value>, name: &str) -> Result<&'a str> {
    arguments
        .get(name)
        .and_then(Value::as_str)
        .filter(|v| v.len() <= MAX_TEXT_BYTES)
        .ok_or(Reason::InvalidMapping)
}
fn integer(arguments: &Map<String, Value>, name: &str) -> Result<u64> {
    arguments
        .get(name)
        .and_then(Value::as_u64)
        .ok_or(Reason::InvalidMapping)
}
fn keys(arguments: &Map<String, Value>, allowed: &[&str]) -> Result<()> {
    if arguments.keys().all(|key| allowed.contains(&key.as_str())) {
        Ok(())
    } else {
        mapping()
    }
}
fn fields(rule: &Rule, expected: usize) -> Result<()> {
    if rule.fields.len() == expected {
        Ok(())
    } else {
        mapping()
    }
}
fn socket(value: &Value) -> Result<SocketAddr> {
    value
        .as_str()
        .ok_or(Reason::TypeMismatch)?
        .parse()
        .map_err(|_| Reason::InvalidValue)
}
fn normalized(path: &Path, absolute: bool) -> bool {
    path.to_str().is_some_and(|s| {
        !s.is_empty() && s.len() <= MAX_TEXT_BYTES && !s.chars().any(char::is_control)
    }) && (!absolute || path.is_absolute())
        && path
            .components()
            .all(|c| matches!(c, Component::RootDir | Component::Normal(_)))
        && path.components().collect::<PathBuf>().as_os_str() == path.as_os_str()
}
fn relative(path: &str) -> bool {
    normalized(Path::new(path), false) && !Path::new(path).is_absolute()
}

fn apply(rule: &Rule, value: &Value, root: &Path) -> Result<()> {
    let args = &rule.arguments;
    match rule.kind.as_str() {
        "integer-range" => {
            fields(rule, 1)?;
            keys(args, &["minimum", "maximum"])?;
            let min = integer(args, "minimum")?;
            let max = integer(args, "maximum")?;
            if min > max {
                return mapping();
            }
            let actual = candidate(value, &rule.fields[0])?
                .as_u64()
                .ok_or(Reason::TypeMismatch)?;
            if actual < min || actual > max {
                return Err(Reason::InvalidValue);
            }
        }
        "integer-order" => {
            fields(rule, 2)?;
            keys(args, &["relation", "left_scale", "right_scale"])?;
            let relation = text(args, "relation")?;
            if !matches!(relation, "less-or-equal" | "less-than") {
                return mapping();
            }
            let scale = |name| match args.get(name) {
                None => Ok(1),
                Some(value) => value
                    .as_u64()
                    .filter(|n| *n != 0)
                    .ok_or(Reason::InvalidMapping),
            };
            let lhs = candidate(value, &rule.fields[0])?
                .as_u64()
                .ok_or(Reason::TypeMismatch)?
                .checked_mul(scale("left_scale")?)
                .ok_or(Reason::InvalidValue)?;
            let rhs = candidate(value, &rule.fields[1])?
                .as_u64()
                .ok_or(Reason::TypeMismatch)?
                .checked_mul(scale("right_scale")?)
                .ok_or(Reason::InvalidValue)?;
            if lhs > rhs || (relation == "less-than" && lhs == rhs) {
                return Err(Reason::InvalidValue);
            }
        }
        "enum" => {
            fields(rule, 1)?;
            keys(args, &["values"])?;
            let choices = args
                .get("values")
                .and_then(Value::as_array)
                .ok_or(Reason::InvalidMapping)?;
            if choices.is_empty()
                || choices.len() > MAX_RULES
                || choices.iter().any(|v| {
                    !matches!(
                        v,
                        Value::String(_) | Value::Number(_) | Value::Bool(_) | Value::Null
                    )
                })
            {
                return mapping();
            }
            if !choices.contains(candidate(value, &rule.fields[0])?) {
                return Err(Reason::InvalidValue);
            }
        }
        "socket-address" => {
            fields(rule, 1)?;
            keys(args, &["nonzero", "loopback", "loopback_when"])?;
            let actual = socket(candidate(value, &rule.fields[0])?)?;
            let conditional = match args.get("loopback_when") {
                None | Some(Value::Null) => false,
                Some(Value::String(pointer)) if valid_pointer(pointer) => {
                    candidate(value, pointer)?
                        .as_bool()
                        .ok_or(Reason::TypeMismatch)?
                }
                _ => return mapping(),
            };
            if (flag(args, "nonzero", false)? && actual.port() == 0)
                || ((flag(args, "loopback", false)? || conditional) && !actual.ip().is_loopback())
            {
                return Err(Reason::InvalidValue);
            }
        }
        "distinct-sockets" => {
            keys(args, &[])?;
            if rule.fields.len() < 2 {
                return mapping();
            }
            let mut sockets = Vec::new();
            for pointer in &rule.fields {
                let address = socket(candidate(value, pointer)?)?;
                if sockets.iter().any(|other: &SocketAddr| {
                    other.port() == address.port()
                        && (other.ip() == address.ip()
                            || other.ip().is_unspecified()
                            || address.ip().is_unspecified())
                }) {
                    return Err(Reason::InvalidValue);
                }
                sockets.push(address);
            }
        }
        "path" => {
            fields(rule, 1)?;
            keys(args, &["optional", "absolute"])?;
            let actual = candidate(value, &rule.fields[0])?;
            if actual.is_null() && flag(args, "optional", false)? {
                return Ok(());
            }
            let path = actual.as_str().ok_or(Reason::TypeMismatch)?;
            if !normalized(Path::new(path), flag(args, "absolute", false)?) {
                return Err(Reason::InvalidValue);
            }
        }
        "root-state-paths" => {
            fields(rule, 0)?;
            keys(args, &["paths", "protected"])?;
            let paths = args
                .get("paths")
                .and_then(Value::as_array)
                .ok_or(Reason::InvalidMapping)?;
            let protected = args
                .get("protected")
                .and_then(Value::as_array)
                .ok_or(Reason::InvalidMapping)?;
            if paths.is_empty() || paths.len() > MAX_FIELDS || protected.len() > MAX_RULES {
                return mapping();
            }
            let mut resolved = Vec::new();
            for raw in paths {
                let spec: PathSpec =
                    serde_json::from_value(raw.clone()).map_err(|_| Reason::InvalidMapping)?;
                let path = resolve(&spec, value, root, 0)?;
                if path == Path::new("/") || path == root {
                    return Err(Reason::InvalidValue);
                }
                for reserved in protected {
                    let reserved = reserved.as_str().ok_or(Reason::InvalidMapping)?;
                    if !relative(reserved) {
                        return mapping();
                    }
                    let reserved = root.join(reserved);
                    if path.starts_with(&reserved) || reserved.starts_with(&path) {
                        return Err(Reason::InvalidValue);
                    }
                }
                resolved.push(path);
            }
            for (index, path) in resolved.iter().enumerate() {
                if resolved[..index]
                    .iter()
                    .any(|other| path.starts_with(other) || other.starts_with(path))
                {
                    return Err(Reason::InvalidValue);
                }
            }
        }
        "path-relation" => {
            fields(rule, 0)?;
            keys(
                args,
                &[
                    "parent",
                    "child",
                    "allow_child_subtree",
                    "forbidden_child_first_component_prefixes",
                ],
            )?;
            let parent: PathSpec =
                serde_json::from_value(args.get("parent").ok_or(Reason::InvalidMapping)?.clone())
                    .map_err(|_| Reason::InvalidMapping)?;
            let child: PathSpec =
                serde_json::from_value(args.get("child").ok_or(Reason::InvalidMapping)?.clone())
                    .map_err(|_| Reason::InvalidMapping)?;
            let parent = resolve(&parent, value, root, 0)?;
            let child = resolve(&child, value, root, 0)?;
            let allow = args
                .get("allow_child_subtree")
                .and_then(Value::as_bool)
                .ok_or(Reason::InvalidMapping)?;
            let prefixes = args
                .get("forbidden_child_first_component_prefixes")
                .and_then(Value::as_array)
                .ok_or(Reason::InvalidMapping)?;
            if prefixes.len() > MAX_RULES
                || prefixes.iter().any(|v| {
                    v.as_str()
                        .is_none_or(|s| s.len() > MAX_TEXT_BYTES || s.chars().any(char::is_control))
                })
            {
                return mapping();
            }
            if parent.starts_with(&child) {
                return Err(Reason::InvalidValue);
            }
            if let Ok(relative) = child.strip_prefix(&parent) {
                let first = relative
                    .components()
                    .next()
                    .and_then(|c| c.as_os_str().to_str())
                    .ok_or(Reason::InvalidValue)?;
                if !allow
                    || prefixes
                        .iter()
                        .any(|prefix| first.starts_with(prefix.as_str().expect("validated prefix")))
                {
                    return Err(Reason::InvalidValue);
                }
            }
        }
        "external-web-override" => {
            fields(rule, 1)?;
            keys(args, &["development_pointer", "source_bound"])?;
            let pointer = text(args, "development_pointer")?;
            if !valid_pointer(pointer) {
                return mapping();
            }
            let development = candidate(value, pointer)?
                .as_bool()
                .ok_or(Reason::TypeMismatch)?;
            let bound = args
                .get("source_bound")
                .and_then(Value::as_bool)
                .ok_or(Reason::InvalidMapping)?;
            if !candidate(value, &rule.fields[0])?.is_null() && (!development || bound) {
                return Err(Reason::InvalidValue);
            }
        }
        "url" => {
            fields(rule, 1)?;
            keys(
                args,
                &[
                    "allowed_schemes",
                    "no_credentials",
                    "no_query",
                    "no_fragment",
                    "loopback_host",
                    "origin_only",
                    "production_public_rtsps",
                    "development_pointer",
                    "relative_path",
                ],
            )?;
            let input = candidate(value, &rule.fields[0])?
                .as_str()
                .ok_or(Reason::TypeMismatch)?;
            if input.len() > MAX_TEXT_BYTES || input.chars().any(char::is_control) {
                return Err(Reason::InvalidValue);
            }
            if flag(args, "relative_path", false)?
                && input.starts_with('/')
                && !input.starts_with("//")
            {
                if input.contains(['?', '#']) {
                    return Err(Reason::InvalidValue);
                }
                return Ok(());
            }
            let parsed = url::Url::parse(input).map_err(|_| Reason::InvalidValue)?;
            let schemes = args
                .get("allowed_schemes")
                .and_then(Value::as_array)
                .ok_or(Reason::InvalidMapping)?;
            if schemes.is_empty()
                || schemes.len() > MAX_FIELDS
                || schemes.iter().any(|s| s.as_str().is_none())
            {
                return mapping();
            }
            let host = parsed.host().ok_or(Reason::InvalidValue)?;
            // Non-special schemes such as RTSP can expose a numeric host as
            // Domain, including bracketed IPv6. Parse that representation too.
            let address = match &host {
                url::Host::Domain(s) => s
                    .strip_prefix('[')
                    .and_then(|s| s.strip_suffix(']'))
                    .unwrap_or(s)
                    .parse::<std::net::IpAddr>()
                    .ok(),
                url::Host::Ipv4(ip) => Some(std::net::IpAddr::V4(*ip)),
                url::Host::Ipv6(ip) => Some(std::net::IpAddr::V6(*ip)),
            };
            let loopback = matches!(&host, url::Host::Domain(s) if s.eq_ignore_ascii_case("localhost"))
                || address.is_some_and(|ip| ip.is_loopback());
            let unspecified = address.is_some_and(|ip| ip.is_unspecified());
            if !schemes.iter().any(|s| s.as_str() == Some(parsed.scheme()))
                || (flag(args, "no_credentials", false)?
                    && (!parsed.username().is_empty() || parsed.password().is_some()))
                || (flag(args, "no_query", false)? && parsed.query().is_some())
                || (flag(args, "no_fragment", false)? && parsed.fragment().is_some())
                || (flag(args, "loopback_host", false)? && !loopback)
                || (flag(args, "origin_only", false)? && !matches!(parsed.path(), "" | "/"))
            {
                return Err(Reason::InvalidValue);
            }
            if flag(args, "production_public_rtsps", false)? {
                let pointer = text(args, "development_pointer")?;
                if !valid_pointer(pointer) {
                    return mapping();
                }
                let development = candidate(value, pointer)?
                    .as_bool()
                    .ok_or(Reason::TypeMismatch)?;
                if !development && (parsed.scheme() != "rtsps" || loopback || unspecified) {
                    return Err(Reason::InvalidValue);
                }
            }
        }
        _ => return mapping(),
    }
    Ok(())
}

fn resolve(spec: &PathSpec, value: &Value, root: &Path, depth: usize) -> Result<PathBuf> {
    if depth >= MAX_PATH_DEPTH || spec.value.len() > MAX_TEXT_BYTES || spec.suffix.len() > 4096 {
        return mapping();
    }
    let selected = match spec.base.as_str() {
        "root" => Some(spec.value.as_str()),
        "config" if valid_pointer(&spec.value) => match value.pointer(&spec.value) {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.as_str()),
            _ => return Err(Reason::TypeMismatch),
        },
        _ => return mapping(),
    };
    let mut path = if let Some(selected) = selected {
        let decoded = match spec.encoding.as_str() {
            "path" => selected,
            "sqlite-url" => selected
                .strip_prefix("sqlite://")
                .filter(|s| !s.contains(['?', '#']))
                .ok_or(Reason::InvalidValue)?,
            _ => return mapping(),
        };
        if spec.base == "root" {
            if !relative(decoded) {
                return Err(Reason::InvalidValue);
            }
            root.join(decoded)
        } else if Path::new(decoded).is_absolute() {
            PathBuf::from(decoded)
        } else {
            root.join(decoded)
        }
    } else {
        resolve(
            spec.fallback.as_ref().ok_or(Reason::MissingField)?,
            value,
            root,
            depth + 1,
        )?
    };
    if !spec.suffix.is_empty() {
        if !relative(&spec.suffix) {
            return Err(Reason::InvalidValue);
        }
        path = path.join(&spec.suffix);
    }
    if !normalized(&path, true) {
        return Err(Reason::InvalidValue);
    }
    Ok(path)
}

#[cfg(test)]
mod tests;
