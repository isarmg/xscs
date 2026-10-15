use xscs_protocol::*;
#[test]
fn mobile_platforms_are_controllers_and_desktop_platforms_support_both_roles() {
    for (os, mobile) in [
        (ClientOs::LinuxX86_64, false),
        (ClientOs::WindowsX86_64, false),
        (ClientOs::MacosAarch64, false),
        (ClientOs::IosAarch64, true),
        (ClientOs::AndroidAarch64, true),
    ] {
        assert!(os.supports_role(InstanceRole::Controller));
        assert_eq!(os.supports_role(InstanceRole::Controlled), !mobile);
        let encoded = serde_json::to_string(&os).unwrap();
        assert_eq!(serde_json::from_str::<ClientOs>(&encoded).unwrap(), os);
    }
}
#[test]
fn accepts_only_embedded_rustdesk_and_current_contract() {
    let valid = serde_json::json!({"protocol":PROTOCOL,"client_version":"2.0.0-dev","os":"linux_x86_64","backend":"rustdesk","execution_mode":"embedded","diagnostics":true});
    let caps: Capabilities = serde_json::from_value(valid.clone()).unwrap();
    assert!(caps.validate().is_ok());
    for (field, value) in [
        ("backend", serde_json::json!("sunshine")),
        ("execution_mode", serde_json::json!("standalone")),
        ("protocol", serde_json::json!("xscs-management/1")),
    ] {
        let mut invalid = valid.clone();
        invalid[field] = value;
        assert!(
            serde_json::from_value::<Capabilities>(invalid)
                .map(|c| c.validate().is_err())
                .unwrap_or(true)
        );
    }
    for kind in [
        "save_config",
        "patch_config",
        "restart",
        "list_applications",
        "submit_pairing_pin",
        "control_service",
    ] {
        assert!(serde_json::from_value::<Command>(serde_json::json!({"kind":kind})).is_err())
    }
}
#[test]
fn readiness_requires_observed_native_version() {
    let mut snapshot = DiagnosticSnapshot::awaiting_integration();
    assert!(snapshot.validate().is_ok());
    snapshot.integration_state = IntegrationState::Ready;
    assert!(snapshot.validate().is_err());
    snapshot.rustdesk_version = Some("1.5.1-custom".into());
    assert!(snapshot.validate().is_ok());
    snapshot.rustdesk_version = Some("version\nforged-log".into());
    assert!(snapshot.validate().is_err());
}
