use serde_json::Value;
fn string(v: &Value) -> bool {
    v.is_string()
}
fn nullable_string(v: &Value) -> bool {
    v.is_null() || v.is_string()
}
fn number(v: &Value) -> bool {
    v.is_null() || v.as_f64().is_some_and(f64::is_finite)
}
fn member(v: &Value, choices: &[&str]) -> bool {
    v.as_str().is_some_and(|s| choices.contains(&s))
}
fn fields(v: &Value, keys: &[&str], test: fn(&Value) -> bool) -> bool {
    v.is_object() && keys.iter().all(|k| v.get(k).is_some_and(test))
}
pub fn diagnostics(v: &Value) -> bool {
    let version = match v["backend"].as_str() {
        Some("rustdesk") if v["one_kvm_version"].is_null() => &v["rustdesk_version"],
        Some("one_kvm") if v["rustdesk_version"].is_null() => &v["one_kvm_version"],
        _ => return false,
    };
    v.is_object()
        && v["execution_mode"] == "embedded"
        && member(
            &v["integration_state"],
            &["awaiting_integration", "ready", "unavailable"],
        )
        && nullable_string(version)
        && (v["integration_state"] != "ready" || version.as_str().is_some_and(|v| !v.is_empty()))
        && (v["integration_state"] != "awaiting_integration" || version.is_null())
}
pub fn device(v: &Value) -> bool {
    fields(v, &["id", "name"], string)
        && member(&v["role"], &["controller", "controlled"])
        && (v["backend"].is_null() || member(&v["backend"], &["rustdesk", "one_kvm"]))
        && v["addresses"].as_array().is_some_and(|a| {
            a.iter().all(|v| {
                fields(v, &["host"], string)
                    && v["port"].as_u64().is_some_and(|v| v > 0 && v <= 65535)
            })
        })
        && fields(
            v,
            &["registered", "pairing_pending", "revoked", "client_online"],
            Value::is_boolean,
        )
        && number(&v["addresses_at_micros"])
        && number(&v["last_seen_at_micros"])
        && (v["diagnostics"].is_null()
            || diagnostics(&v["diagnostics"])
                && (v["backend"].is_null() || v["diagnostics"]["backend"] == v["backend"]))
        && (v["capabilities"].is_null()
            || fields(&v["capabilities"], &["client_version", "os"], string)
                && v["capabilities"]["protocol"] == "xscs-management/2"
                && member(&v["capabilities"]["backend"], &["rustdesk", "one_kvm"])
                && (v["backend"].is_null() || v["capabilities"]["backend"] == v["backend"])
                && v["capabilities"]["execution_mode"] == "embedded"
                && v["capabilities"]["diagnostics"] == true)
}
fn code(v: &Value) -> bool {
    v.as_str().is_some_and(|s| {
        s.len() == 36
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
    })
}
fn report(v: &Value) -> bool {
    v.is_null()
        || v["kind"] == "diagnostics_read" && diagnostics(&v["snapshot"])
        || member(&v["kind"], &["rejected", "unknown"]) && string(&v["reason"])
}
fn operation(v: &Value) -> bool {
    fields(
        v,
        &["operation_id", "device_id", "action", "created_at_server"],
        string,
    ) && member(
        &v["state"],
        &[
            "pending",
            "running",
            "succeeded",
            "failed",
            "unknown",
            "dead_letter",
            "resolved",
        ],
    ) && report(&v["result"])
        && report(&v["reconciliation"])
}
pub fn validate(path: &str, v: &Value) -> bool {
    if v.is_null() {
        return false;
    }
    let path = path.split('?').next().unwrap_or(path);
    if path.ends_with("/access") {
        return v["peer_ids"]
            .as_array()
            .is_some_and(|a| a.len() <= 32 && a.iter().all(Value::is_string))
            && v["revision"].as_str().is_some_and(|s| {
                s.len() == 64
                    && s.bytes()
                        .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            });
    }
    if path.ends_with("/authorization") {
        return fields(v, &["manager_id", "device_id"], string) && code(&v["authorization_code"]);
    }
    if path.ends_with("/tasks") {
        return if let Some(a) = v.as_array() {
            a.iter().all(operation)
        } else {
            operation(v)
        };
    }
    if path.ends_with("/devices") {
        return if let Some(a) = v.as_array() {
            a.iter().all(device)
        } else {
            device(&v["device"]) && string(&v["manager_id"]) && code(&v["token"])
        };
    }
    device(v)
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn access_revision_and_limits() {
        assert!(validate(
            "/devices/a/access",
            &json!({"peer_ids":[],"revision":"0".repeat(64)})
        ));
        assert!(!validate(
            "/devices/a/access",
            &json!({"peer_ids":[],"revision":"not-a-revision"})
        ));
    }
    #[test]
    fn inconsistent_diagnostics_rejected() {
        assert!(!diagnostics(
            &json!({"backend":"rustdesk","execution_mode":"embedded","integration_state":"ready","rustdesk_version":null})
        ));
    }

    #[test]
    fn one_kvm_diagnostics_use_the_kvm_version() {
        let mut value = json!({"backend":"one_kvm","execution_mode":"embedded","integration_state":"ready","rustdesk_version":null,"one_kvm_version":"0.2.8"});
        assert!(diagnostics(&value));
        let mut instance = json!({"id":"kvm","name":"KVM","role":"controlled","backend":"one_kvm","addresses":[],"registered":true,"pairing_pending":false,"revoked":false,"client_online":true,"addresses_at_micros":null,"last_seen_at_micros":null,"diagnostics":value,"capabilities":{"protocol":"xscs-management/2","client_version":"0.2.8","backend":"one_kvm","execution_mode":"embedded","diagnostics":true,"os":"linux_armv7"}});
        assert!(device(&instance));
        instance["capabilities"]["os"] = json!("linux_aarch64");
        assert!(device(&instance));
        instance["capabilities"]["backend"] = json!("rustdesk");
        assert!(!device(&instance));
        value["rustdesk_version"] = json!("not-rustdesk");
        assert!(!diagnostics(&value));
        value["rustdesk_version"] = Value::Null;
        value["one_kvm_version"] = Value::Null;
        assert!(!diagnostics(&value));
    }
}
