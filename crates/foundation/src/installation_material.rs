//! Pure build-time assembly of current shared installation definitions.
//!
//! Products bind these fragments to their release and add their owned database,
//! configuration, secrets and resources. This module performs no filesystem
//! access and executes neither a server nor an installation operation.

use crate::{
    admin_auth, admin_core, admin_sqlite, contracts::ReleaseIdentity, platform_db,
    schema_identity::SchemaIdentity,
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

/// Bounded complete session validation in independent current-state checkers.
/// This includes retained revoked sessions, unlike the active-session budget.
pub const MAXIMUM_SESSION_CHECK_ROWS: usize = 2_000_000;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid release identity")]
    Release(#[from] crate::contracts::ValidationError),
    #[error("invalid schema identity")]
    Schema(#[from] crate::schema_identity::Error),
    #[error("cannot generate current credential vectors")]
    Credentials(#[from] admin_auth::Error),
    #[error("unsupported installation profile or target")]
    Unsupported,
}

/// Generate shared fragments. Only the control-plane profile owns the shared
/// platform metadata; `None` exports the shared administrator tables alone.
pub fn common(
    identity: &ReleaseIdentity,
    schema: &SchemaIdentity,
    profile: Option<&str>,
) -> Result<Value, Error> {
    identity.validate()?;
    schema.validate()?;
    if identity.target != "x86_64-unknown-linux-gnu"
        || schema.schema_revision > crate::contracts::MAX_SAFE_JSON_INTEGER
        || !matches!(profile, None | Some("server-control-plane"))
    {
        return Err(Error::Unsupported);
    }
    let definition = admin_sqlite::installation_contract()?;
    let mut initialize = vec![statement(
        crate::schema_identity::PRODUCT_METADATA_INSERT_SQL,
        vec![
            parameter("schema.application", json!(schema.application), "text"),
            parameter(
                "schema.application_version",
                json!(schema.application_version),
                "text",
            ),
            parameter(
                "schema.schema_revision",
                json!(schema.schema_revision),
                "integer",
            ),
            parameter("schema.schema_sha256", json!(schema.schema_sha256), "text"),
        ],
        Some(1),
    )];
    let mut checks = Vec::new();
    if let Some(profile) = profile {
        let values = vec![
            literal(json!(platform_db::PLATFORM_GENERATION), "integer"),
            literal(json!(platform_db::PLATFORM_SCHEMA_REVISION), "integer"),
            literal(json!(profile), "text"),
        ];
        let mut insert_values = values.clone();
        insert_values.push(parameter("now_micros", Value::Null, "integer"));
        initialize.push(statement(
            platform_db::PLATFORM_METADATA_INSERT_SQL,
            insert_values,
            Some(1),
        ));
        checks.push(json!({"name":"current-platform-metadata", "sql":platform_db::PLATFORM_METADATA_CHECK_SQL,"parameters":values,"expect":"true"}));
    }
    initialize.push(statement(
        admin_sqlite::ADMINISTRATOR_INSERT_SQL,
        vec![
            parameter("administrator_id", Value::Null, "text"),
            parameter("username", Value::Null, "text"),
            parameter("password_hash", Value::Null, "text"),
            literal(json!(true), "boolean"),
            literal(json!(1), "integer"),
            parameter("now_micros", Value::Null, "integer"),
            parameter("now_micros", Value::Null, "integer"),
            literal(Value::Null, "null"),
        ],
        Some(1),
    ));
    initialize.push(audit(admin_core::SecurityAction::AdministratorCreated));
    let reset = vec![
        statement(
            admin_sqlite::ADMINISTRATOR_PASSWORD_RESET_SQL,
            vec![
                parameter("password_hash", Value::Null, "text"),
                parameter("now_micros", Value::Null, "integer"),
                parameter("administrator_id", Value::Null, "text"),
            ],
            Some(1),
        ),
        statement(
            admin_sqlite::ADMINISTRATOR_SESSIONS_REVOKE_SQL,
            vec![
                parameter("now_micros", Value::Null, "integer"),
                parameter("administrator_id", Value::Null, "text"),
            ],
            None,
        ),
        audit(admin_core::SecurityAction::AdministratorPasswordChanged),
        audit(admin_core::SecurityAction::AdministratorSessionsRevoked),
    ];
    let vectors = &definition["credential_validation_vectors"];
    let mut cases = Vec::new();
    for vector in vectors["username"]
        .as_array()
        .expect("owned vectors are arrays")
    {
        let username = vector["input"].as_str().expect("owned username vector");
        let password = "contract-fixture-password";
        let normalized = admin_auth::normalize_administrator_username(username).ok();
        cases.push(json!({"name":format!("username-{}", cases.len()),"username":username,"password":password,"normalized_username":normalized,"valid":normalized.is_some()}));
    }
    for vector in vectors["password"]
        .as_array()
        .expect("owned vectors are arrays")
    {
        cases.push(json!({"name":format!("password-{}", cases.len()),"username":"admin","password":vector["input"],"normalized_username":"admin","valid":vector["valid"]}));
    }
    let policy = &definition["credential_policy"];
    let credentials = json!({
        "policy_revision": admin_core::POLICY_REVISION,
        "username": {"normalization":"ascii-trim-lowercase","minimum_bytes":policy["username"]["min_bytes"],"maximum_bytes":policy["username"]["max_bytes"],"pattern":admin_auth::ADMINISTRATOR_USERNAME_PATTERN,"reject_ascii_control":true},
        "password": {"minimum_bytes":policy["password"]["min_bytes"],"maximum_bytes":policy["password"]["max_bytes"],"reject_ascii_control":true},
        "argon2": {"algorithm":policy["password_hash"]["algorithm"],"version":policy["password_hash"]["version"],"memory_kib":policy["password_hash"]["memory_kib"],"iterations":policy["password_hash"]["iterations"],"parallelism":policy["password_hash"]["parallelism"],"output_bytes":policy["password_hash"]["output_bytes"],"salt_bytes":policy["password_hash"]["salt_bytes"]},
        "administrator_query":admin_sqlite::ADMINISTRATORS_QUERY_SQL,
        "session_query":admin_sqlite::ADMIN_SESSIONS_QUERY_SQL,
        "session_digest_bytes":admin_core::DIGEST_BYTES,
        "maximum_session_rows":MAXIMUM_SESSION_CHECK_ROWS,
        "administrator_id_format":"base64url-no-pad",
        "identifier_max_bytes":admin_core::IDENTIFIER_MAX_BYTES,
        "identifier_random_bytes":admin_auth::SESSION_TOKEN_BYTES,
        "cases":cases
    });
    Ok(json!({
        "contract_version":1,"identity":identity,"schema_identity":schema,
        "authority_sources": authority_sources(), "credentials":credentials,
        "database": {"schema_identity":schema,"schema_stamp":schema,"fingerprint_algorithm":"sqlite-schema-v1","initialize":initialize,"reset_administrator":reset,"checks":checks,"validation":[]},
        "definitions": {"administrator":definition,"envelope":crate::secret_envelope::envelope_contract(),"platform":{"generation":platform_db::PLATFORM_GENERATION,"schema_revision":platform_db::PLATFORM_SCHEMA_REVISION,"ddl":platform_db::PLATFORM_METADATA_DDL,"insert_sql":platform_db::PLATFORM_METADATA_INSERT_SQL,"check_sql":platform_db::PLATFORM_METADATA_CHECK_SQL}}
    }))
}

fn parameter(source: &str, value: Value, kind: &str) -> Value {
    json!({"source":source,"value":value,"type":kind})
}
fn literal(value: Value, kind: &str) -> Value {
    parameter("literal", value, kind)
}
fn statement(sql: &str, parameters: Vec<Value>, expected_changes: Option<u64>) -> Value {
    json!({"sql":sql,"parameters":parameters,"expected_changes":expected_changes})
}
fn audit(action: admin_core::SecurityAction) -> Value {
    statement(
        admin_sqlite::SECURITY_AUDIT_INSERT_SQL,
        vec![
            parameter("event_id", Value::Null, "text"),
            literal(json!(action.as_str()), "text"),
            literal(json!(admin_core::AuditOutcome::Success.as_str()), "text"),
            literal(Value::Null, "null"),
            parameter("username_sha256", Value::Null, "blob"),
            literal(Value::Null, "null"),
            literal(json!("{}"), "text"),
            parameter("now_micros", Value::Null, "integer"),
        ],
        Some(1),
    )
}
fn authority_sources() -> Vec<Value> {
    [
        (
            "xcss/src/admin_auth/mod.rs",
            include_bytes!("admin_auth/mod.rs").as_slice(),
        ),
        (
            "xcss/src/admin_core/mod.rs",
            include_bytes!("admin_core/mod.rs").as_slice(),
        ),
        (
            "xcss/src/admin_sqlite/mod.rs",
            include_bytes!("admin_sqlite/mod.rs").as_slice(),
        ),
        (
            "xcss/src/admin_sqlite/installation_contract.rs",
            include_bytes!("admin_sqlite/installation_contract.rs").as_slice(),
        ),
        (
            "xcss/schemas/admin/persistent-v1.sql",
            include_bytes!("../schemas/admin/persistent-v1.sql").as_slice(),
        ),
        (
            "xcss/src/schema_identity/mod.rs",
            include_bytes!("schema_identity/mod.rs").as_slice(),
        ),
        (
            "xcss/src/platform_db/mod.rs",
            include_bytes!("platform_db/mod.rs").as_slice(),
        ),
        (
            "xcss/schemas/platform/v1.sql",
            include_bytes!("../schemas/platform/v1.sql").as_slice(),
        ),
        (
            "xcss/src/secret_envelope/mod.rs",
            include_bytes!("secret_envelope/mod.rs").as_slice(),
        ),
        (
            "xcss/src/installation_material.rs",
            include_bytes!("installation_material.rs").as_slice(),
        ),
        (
            "xcss/src/configuration_contract.rs",
            include_bytes!("configuration_contract.rs").as_slice(),
        ),
        (
            "xcss/src/configuration_contract/rules.rs",
            include_bytes!("configuration_contract/rules.rs").as_slice(),
        ),
        (
            "xcss/src/server_layout.rs",
            include_bytes!("server_layout.rs").as_slice(),
        ),
    ]
    .into_iter()
    .map(|(path, bytes)| {
        let digest = Sha256::digest(bytes);
        let sha256: String = digest
            .iter()
            .flat_map(|byte| {
                const HEX: &[u8; 16] = b"0123456789abcdef";
                [
                    HEX[usize::from(byte >> 4)] as char,
                    HEX[usize::from(byte & 15)] as char,
                ]
            })
            .collect();
        json!({"path":path,"sha256":sha256,"generator":"xcss::installation_material::common"})
    })
    .collect()
}

#[cfg(test)]
mod tests;
