//! Real candidate-process HTTP smoke with development:true on loopback.
//! Private fixture provisioning is not an installer or production-TLS test.
#[path = "support/database.rs"]
mod database;

use serde_json::{Value, json};
use std::{
    fs,
    net::TcpListener,
    os::unix::fs::PermissionsExt,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

struct Server(Child);
impl Server {
    fn start(root: &Path, base: &str) -> Self {
        let log = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(root.join("logs/smoke.log"))
            .unwrap();
        let child = Command::new(env!("CARGO_BIN_EXE_xscs"))
            .arg("--root")
            .arg(root)
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log))
            .spawn()
            .unwrap();
        let mut server = Self(child);
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let ready = Command::new("curl")
                .args([
                    "--noproxy",
                    "*",
                    "--silent",
                    "--fail",
                    "--max-time",
                    "1",
                    &format!("{base}/readyz"),
                ])
                .output()
                .unwrap();
            if ready.status.success() {
                return server;
            }
            assert!(
                server.0.try_wait().unwrap().is_none(),
                "server exited: {}",
                fs::read_to_string(root.join("logs/smoke.log")).unwrap()
            );
            assert!(Instant::now() < deadline, "readiness timed out");
            thread::sleep(Duration::from_millis(100));
        }
    }
    fn stop(&mut self) {
        assert!(
            Command::new("kill")
                .args(["-TERM", &self.0.id().to_string()])
                .status()
                .unwrap()
                .success()
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(status) = self.0.try_wait().unwrap() {
                assert!(status.success(), "unclean server exit");
                return;
            }
            assert!(Instant::now() < deadline, "shutdown timed out");
            thread::sleep(Duration::from_millis(100));
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn request(
    root: &Path,
    base: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
    csrf: Option<&str>,
) -> Value {
    let cookie = root.join("run/cookies");
    let mut command = Command::new("curl");
    command
        .args([
            "--noproxy",
            "*",
            "--silent",
            "--show-error",
            "--fail",
            "--max-time",
            "5",
            "-X",
            method,
            "-b",
        ])
        .arg(&cookie)
        .arg("-c")
        .arg(&cookie)
        .args([
            "-H",
            &format!("Origin: {base}"),
            "-H",
            "Sec-Fetch-Site: same-origin",
        ]);
    if let Some(csrf) = csrf {
        command.args(["-H", &format!("X-CSRF-Token: {csrf}")]);
    }
    if let Some(body) = body {
        command.args([
            "-H",
            "Content-Type: application/json",
            "--data-binary",
            &body.to_string(),
        ]);
    }
    let result = command.arg(format!("{base}{path}")).output().unwrap();
    assert!(
        result.status.success(),
        "{method} {path} failed: {}",
        String::from_utf8_lossy(&result.stderr)
    );
    serde_json::from_slice(&result.stdout).unwrap()
}
#[tokio::test]
async fn real_process_login_device_changes_and_restart_preserve_state() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    fs::set_permissions(root, fs::Permissions::from_mode(0o700)).unwrap();
    for dir in ["config", "logs", "run"] {
        fs::create_dir(root.join(dir)).unwrap();
        fs::set_permissions(root.join(dir), fs::Permissions::from_mode(0o700)).unwrap();
    }
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let bind = listener.local_addr().unwrap();
    let base = format!("http://{bind}");
    drop(listener);
    let config = root.join("config/config.json");
    fs::write(
        &config,
        json!({"bind":bind.to_string(), "development":true}).to_string(),
    )
    .unwrap();
    fs::set_permissions(config, fs::Permissions::from_mode(0o600)).unwrap();
    database::initialize_root(root).await.unwrap();
    let mut server = Server::start(root, &base);
    let session = request(
        root,
        &base,
        "POST",
        "/api/v1/auth/login",
        Some(json!({"username":"fixture-admin", "password":"fixture-password-2026"})),
        None,
    );
    assert_eq!(session["authenticated"], true);
    let csrf = session["csrf_token"].as_str().unwrap();
    let ticket = request(
        root,
        &base,
        "POST",
        "/api/v1/rustdesk/devices",
        Some(json!({"name":"Original smoke device"})),
        Some(csrf),
    );
    let id = ticket["device"]["id"].as_str().unwrap();
    assert_eq!(ticket["device"]["name"], "Original smoke device");
    let path = format!("/api/v1/rustdesk/devices/{id}");
    let renamed = request(
        root,
        &base,
        "PATCH",
        &path,
        Some(json!({"name":"Renamed smoke device"})),
        Some(csrf),
    );
    assert_eq!(renamed["name"], "Renamed smoke device");
    let authorization_path = format!("{path}/authorization");
    let code = "r".repeat(36);
    let authorization = request(
        root,
        &base,
        "PUT",
        &authorization_path,
        Some(json!({"authorization_code":code})),
        Some(csrf),
    );
    assert_eq!(authorization["authorization_code"], code);
    let before = request(root, &base, "GET", "/api/v1/rustdesk/devices", None, None);
    assert_eq!(before.as_array().unwrap().len(), 1);
    assert_eq!(before[0]["id"], id);
    assert_eq!(before[0]["name"], "Renamed smoke device");
    server.stop();
    let mut server = Server::start(root, &base);
    // Reuse the actual cookie, without re-seeding the database or logging in again.
    let persisted = request(root, &base, "GET", "/api/v1/auth/session", None, None);
    assert_eq!(persisted["authenticated"], true);
    assert_eq!(persisted["user_id"], session["user_id"]);
    let after = request(root, &base, "GET", "/api/v1/rustdesk/devices", None, None);
    assert_eq!(after, before);
    let persisted_authorization = request(root, &base, "GET", &authorization_path, None, None);
    assert_eq!(persisted_authorization, authorization);
    server.stop();
}
