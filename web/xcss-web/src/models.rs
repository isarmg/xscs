//! Browser DTOs. The existing runtime contracts run before these are constructed.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeviceRole {
    Controller,
    Controlled,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Address {
    pub host: String,
    pub port: u16,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Diagnostics {
    pub backend: String,
    pub execution_mode: String,
    pub integration_state: String,
    pub rustdesk_version: Option<String>,
    #[serde(flatten)]
    pub extensions: serde_json::Map<String, serde_json::Value>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Capabilities {
    pub os: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub role: DeviceRole,
    pub addresses: Vec<Address>,
    pub registered: bool,
    pub revoked: bool,
    pub client_online: bool,
    // Preserve JSON integer precision until the JS Date rendering boundary.
    pub addresses_at_micros: Option<serde_json::Number>,
    pub last_seen_at_micros: Option<serde_json::Number>,
    pub diagnostics: Option<Diagnostics>,
    pub capabilities: Option<Capabilities>,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Enrollment {
    pub device: Device,
    pub token: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Authorization {
    pub authorization_code: String,
}
#[derive(Clone, Debug, Deserialize)]
pub struct Access {
    pub peer_ids: Vec<String>,
    pub revision: String,
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Pending,
    Running,
    Succeeded,
    Failed,
    Unknown,
    DeadLetter,
    Resolved,
}
impl OperationState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Running => "running",
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Unknown => "unknown",
            Self::DeadLetter => "dead_letter",
            Self::Resolved => "resolved",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DiagnosticReport {
    DiagnosticsRead {
        snapshot: Diagnostics,
        #[serde(flatten)]
        extensions: serde_json::Map<String, serde_json::Value>,
    },
    Rejected {
        reason: String,
        #[serde(flatten)]
        extensions: serde_json::Map<String, serde_json::Value>,
    },
    Unknown {
        reason: String,
        #[serde(flatten)]
        extensions: serde_json::Map<String, serde_json::Value>,
    },
}
#[derive(Clone, Debug, Deserialize)]
pub struct Operation {
    pub action: String,
    pub created_at_server: String,
    pub state: OperationState,
    pub result: Option<DiagnosticReport>,
    pub reconciliation: Option<DiagnosticReport>,
}
impl Operation {
    pub fn display_result(&self) -> String {
        serde_json::to_string_pretty(&self.reconciliation.as_ref().or(self.result.as_ref()))
            .unwrap_or_default()
    }
}

#[derive(Serialize)]
pub struct CreateDevice {
    pub name: String,
    pub role: DeviceRole,
}
#[derive(Serialize)]
pub struct RenameDevice {
    pub name: String,
}
#[derive(Serialize)]
pub struct UpdateAccess {
    pub peer_ids: Vec<String>,
    pub expected_revision: String,
}
#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeviceCommand {
    ReadDiagnostics,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use xcss_web::api::decode_validated_response;
    #[test]
    fn access_keeps_revision_and_peer_limit_guard() {
        let decode = |v| {
            decode_validated_response::<Access>(v, |v| {
                crate::contracts::validate("/devices/a/access", v)
            })
        };
        let revision = "a".repeat(64);
        assert_eq!(
            decode(json!({"peer_ids":["peer"],"revision":revision}))
                .unwrap()
                .revision,
            revision
        );
        assert!(decode(json!({"peer_ids":[],"revision":"bad"})).is_err());
        assert!(decode(json!({"peer_ids":vec!["peer";33],"revision":revision})).is_err());
    }
    #[test]
    fn device_keeps_exact_micros_and_nullable_fields() {
        let mut v = json!({"id":"a","name":"A","role":"controlled","addresses":[],"registered":true,"pairing_pending":false,"revoked":false,"client_online":false,"addresses_at_micros":9007199254740993i64,"last_seen_at_micros":null,"diagnostics":null,"capabilities":null});
        let d: Device = decode_validated_response(v.clone(), crate::contracts::device).unwrap();
        assert_eq!(
            d.addresses_at_micros.unwrap().as_i64(),
            Some(9007199254740993)
        );
        assert!(d.last_seen_at_micros.is_none());
        v["addresses"] = json!([{"host":"localhost","port":65536}]);
        assert!(decode_validated_response::<Device>(v, crate::contracts::device).is_err());
    }
    #[test]
    fn operation_does_not_accept_unknown_states_or_result_kinds() {
        let valid = json!({"operation_id":"a","device_id":"b","action":"read_diagnostics","created_at_server":"now","state":"unknown","result":{"kind":"rejected","reason":"offline","detail":"kept"},"reconciliation":null});
        let decode = |v| {
            decode_validated_response::<Operation>(v, |v| {
                crate::contracts::validate("/devices/a/tasks", v)
            })
        };
        let decoded = decode(valid.clone()).unwrap();
        assert!(decoded.display_result().contains("offline"));
        assert!(decoded.display_result().contains("kept"));
        let mut invalid = valid.clone();
        invalid["state"] = json!("surprise");
        assert!(decode(invalid).is_err());
        let mut invalid = valid;
        invalid["result"]["kind"] = json!("surprise");
        assert!(decode(invalid).is_err());
    }
}
