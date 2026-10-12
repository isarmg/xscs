use std::process::Command;

use xscs::release_contract::{BinaryIdentity, SOURCE_REVISION, embedded};

#[test]
fn identity_command_reports_the_embedded_current_contract() {
    let output = Command::new(env!("CARGO_BIN_EXE_xscs"))
        .arg("identity")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(output.stderr.is_empty());

    let reported: BinaryIdentity = serde_json::from_slice(&output.stdout).unwrap();
    let embedded = embedded().unwrap();
    assert_eq!(reported.manifest_format, embedded.manifest_format);
    assert_eq!(reported.application, embedded.application);
    assert_eq!(reported.version, embedded.version);
    assert_eq!(reported.api_prefix, embedded.api_prefix);
    assert_eq!(reported.schema_revision, embedded.schema_revision);
    assert_eq!(reported.schema_sha256, embedded.schema_sha256);
    assert_eq!(reported.target, embedded.target);
    assert_eq!(reported.source_revision, SOURCE_REVISION);
}

#[test]
fn release_tooling_targets_only_the_current_product_contract() {
    let package_release = include_str!("../../../scripts/package-release.py");
    let manifest_writer = include_str!("../../../scripts/write-release-manifest.py");
    let systemd_unit = include_str!("../../../deploy/xscs.service");

    assert!(package_release.contains("Cargo.toml"));
    assert!(manifest_writer.contains("Cargo.toml"));
    assert!(manifest_writer.contains(&format!(
        "identity[\"schema_revision\"] != {}",
        xscs::database_schema::SCHEMA_REVISION
    )));
    let starts: Vec<_> = systemd_unit
        .lines()
        .filter(|line| line.starts_with("ExecStart="))
        .collect();
    assert_eq!(starts.len(), 1);
    assert!(starts[0].contains("/opt/isarmg/xscs/bin/xscs"));
    assert!(starts[0].contains("/etc/isarmg/xscs/config.json"));
    assert!(!starts[0].contains("--release-root"));
    assert!(!systemd_unit.contains("DynamicUser="));
    assert!(systemd_unit.contains("User=isarmg-xscs"));

    for source in [package_release, manifest_writer, systemd_unit] {
        assert!(!source.contains("0.7.0"));
    }
}
