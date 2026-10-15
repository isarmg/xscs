//! Source-bound current installation materials generated at build time.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::Path;
use xcss::contracts::{ReleaseIdentity, SchemaIdentity};
use xcss::secret_envelope::EnvelopeDomain;
const PRODUCT: &str = "xscs";
fn root_path(value: &str) -> Value {
    json!({"base":"root","value":value,"fallback":null,"suffix":"","encoding":"path"})
}
fn data_file(value: &str) -> Value {
    let mut path = crate::config_definition::data_path_spec();
    path["suffix"] = json!(value);
    path
}
fn asset(directory: &Path, name: &str, bytes: &[u8]) -> Value {
    std::fs::write(directory.join(name), bytes).expect("write current contract asset");
    json!({"path":format!("share/contracts/{name}"),"sha256":hex::encode(Sha256::digest(bytes))})
}
fn relocate(value: &mut Value) {
    match value {
        Value::String(text) => {
            if let Some(suffix) = text.strip_prefix(&format!("/opt/isarmg/{PRODUCT}/")) {
                *text = format!("$ROOT/{suffix}")
            }
        }
        Value::Object(values) => values.values_mut().for_each(relocate),
        Value::Array(values) => values.iter_mut().for_each(relocate),
        _ => {}
    }
}
fn expand(value: &mut Value, root: &str) {
    match value {
        Value::String(text) => {
            if text.starts_with("$ROOT/") {
                *text = text.replacen("$ROOT", root, 1)
            }
        }
        Value::Object(values) => values.values_mut().for_each(|v| expand(v, root)),
        Value::Array(values) => values.iter_mut().for_each(|v| expand(v, root)),
        _ => {}
    }
}
fn config_cases(defaults: &Value) -> Vec<Value> {
    let mut cases = vec![
        json!({"name":"current-defaults","value":defaults,"valid":true}),
        json!({"name":"omitted-fields-current-defaults","value":{},"valid":true}),
    ];
    for (name, field, value, valid) in [
        (
            "unknown-property",
            "removed_timezone",
            json!("local"),
            false,
        ),
        ("unknown-log-level", "log_level", json!("invalid"), false),
        ("invalid-bind", "bind", json!("invalid"), false),
        ("data-required", "data_dir", Value::Null, false),
        ("relative-state", "data_dir", json!("relative"), false),
        (
            "protected-state",
            "data_dir",
            json!("$ROOT/maintenance/private"),
            false,
        ),
        (
            "bound-release-web",
            "development_web_dir",
            json!("$ROOT/external-web"),
            false,
        ),
    ] {
        let mut value_case = defaults.clone();
        value_case[field] = value;
        cases.push(json!({"name":name,"value":value_case,"valid":valid}));
    }
    let mut development = defaults.clone();
    development["development"] = json!(true);
    development["bind"] = json!("0.0.0.0:8080");
    cases.push(json!({"name":"development-loopback-required","value":development,"valid":false}));

    for case in &cases {
        let mut value = defaults.clone();
        for (key, setting) in case["value"].as_object().unwrap() {
            value[key] = setting.clone();
        }
        expand(&mut value, "/installation-vector");
        let valid = serde_json::from_value::<crate::config_definition::Settings>(value)
            .ok()
            .is_some_and(|settings| {
                crate::config_definition::validate(
                    &settings,
                    Path::new("/installation-vector"),
                    true,
                    xcss::config::ConfigSource::File,
                )
                .is_ok()
            });
        assert_eq!(
            valid,
            case["valid"].as_bool().unwrap(),
            "runtime configuration contradicts installation vector {}",
            case["name"]
        );
    }
    cases
}
fn secret_valid(value: Value) -> bool {
    serde_json::from_value::<crate::secret_definition::SecretsDocument>(value)
        .is_ok_and(|secret| secret.key().is_ok())
}
fn secret_material() -> Value {
    let valid = json!({"version":crate::secret_definition::VERSION,"credential_key_id":crate::secret_definition::KEY_ID,"credential_key":STANDARD.encode([9u8;32])});
    let field = "credential_key";
    let mut unknown = valid.clone();
    unknown["unknown"] = json!(true);
    let mut wrong_length = valid.clone();
    wrong_length[field] = json!(STANDARD.encode([9u8; 31]));
    let cases = vec![
        json!({"name":"current-shape","input":valid,"valid":true}),
        json!({"name":"missing-field","input":{},"valid":false}),
        json!({"name":"unknown-field","input":unknown,"valid":false}),
        json!({"name":"wrong-key-length","input":wrong_length,"valid":false}),
    ];
    for case in &cases {
        assert_eq!(
            secret_valid(case["input"].clone()),
            case["valid"].as_bool().unwrap(),
            "runtime secret contradicts installation vector"
        );
    }
    json!({"name":"instance-secrets","path":data_file("secrets.json"),"format":"json",
        "fields":[{"name":field,"bytes":32,"encoding":"base64"}],"constants":json!({"version":crate::secret_definition::VERSION,"credential_key_id":crate::secret_definition::KEY_ID}),"validation":[],"cases":cases})
}
fn authorization_material() -> Value {
    use crate::persistent_crypto_definition::ClientAuthorizationEnvelope;
    json!({"kind":"sgev-authorization-v1","description":"Current product envelope, object binding and authorization digest",
        "parameters":{"domain":std::str::from_utf8(ClientAuthorizationEnvelope::DOMAIN).unwrap(),"revision":ClientAuthorizationEnvelope::REVISION,
            "binding":"uuid-text","key_secret":"instance-secrets.credential_key","plaintext_bytes":36,"digest_encoding":"raw",
            "maximum_rows":2_000_000,"maximum_ciphertext_bytes":1024,"query":"SELECT device_id AS identity,authorization_code_enc AS ciphertext,enrollment_hash AS digest FROM devices","digest_nullable":true,"ciphertext_prefix":format!("{}{}:",crate::persistent_crypto_definition::PREFIX,crate::secret_definition::KEY_ID),"ciphertext_encoding":"base64",},
        "cases":serde_json::from_str::<Value>(include_str!("../../../schema/validation/authorization.json")).unwrap()})
}
pub(crate) fn emit(
    destination: &Path,
    identity: &ReleaseIdentity,
    schema: &SchemaIdentity,
    ddl: &str,
) {
    let directory = destination.join("contracts");
    std::fs::create_dir_all(&directory).expect("create current materials directory");
    let mut material =
        xcss::installation_material::common(identity, schema, Some("server-control-plane"))
            .expect("public installation authority");
    let definitions = material
        .as_object_mut()
        .unwrap()
        .remove("definitions")
        .unwrap();
    asset(
        &directory,
        "common-definitions.json",
        &serde_json::to_vec(&definitions).unwrap(),
    );
    let mut database = material
        .as_object_mut()
        .unwrap()
        .remove("database")
        .unwrap();
    database["name"] = json!(PRODUCT);
    database["path"] = data_file(&format!("{PRODUCT}.sqlite3"));
    database["ddl"] = asset(&directory, "current-schema.sql", ddl.as_bytes());
    database["validation"] = json!([authorization_material()]);
    add_operation_material(&directory, &mut database);
    material["databases"] = json!([database]);
    let mut config = xcss::configuration_contract::generate::<crate::config_definition::Settings>()
        .expect("current owned configuration data");
    config["schema"].as_object_mut().unwrap().remove("required");
    relocate(&mut config["defaults"]);
    config["data_dir"] = crate::config_definition::data_path_spec();
    config["bind_pointer"] = json!("/bind");
    config["rules"] = crate::config_definition::current_rules(true);
    config["cases"] = json!(config_cases(&config["defaults"]));
    material["config"] = config;
    material["secrets"] = json!([secret_material()]);
    let state = crate::config_definition::data_path_spec();
    let instance = data_file(".state-instance.lock");
    let database_lock = data_file(".xscs.sqlite3.xscs.instance.lock");
    material["resources"] = json!({"state_paths":[state,root_path("config")],"writable_paths":[state,root_path("logs"),root_path("run")],"readonly_paths":[root_path("config/config.json")],"shared_paths":[],"business_paths":[],"business_directories":[],"config_write_paths":[],"private_directories":[state,root_path("logs"),root_path("run")],"runtime_locks":[{"path":instance,"kind":"file"},{"path":database_lock,"kind":"file"}]});
    material["writers"] = json!([{"role":"server","service":format!("{PRODUCT}.service"),"configuration_paths":[root_path("config/config.json")]}]);
    material["media"] = Value::Null;
    for (path, bytes) in [
        (
            "crates/server/src/database_definition.rs",
            include_bytes!("database_definition.rs").as_slice(),
        ),
        (
            "crates/server/src/config_definition.rs",
            include_bytes!("config_definition.rs").as_slice(),
        ),
        (
            "crates/server/src/secret_definition.rs",
            include_bytes!("secret_definition.rs").as_slice(),
        ),
        (
            "crates/server/src/persistent_crypto_definition.rs",
            include_bytes!("persistent_crypto_definition.rs").as_slice(),
        ),
        (
            "crates/server/src/installation_definition.rs",
            include_bytes!("installation_definition.rs").as_slice(),
        ),
        (
            "schema/product.sql",
            include_bytes!("../../../schema/product.sql").as_slice(),
        ),
        (
            "schema/validation/authorization.json",
            include_bytes!("../../../schema/validation/authorization.json").as_slice(),
        ),
    ] {
        material["authority_sources"].as_array_mut().unwrap().push(json!({"path":path,"sha256":hex::encode(Sha256::digest(bytes)),"generator":format!("{PRODUCT}/crates/server/build.rs@{}",identity.source_revision)}));
    }
    asset(
        &directory,
        "install.json",
        &serde_json::to_vec(&material).unwrap(),
    );
}

