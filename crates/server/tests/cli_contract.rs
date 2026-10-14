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
#[path = "support/database.rs"]
mod fixture_database;
#[tokio::test]
async fn runtime_rejects_invalid_installed_administrator_without_repair() {
    use sqlx::Connection as _;
    let (_temp, root, _config) = fixture();
    fixture_database::initialize_root(&root).await.unwrap();
    let path = root.join("state").join(concat!("xscs", ".sqlite3"));
    let mut connection = sqlx::SqliteConnection::connect(&format!("sqlite://{}", path.display()))
        .await
        .unwrap();
    sqlx::query("UPDATE _common_administrators SET password_hash='invalid-persisted-hash'")
        .execute(&mut connection)
        .await
        .unwrap();
    sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
        .execute(&mut connection)
        .await
        .unwrap();
    connection.close().await.unwrap();
    let before = fs::read(&path).unwrap();
    for _ in 0..3 {
        let output = Command::new(BINARY)
            .env_clear()
            .arg("--root")
            .arg(&root)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(fs::read(&path).unwrap(), before);
        assert!(fs::read_dir(root.join("run")).unwrap().next().is_none());
        assert!(!String::from_utf8_lossy(&output.stderr).contains("fixture-password-2026"));
    }
}
#[tokio::test]
async fn runtime_rejects_malformed_instance_key_without_recreating_it() {
    let (_temp, root, _config) = fixture();
    fixture_database::initialize_root(&root).await.unwrap();
    let key = root.join("state/secrets.json");
    fs::write(&key, b"{}").unwrap();
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
