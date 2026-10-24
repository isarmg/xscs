use super::*;
use serde::Deserialize;

#[derive(Debug)]
struct ValidatedSecret;
impl<'de> Deserialize<'de> for ValidatedSecret {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = String::deserialize(deserializer)?;
        if value == "valid" {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom("sensitive validator diagnostic"))
        }
    }
}

#[test]
fn custom_deserializers_keep_nested_object_and_array_paths_without_values() {
    #[derive(Debug, Deserialize)]
    struct Nested {
        _items: Vec<ValidatedSecret>,
    }
    // Rename lets the test exercise JSON-pointer escaping in an actual field.
    #[derive(Debug, Deserialize)]
    struct Config {
        #[serde(rename = "a/b~c")]
        nested: Nested,
    }
    for (value, expected) in [
        (
            json!({"a/b~c":{"_items":["valid","private-input"]}}),
            "/a~1b~0c/_items/1",
        ),
        (json!({"a/b~c":{"_items":[1]}}), "/a~1b~0c/_items/0"),
    ] {
        let error = strict::deserialize::<Config>(value, ConfigSource::File).unwrap_err();
        assert_eq!(error.path, expected);
        assert_eq!(error.source, ConfigSource::File);
        let rendered = serde_json::to_string(&error.envelope()).unwrap();
        assert!(!rendered.contains("private-input"));
        assert!(!rendered.contains("sensitive validator diagnostic"));
    }
    let valid =
        strict::deserialize::<Config>(json!({"a/b~c":{"_items":["valid"]}}), ConfigSource::File)
            .unwrap();
    assert_eq!(valid.nested._items.len(), 1);
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Config {
    listen: String,
    workers: u16,
    tls: Tls,
    targets: Vec<String>,
    #[serde(default)]
    optional: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Tls {
    enabled: bool,
    password: String,
}

fn defaults() -> Config {
    Config {
        listen: "127.0.0.1:8000".into(),
        workers: 4,
        tls: Tls {
            enabled: true,
            password: "default-secret".into(),
        },
        targets: vec![],
        optional: None,
    }
}

#[test]
fn explicit_sources_have_one_precedence_and_nested_leaf_provenance() {
    let env = read_environment(
        &[
            EnvMapping {
                variable: "APP_LISTEN",
                path: "/listen",
                kind: EnvValueKind::String,
            },
            EnvMapping {
                variable: "APP_WORKERS",
                path: "/workers",
                kind: EnvValueKind::UnsignedInteger,
            },
        ],
        |key| match key {
            "APP_LISTEN" => Some("127.0.0.1:9000".into()),
            "APP_WORKERS" => Some("12".into()),
            _ => panic!("undeclared environment name read"),
        },
    )
    .unwrap();
    let file = br#"{"listen":"127.0.0.1:8080","workers":8,"tls":{"password":"file-secret"}}"#;
    let result = resolve(
        &defaults(),
        Some(file),
        &env,
        &[Override::new("/listen", "127.0.0.1:10000")],
    )
    .unwrap();
    assert_eq!(result.value.listen, "127.0.0.1:10000");
    assert_eq!(result.value.workers, 12);
    assert!(result.value.tls.enabled);
    assert_eq!(result.value.tls.password, "file-secret");
    assert_eq!(result.sources["/listen"], ConfigSource::CommandLine);
    assert_eq!(result.sources["/workers"], ConfigSource::Environment);
    assert_eq!(result.sources["/tls/password"], ConfigSource::File);
    assert_eq!(result.sources["/tls/enabled"], ConfigSource::Default);
    let diagnostic = serde_json::to_string(&result.sources).unwrap();
    assert!(!diagnostic.contains("secret"));
}

#[test]
fn malformed_lower_priority_input_cannot_be_hidden_by_an_override() {
    let error = resolve(
        &defaults(),
        Some(br#"{"workers":"secret-number"}"#),
        &[],
        &[Override::new("/workers", 2)],
    )
    .err()
    .unwrap();
    assert_eq!(error.reason, Reason::TypeMismatch);
    assert_eq!(error.path, "/workers");
    assert_eq!(error.source, ConfigSource::File);
    assert!(
        !serde_json::to_string(&error.envelope())
            .unwrap()
            .contains("secret-number")
    );
}

#[test]
fn product_value_constraints_are_checked_before_higher_priority_overrides() {
    let validate = |config: &Config, source| {
        if config.workers == 0 {
            Err(ConfigError::new(Reason::InvalidValue, "/workers", source))
        } else {
            Ok(())
        }
    };
    let result = resolve_validated(
        &defaults(),
        Some(br#"{"workers":0}"#),
        &[],
        &[Override::new("/workers", 2)],
        validate,
    );
    let error = result.err().unwrap();
    assert_eq!(
        (error.reason, error.path.as_str(), error.source),
        (Reason::InvalidValue, "/workers", ConfigSource::File)
    );
}

#[test]
fn rejects_unknown_fields_without_requiring_a_serde_annotation() {
    for (bytes, path) in [
        (&br#"{"old_listen":"secret"}"#[..], "/old_listen"),
        (&br#"{"tls":{"unknown":"secret"}}"#[..], "/tls/unknown"),
    ] {
        let error = resolve(&defaults(), Some(bytes), &[], &[]).err().unwrap();
        assert_eq!(error.reason, Reason::UnknownField);
        assert_eq!(error.path, path);
    }
}

#[test]
fn rejects_duplicate_fields_invalid_arrays_missing_fields_and_null() {
    for (bytes, reason, path) in [
        (
            &br#"{"tls":{"password":"a","password":"b"}}"#[..],
            Reason::DuplicateField,
            "/tls/password",
        ),
        (
            &br#"{"targets":["valid",9]}"#[..],
            Reason::TypeMismatch,
            "/targets/1",
        ),
        (&br#"{"tls":null}"#[..], Reason::TypeMismatch, "/tls"),
        (
            &br#"{"tls":[true,"array-secret"]}"#[..],
            Reason::TypeMismatch,
            "/tls",
        ),
        (&br#"[]"#[..], Reason::TypeMismatch, ""),
    ] {
        let error = resolve(&defaults(), Some(bytes), &[], &[]).err().unwrap();
        assert_eq!((error.reason, error.path.as_str()), (reason, path));
    }
    let error = resolve(
        &defaults(),
        None,
        &[],
        &[Override::new("/tls", json!({"enabled":true}))],
    )
    .err()
    .unwrap();
    assert_eq!(
        (error.reason, error.path.as_str()),
        (Reason::MissingField, "/tls/password")
    );
}

#[test]
fn environment_types_are_explicit_and_errors_never_expose_values() {
    for kind in [
        EnvValueKind::Boolean,
        EnvValueKind::UnsignedInteger,
        EnvValueKind::SignedInteger,
        EnvValueKind::Json,
    ] {
        let error = read_environment(
            &[EnvMapping {
                variable: "APP_INPUT",
                path: "/workers",
                kind,
            }],
            |_| Some("secret-invalid".into()),
        )
        .unwrap_err();
        assert!(!format!("{error:?}").contains("secret-invalid"));
        assert_eq!(error.path, "/workers");
        assert_eq!(error.source, ConfigSource::Environment);
    }
    let error = read_environment(
        &[EnvMapping {
            variable: "APP_TLS",
            path: "/tls",
            kind: EnvValueKind::Json,
        }],
        |_| Some(r#"{"enabled":false,"enabled":true}"#.into()),
    )
    .unwrap_err();
    assert_eq!(
        (error.reason, error.path.as_str()),
        (Reason::DuplicateField, "/tls/enabled")
    );
}

#[test]
fn input_budgets_invalid_pointers_and_ambiguous_overrides_fail() {
    let error = resolve(
        &defaults(),
        Some(&vec![b' '; MAX_CONFIG_BYTES + 1]),
        &[],
        &[],
    )
    .err()
    .unwrap();
    assert_eq!(error.reason, Reason::LimitExceeded);
    for path in ["workers", "/tls/~2", "/targets/0"] {
        assert!(resolve(&defaults(), None, &[], &[Override::new(path, 1)]).is_err());
    }
    let error = resolve(
        &defaults(),
        None,
        &[],
        &[Override::new("/workers", 1), Override::new("/workers", 2)],
    )
    .err()
    .unwrap();
    assert_eq!(error.reason, Reason::DuplicateField);
    assert!(
        !format!("{:?}", Override::new("/tls/password", "debug-secret")).contains("debug-secret")
    );
}

#[derive(Debug, Serialize, Deserialize, PartialEq)]
enum Mode {
    Local,
    Remote { host: String },
    Port(u16),
}
#[derive(Debug, Serialize, Deserialize)]
struct EnumConfig {
    mode: Mode,
}

#[test]
fn normal_current_serde_enum_contracts_work_with_safe_value_failures() {
    let default = EnumConfig { mode: Mode::Local };
    for (bytes, expected) in [
        (&br#"{"mode":"Local"}"#[..], Mode::Local),
        (
            &br#"{"mode":{"Remote":{"host":"localhost"}}}"#[..],
            Mode::Remote {
                host: "localhost".into(),
            },
        ),
        (&br#"{"mode":{"Port":80}}"#[..], Mode::Port(80)),
    ] {
        assert_eq!(
            resolve(&default, Some(bytes), &[], &[]).unwrap().value.mode,
            expected
        );
    }
    let error = resolve(&default, Some(br#"{"mode":"enum-secret"}"#), &[], &[])
        .err()
        .unwrap();
    assert_eq!(error.reason, Reason::InvalidValue);
    assert!(!format!("{error:?}").contains("enum-secret"));
}

#[cfg(unix)]
#[test]
fn private_configuration_read_is_bounded_readonly_and_rejects_unsafe_files() {
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, symlink},
    };
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let path = directory.path().join("config.json");
    fs::write(&path, b"{\"workers\":4}").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(read_private_file(&path).unwrap(), b"{\"workers\":4}");
    assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    let missing = directory.path().join("missing.json");
    assert_eq!(
        read_private_file(&missing)
            .unwrap_err()
            .envelope()
            .code
            .as_str(),
        "config_file_not_found"
    );
    let fifo = directory.path().join("fifo.json");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::Mode::RUSR | rustix::fs::Mode::WUSR,
    )
    .unwrap();
    assert_eq!(
        read_private_file(&fifo).unwrap_err().reason,
        Reason::UnsafeFile
    );
    fs::remove_file(fifo).unwrap();
    let link = directory.path().join("link.json");
    symlink(&path, &link).unwrap();
    assert!(read_private_file(&link).is_err());
    fs::hard_link(&path, directory.path().join("alias.json")).unwrap();
    assert!(read_private_file(&path).is_err());
    fs::remove_file(directory.path().join("alias.json")).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
    assert!(read_private_file(&path).is_err());
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::write(&path, vec![0; MAX_CONFIG_BYTES + 1]).unwrap();
    assert_eq!(
        read_private_file(&path).unwrap_err().reason,
        Reason::LimitExceeded
    );
}

#[test]
fn managed_configuration_rejects_unsafe_modes_links_groups_and_unbound_owner() {
    use std::{
        fs,
        os::unix::fs::{PermissionsExt, chown, symlink},
    };
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let path = directory.path().join("config.json");
    fs::write(&path, b"{\"workers\":4}").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    assert_eq!(read_managed_file(&path, true).unwrap(), b"{\"workers\":4}");
    if rustix::process::geteuid().as_raw() != 0 {
        assert_eq!(
            read_managed_file(&path, false).unwrap_err().reason,
            Reason::UnsafeFile
        );
    } else {
        assert_eq!(read_managed_file(&path, false).unwrap(), b"{\"workers\":4}");
    }
    for mode in [0o644, 0o660, 0o664, 0o4640] {
        fs::set_permissions(&path, fs::Permissions::from_mode(mode)).unwrap();
        assert_eq!(
            read_managed_file(&path, true).unwrap_err().reason,
            Reason::UnsafeFile
        );
    }
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    fs::hard_link(&path, directory.path().join("hard-link")).unwrap();
    assert_eq!(
        read_managed_file(&path, true).unwrap_err().reason,
        Reason::UnsafeFile
    );
    fs::remove_file(directory.path().join("hard-link")).unwrap();
    symlink(&path, directory.path().join("link")).unwrap();
    assert_eq!(
        read_managed_file(&directory.path().join("link"), true)
            .unwrap_err()
            .reason,
        Reason::UnsafeFile
    );
    let nested = directory.path().join("nested");
    fs::create_dir(&nested).unwrap();
    fs::set_permissions(&nested, fs::Permissions::from_mode(0o700)).unwrap();
    fs::copy(&path, nested.join("config.json")).unwrap();
    symlink(&nested, directory.path().join("directory-link")).unwrap();
    assert_eq!(
        read_managed_file(&directory.path().join("directory-link/config.json"), true)
            .unwrap_err()
            .reason,
        Reason::UnsafeFile
    );
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o757)).unwrap();
    assert_eq!(
        read_managed_file(&nested.join("config.json"), true)
            .unwrap_err()
            .reason,
        Reason::UnsafeFile
    );
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let fifo = directory.path().join("fifo");
    rustix::fs::mkfifoat(
        rustix::fs::CWD,
        &fifo,
        rustix::fs::Mode::from_raw_mode(0o600),
    )
    .unwrap();
    assert_eq!(
        read_managed_file(&fifo, true).unwrap_err().reason,
        Reason::UnsafeFile
    );
    fs::write(&path, vec![0; MAX_CONFIG_BYTES + 1]).unwrap();
    assert_eq!(
        read_managed_file(&path, true).unwrap_err().reason,
        Reason::LimitExceeded
    );
    assert_eq!(
        read_managed_file(&directory.path().join("missing"), true)
            .unwrap_err()
            .reason,
        Reason::MissingFile
    );
    if rustix::process::geteuid().as_raw() == 0 {
        fs::write(&path, b"{} ").unwrap();
        chown(&path, Some(0), Some(33)).unwrap();
        assert_eq!(
            read_managed_file(&path, false).unwrap_err().reason,
            Reason::UnsafeFile
        );
        chown(&path, Some(33), Some(0)).unwrap();
        assert_eq!(
            read_managed_file(&path, false).unwrap_err().reason,
            Reason::UnsafeFile
        );
    }
}

#[test]
fn managed_configuration_is_readable_but_not_writable_by_the_real_service_uid() {
    use std::{
        fs,
        os::unix::{
            fs::{PermissionsExt, chown},
            process::CommandExt,
        },
        process::Command,
    };
    if rustix::process::geteuid().as_raw() != 0 {
        // Creating root-controlled installation fixtures requires an actual
        // administrator. Non-root CI still exercises all portable negatives.
        return;
    }
    let root = tempfile::tempdir().unwrap();
    fs::set_permissions(root.path(), fs::Permissions::from_mode(0o750)).unwrap();
    chown(root.path(), Some(0), Some(33)).unwrap();
    let config_dir = root.path().join("config");
    fs::create_dir(&config_dir).unwrap();
    fs::set_permissions(&config_dir, fs::Permissions::from_mode(0o750)).unwrap();
    chown(&config_dir, Some(0), Some(33)).unwrap();
    let path = config_dir.join("config.json");
    fs::write(&path, b"{\"workers\":4}").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
    chown(&path, Some(0), Some(33)).unwrap();
    let run = |uid, kind: &str| {
        let result = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "config::tests::managed_configuration_uid_helper",
                "--ignored",
            ])
            .env("XCSS_CONFIG_TEST_ROOT", root.path())
            .env("XCSS_CONFIG_TEST_KIND", kind)
            .uid(uid)
            .gid(uid)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "helper failed: {} {}",
            String::from_utf8_lossy(&result.stdout),
            String::from_utf8_lossy(&result.stderr)
        );
        let output = String::from_utf8_lossy(&result.stdout);
        assert!(
            output.contains("running 1 test") && output.contains("1 passed"),
            "UID helper must run exactly once: {output}"
        );
    };
    run(33, "managed");
    run(34, "unrelated");
    assert_eq!(fs::read(&path).unwrap(), b"{\"workers\":4}");
    assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    assert_eq!(fs::read_dir(&config_dir).unwrap().count(), 1);
    // Development ownership is explicit and cannot be used by a release build.
    chown(&path, Some(33), Some(33)).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    chown(&config_dir, Some(33), Some(33)).unwrap();
    fs::set_permissions(&config_dir, fs::Permissions::from_mode(0o700)).unwrap();
    run(33, "development");
}

#[test]
fn managed_configuration_rejects_additional_access_and_default_acl() {
    use std::{fs, os::unix::fs::PermissionsExt};
    let directory = tempfile::tempdir().unwrap();
    fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let path = directory.path().join("config.json");
    fs::write(&path, b"{}").unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    // Linux POSIX ACL xattr v2: owner, named user, owning group, mask, other.
    // The named user has no effective rights, so even unchanged mode 0600 must
    // not disguise an additional access policy or inherit it into later files.
    let mut acl = 2u32.to_le_bytes().to_vec();
    for (tag, permission, identity) in [
        (1u16, 6u16, u32::MAX),
        (2, 4, 34),
        (4, 0, u32::MAX),
        (16, 0, u32::MAX),
        (32, 0, u32::MAX),
    ] {
        acl.extend_from_slice(&tag.to_le_bytes());
        acl.extend_from_slice(&permission.to_le_bytes());
        acl.extend_from_slice(&identity.to_le_bytes());
    }
    match rustix::fs::setxattr(
        &path,
        "system.posix_acl_access",
        &acl,
        rustix::fs::XattrFlags::empty(),
    ) {
        Err(rustix::io::Errno::NOTSUP) => return,
        result => result.unwrap(),
    }
    assert_eq!(
        read_managed_file(&path, true).unwrap_err().reason,
        Reason::UnsafeFile
    );
    rustix::fs::removexattr(&path, "system.posix_acl_access").unwrap();
    assert_eq!(read_managed_file(&path, true).unwrap(), b"{}");
    rustix::fs::setxattr(
        directory.path(),
        "system.posix_acl_default",
        &acl,
        rustix::fs::XattrFlags::empty(),
    )
    .unwrap();
    assert_eq!(
        read_managed_file(&path, true).unwrap_err().reason,
        Reason::UnsafeFile
    );
}

#[test]
#[ignore = "executed in an isolated UID child by the managed configuration test"]
fn managed_configuration_uid_helper() {
    use std::{fs, os::unix::fs::MetadataExt, path::PathBuf};
    let root = PathBuf::from(std::env::var_os("XCSS_CONFIG_TEST_ROOT").expect("isolated fixture"));
    let kind = std::env::var("XCSS_CONFIG_TEST_KIND").unwrap();
    let config_dir = root.join("config");
    let path = config_dir.join("config.json");
    if kind == "unrelated" {
        assert_eq!(rustix::process::geteuid().as_raw(), 34);
        assert!(read_managed_file(&path, false).is_err());
        return;
    }
    assert_eq!(rustix::process::geteuid().as_raw(), 33);
    if kind == "development" {
        assert_eq!(
            read_managed_file(&path, false).unwrap_err().reason,
            Reason::UnsafeFile
        );
        assert_eq!(read_managed_file(&path, true).unwrap(), b"{\"workers\":4}");
        return;
    }
    assert_eq!(fs::metadata(&path).unwrap().uid(), 0);
    assert_eq!(fs::metadata(&path).unwrap().gid(), 33);
    assert_eq!(read_managed_file(&path, false).unwrap(), b"{\"workers\":4}");
    assert!(read_private_file(&path).is_err());
    let denied = |result: std::io::Result<()>| {
        assert_eq!(
            result.unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
    };
    denied(fs::OpenOptions::new().write(true).open(&path).map(|_| ()));
    denied(
        fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .map(|_| ()),
    );
    denied(fs::write(config_dir.join("new-config"), b"{}"));
    denied(fs::write(root.join("new-root-file"), b"{}"));
    denied(fs::remove_file(&path));
    denied(fs::rename(&path, config_dir.join("renamed-config")));
    denied(fs::rename(&config_dir, root.join("renamed-directory")));
}

#[test]
fn deny_unknown_fields_reports_each_key_once() {
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct StrictChild {}
    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct StrictRoot {
        #[serde(rename = "child")]
        _child: StrictChild,
    }
    for (value, expected) in [
        (json!({"old_key":"private-value"}), "/old_key"),
        (json!({"child":{"a/b~c":"private-value"}}), "/child/a~1b~0c"),
    ] {
        let error = strict::deserialize::<StrictRoot>(value, ConfigSource::File).unwrap_err();
        assert_eq!(error.reason, Reason::UnknownField);
        assert_eq!(error.path, expected);
        assert!(
            !serde_json::to_string(&error.envelope())
                .unwrap()
                .contains("private-value")
        );
    }
}

#[test]
fn cross_field_constraints_are_checked_for_the_complete_override_source() {
    #[derive(Serialize, Deserialize)]
    struct QueueConfig {
        queue_capacity: usize,
        batch_size: usize,
    }
    let defaults = QueueConfig {
        queue_capacity: 256,
        batch_size: 64,
    };
    let validate = |value: &QueueConfig, source| {
        if value.batch_size > value.queue_capacity {
            Err(ConfigError::new(
                Reason::InvalidValue,
                "/batch_size",
                source,
            ))
        } else {
            Ok(())
        }
    };
    for overrides in [
        vec![
            Override::new("/queue_capacity", 32),
            Override::new("/batch_size", 16),
        ],
        vec![
            Override::new("/batch_size", 16),
            Override::new("/queue_capacity", 32),
        ],
    ] {
        for (environment, command_line, source) in [
            (overrides.as_slice(), &[][..], ConfigSource::Environment),
            (&[][..], overrides.as_slice(), ConfigSource::CommandLine),
        ] {
            let loaded = resolve_validated(&defaults, None, environment, command_line, validate)
                .expect("a valid complete source must not depend on field application order");
            assert_eq!(loaded.value.queue_capacity, 32);
            assert_eq!(loaded.value.batch_size, 16);
            assert_eq!(loaded.sources["/queue_capacity"], source);
            assert_eq!(loaded.sources["/batch_size"], source);
        }
    }
    let error = resolve_validated(
        &defaults,
        None,
        &[Override::new("/queue_capacity", 32)],
        &[Override::new("/batch_size", 16)],
        validate,
    )
    .err()
    .unwrap();
    assert_eq!(
        (error.reason, error.source),
        (Reason::InvalidValue, ConfigSource::Environment)
    );
}
