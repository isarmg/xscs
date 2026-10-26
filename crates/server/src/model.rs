use crate::error::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use xscs_protocol::{Backend, Capabilities, DiagnosticSnapshot, HostAddress, InstanceRole};

#[derive(Debug, Serialize)]
pub struct DeviceView {
    pub id: String,
    pub name: String,
    pub role: InstanceRole,
    pub backend: Backend,
    pub addresses: Vec<HostAddress>,
    pub addresses_at_micros: Option<i64>,
    pub registered: bool,
    pub pairing_pending: bool,
    pub revoked: bool,
    pub client_online: bool,
    pub diagnostics: Option<DiagnosticSnapshot>,
    pub capabilities: Option<Capabilities>,
    pub last_seen_at_micros: Option<i64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeviceName {
    pub name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateDeviceRequest {
    pub name: Option<String>,
    pub role: InstanceRole,
    #[serde(default)]
    pub backend: Backend,
}
#[derive(Debug, Serialize)]
pub struct ClientAuthorization {
    pub manager_id: String,
    pub device_id: String,
    pub authorization_code: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateClientAuthorization {
    pub authorization_code: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationResolutionRequest {
    pub resolution: xcss::operations::Resolution,
}
pub fn validate_instance_name(value: &str) -> AppResult<()> {
    if value.trim().is_empty()
        || value != value.trim()
        || value.chars().count() > 32
        || value.chars().any(char::is_control)
    {
        return Err(AppError::BadRequest(
            "实例名称必须为 1–32 个字符，不能包含控制字符或首尾空白字符".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_request_allows_the_server_default_only_when_name_is_omitted() {
        let request: CreateDeviceRequest =
            serde_json::from_value(serde_json::json!({"role":"controlled"})).unwrap();
        assert!(request.name.is_none());
        assert_eq!(request.backend, Backend::Rustdesk);
        let kvm: CreateDeviceRequest =
            serde_json::from_value(serde_json::json!({"role":"controlled","backend":"one_kvm"}))
                .unwrap();
        assert_eq!(kvm.backend, Backend::OneKvm);
        assert!(
            serde_json::from_value::<CreateDeviceRequest>(
                serde_json::json!({"role":"controlled","backend":"unknown"}),
            )
            .is_err()
        );
        assert!(serde_json::from_value::<CreateDeviceRequest>(serde_json::json!({})).is_err());
        assert!(
            serde_json::from_value::<CreateDeviceRequest>(serde_json::json!({"other": true}))
                .is_err()
        );
    }

    #[test]
    fn instance_name_uses_unicode_scalar_length_and_rust_whitespace() {
        let marker = '\u{feff}';
        assert!(validate_instance_name(&format!("{marker}{}", "a".repeat(31))).is_ok());
        assert!(validate_instance_name(&format!("{marker}{}", "a".repeat(32))).is_err());
        assert!(validate_instance_name("\u{3000}name").is_err());
        assert!(validate_instance_name("name\u{3000}").is_err());
    }
}