fn add_operation_material(directory: &Path, database: &mut Value) {
    use crate::persistent_crypto_definition::*;
    database["initialize"].as_array_mut().unwrap().push(json!({
        "sql":"INSERT INTO manager_identity(singleton,manager_id) VALUES(1,?)",
        "parameters":[{"source":"uuid-v4","value":"","type":"text"}],"expected_changes":1}));
    database["checks"].as_array_mut().unwrap().push(json!({"name":"manager-identity","sql":"SELECT manager_id FROM manager_identity WHERE singleton=1","expect":"one-row","parameters":[]}));
    asset(
        directory,
        "crypto-definitions.json",
        &serde_json::to_vec(&json!({
        "ciphertext_prefix":PREFIX,"key_id":crate::secret_definition::KEY_ID,
        "aad_format":std::str::from_utf8(AAD_FORMAT).unwrap(),
        "hkdf_salt":std::str::from_utf8(HKDF_SALT).unwrap(),
        "request_fingerprint_info":std::str::from_utf8(REQUEST_FINGERPRINT_INFO).unwrap(),
        "idempotency_key_hash_info":std::str::from_utf8(IDEMPOTENCY_KEY_HASH_INFO).unwrap()}))
        .unwrap(),
    );
    database["validation"].as_array_mut().unwrap().push(json!({"kind":"sqlite-json-schema-v1",
        "description":"Current manager identity decodes as UUID",
        "parameters":{"maximum_bytes":128,"maximum_rows":1,"schema":{"type":"string","format":"uuid"},"rules":[],
            "query":"SELECT json_quote(manager_id) AS value FROM manager_identity"},
        "cases":[{"name":"current-uuid","input":"00112233-4455-4677-8899-aabbccddeeff","valid":true},
            {"name":"nil-current-uuid","input":"00000000-0000-0000-0000-000000000000","valid":true},
            {"name":"invalid-current-uuid","input":"invalid-identity","valid":false}]}));
    let request_schema =
        serde_json::to_value(schemars::schema_for!(xscs_protocol::StoredRequest)).unwrap();
    let mut commands = serde_json::Map::new();
    for command in xscs_protocol::COMMAND_STORAGE_CONTRACTS {
        commands.insert(
            command.kind.into(),
            json!({"action":command.action,"resource_class":command.resource}),
        );
    }
    let parameters = json!({"domain":std::str::from_utf8(OperationRequestEnvelope::DOMAIN).unwrap(),"revision":OperationRequestEnvelope::REVISION,
        "binding_components":[{"source":"literal","value":std::str::from_utf8(AAD_FORMAT).unwrap()},
            {"source":"literal","value":std::str::from_utf8(OPERATION_REQUEST_DOMAIN).unwrap()},
            {"source":"operation_id","value":""},{"source":"action","value":""},
            {"source":"literal","value":std::str::from_utf8(OPERATION_REQUEST_FIELD).unwrap()}],
        "ciphertext_prefix":format!("{PREFIX}{}:",crate::secret_definition::KEY_ID),"ciphertext_encoding":"base64",
        "key_secret":"instance-secrets.credential_key","maximum_rows":2_000_000,
        "maximum_payload_bytes":xscs_protocol::MAX_MESSAGE_BYTES*2,"maximum_plaintext_bytes":xscs_protocol::MAX_MESSAGE_BYTES,
        "query":"SELECT operation_id,action,target_key,namespace AS resource_namespace,CAST(request_payload AS TEXT) AS payload_json,lower(hex(request_fingerprint)) AS request_fingerprint FROM _common_operations",
        "fingerprint":{"algorithm":"hmac-sha256","hkdf_salt":std::str::from_utf8(HKDF_SALT).unwrap(),"hkdf_info":std::str::from_utf8(REQUEST_FINGERPRINT_INFO).unwrap()},
        "request_schema":request_schema,"action_pointer":"/command/kind","binding_device_pointer":"/binding/device_id",
        "binding_installation_pointer":"/binding/installation_id","namespace_prefix":"xscc.client.v2","commands":commands});
    let cases: Value =
        serde_json::from_str(include_str!("../../../schema/validation/operation.json")).unwrap();
    database["validation"].as_array_mut().unwrap().push(json!({"kind":"wrapped-sgev-operation-v1","description":"Current durable request SGEV, keyed fingerprint, strict plaintext and dispatch identity","parameters":parameters,"cases":cases}));
    let definitions = xscs_protocol::persistent_json_definitions();
    for (name, column) in [
        ("snapshot", "diagnostics_json"),
        ("capabilities", "capabilities_json"),
    ] {
        let definition = &definitions[name];
        asset(
            directory,
            &format!("{name}.schema.json"),
            &serde_json::to_vec(&definition["schema"]).unwrap(),
        );
        database["validation"].as_array_mut().unwrap().push(json!({"kind":"sqlite-json-schema-v1","description":format!("Product-owned current {name} semantics"),"parameters":{"maximum_bytes":xscs_protocol::MAX_REPORT_BYTES,"maximum_rows":2_000_000,"schema":definition["schema"],"rules":definition["rules"],"query":format!("SELECT {column} AS value FROM devices WHERE {column} IS NOT NULL")},"cases":definition["cases"]}));
    }
}
