use serde_json::json;
use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    process::Command,
};
const BINARY: &str = env!("CARGO_BIN_EXE_xscs");
fn private(path: &Path) {
    fs::create_dir(path).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).unwrap();
}
fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("root");
    private(&root);
    for name in ["config", "logs", "run"] {
        private(&root.join(name));
    }
    let config = root.join("config/config.json");
    fs::write(
        &config,
        serde_json::to_vec(&json!({"bind":"127.0.0.1:0","development":true})).unwrap(),
    )
    .unwrap();
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).unwrap();
    (temp, root, config)
}
fn snapshot(root: &Path) -> BTreeMap<PathBuf, (u32, Vec<u8>, std::time::SystemTime)> {
    fn walk(
        root: &Path,
        path: &Path,
        result: &mut BTreeMap<PathBuf, (u32, Vec<u8>, std::time::SystemTime)>,
    ) {
        for entry in fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            let meta = fs::symlink_metadata(&path).unwrap();
            assert!(!meta.file_type().is_symlink());
            if meta.is_dir() {
                walk(root, &path, result);
            } else {
                result.insert(
                    path.strip_prefix(root).unwrap().to_owned(),
                    (
                        meta.permissions().mode(),
                        fs::read(&path).unwrap(),
                        meta.modified().unwrap(),
                    ),
                );
            }
        }
    }
    let mut result = BTreeMap::new();
    walk(root, root, &mut result);
    result
}
#[test]
fn runtime_rejects_removed_administrative_and_release_commands() {
    for command in [
        "run",
        "init",
        "check",
        "status",
        "config",
        "admin",
        "identity",
        "verify-release",
        "release-identity",
        "release-verify",
        "web-assets",
        "doctor",
        "migrate",
    ] {
        let output = Command::new(BINARY)
            .env_clear()
            .arg(command)
            .output()
            .unwrap();
        assert!(
            !output.status.success(),
            "removed command {command} was accepted"
        );
    }
    for option in ["--help", "--version"] {
        assert!(Command::new(BINARY).arg(option).status().unwrap().success());
    }
}
#[test]
fn runtime_rejects_uninitialized_state_without_creating_installation_state() {
    let (_temp, root, _config) = fixture();
    private(&root.join("state"));
    let before = snapshot(&root);
    let output = Command::new(BINARY)
        .env_clear()
        .arg("--root")
        .arg(&root)
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(snapshot(&root), before);
}
#[tokio::test]
async fn offline_state_checks_are_read_only_and_reset_preserves_instance_keys() {
    let (_temp, root, config) = fixture();
    let before = snapshot(&root);
    let report = xscs::offline::validate_config(&root, &config)
        .await
        .unwrap();
    assert_eq!(report.data_dir, root.join("state"));
    assert_eq!(snapshot(&root), before);
    assert!(xscs::offline::check(&root, &config).await.is_err());
    assert_eq!(snapshot(&root), before);
    xscs::offline::initialize(&root, &config, "fixture-admin", "fixture-password-2026")
        .await
        .unwrap();
    let key = fs::read(root.join("state/secrets.json")).unwrap();
    let before = snapshot(&root);
    xscs::offline::check(&root, &config).await.unwrap();
    xscs::offline::check(&root, &config).await.unwrap();
    assert_eq!(snapshot(&root), before);
    let gate = root.join("state/.state-maintenance-pending.json");
    fs::write(&gate, b"fixture gate owned by the offline coordinator").unwrap();
    fs::set_permissions(&gate, fs::Permissions::from_mode(0o600)).unwrap();
    let before = snapshot(&root);
    xscs::offline::check(&root, &config).await.unwrap();
    assert_eq!(snapshot(&root), before);
    fs::remove_file(gate).unwrap();
    assert!(
        xscs::offline::initialize(&root, &config, "fixture-admin", "another-password-2026")
            .await
            .is_err()
    );
    xscs::offline::reset_password(&root, &config, "fixture-admin", "replacement-password-2026")
        .await
        .unwrap();
    assert_eq!(fs::read(root.join("state/secrets.json")).unwrap(), key);
    xscs::offline::check(&root, &config).await.unwrap();
    let schema = report.schema_identity;
    let before = snapshot(&root);
    assert!(
        xscs::offline::migrate(&root, &config, &root.join("wrong-state"), &schema, &schema)
            .await
            .is_err()
    );
    assert_eq!(snapshot(&root), before);
    xscs::offline::migrate(&root, &config, &root.join("state"), &schema, &schema)
        .await
        .unwrap();
    assert_eq!(snapshot(&root), before);
    assert_eq!(fs::read(root.join("state/secrets.json")).unwrap(), key);
    let before = snapshot(&root);
    xscs::offline::check(&root, &config).await.unwrap();
    assert_eq!(snapshot(&root), before);
}

#[tokio::test]
async fn lightweight_startup_inputs_do_not_replace_complete_read_only_state_validation() {
    use sqlx::Connection as _;
    let (_temp, root, config) = fixture();
    xscs::offline::initialize(&root, &config, "fixture-admin", "fixture-password-2026")
        .await
        .unwrap();
    let value = xscs::ServeConfig::from_root_sources(&root, &config).unwrap();
    let mut connection = sqlx::SqliteConnection::connect(&value.database_url)
        .await
        .unwrap();
    sqlx::query("UPDATE _common_administrators SET password_hash = 'invalid-persisted-hash'")
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
    let before = snapshot(&root);
    xscs::offline::validate_runtime_inputs(&value).unwrap();
    assert_eq!(snapshot(&root), before);
    assert!(xscs::offline::check(&root, &config).await.is_err());
    assert_eq!(snapshot(&root), before);
}
