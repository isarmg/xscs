//! Build-time export; no runtime CLI or installation executor.

use super::{
    ADMIN_PERSISTENT_DDL, ADMINISTRATOR_INSERT_SQL, ADMINISTRATOR_PASSWORD_RESET_SQL,
    ADMINISTRATOR_SESSIONS_REVOKE_SQL, SECURITY_AUDIT_INSERT_SQL,
};
use crate::{admin_auth, admin_core, schema_identity};

/// Generate authoritative public data for a product's installation
/// contract. The product adds its own identity, DDL and business validation.
pub fn installation_contract() -> Result<serde_json::Value, admin_auth::Error> {
    Ok(serde_json::json!({
        "format": "common-administrator-installation-v1",
        "sources": ["src/admin_auth/mod.rs", "src/admin_core/mod.rs", "src/admin_sqlite/mod.rs", "schemas/admin/persistent-v1.sql", "src/schema_identity/mod.rs"],
        "credential_policy": admin_auth::credential_policy(),
        "credential_validation_vectors": admin_auth::credential_validation_vectors()?,
        "authentication_policy": {
            "revision": admin_core::POLICY_REVISION,
            "session_idle_micros": admin_core::SESSION_IDLE_MICROS,
            "session_absolute_micros": admin_core::SESSION_ABSOLUTE_MICROS,
            "sessions_per_administrator": admin_core::SESSIONS_PER_ADMINISTRATOR,
            "sessions_per_installation": admin_core::SESSIONS_GLOBAL,
            "last_seen_write_interval_micros": admin_core::SESSION_LAST_SEEN_WRITE_INTERVAL_MICROS,
            "argon2_concurrency_per_process": admin_core::ARGON2_CONCURRENCY,
            "argon2_acquire_timeout_micros": admin_core::ARGON2_ACQUIRE_TIMEOUT_MICROS,
            "login_attempts": {
                "scope": "main-database-device-and-inode-per-process",
                "scope_identity_lifetime": "verified-read-only-main-file-descriptor-pinned-until-registry-entry-retires",
                "scope_identity_pins_max": admin_core::LOGIN_ATTEMPTS_SCOPES_MAX,
                "counted": "all-admitted-login-attempts-before-credential-lookup",
                "max": admin_core::LOGIN_ATTEMPTS_MAX,
                "window_seconds": admin_core::LOGIN_ATTEMPTS_WINDOW.as_secs(),
                "clock": "monotonic", "reset": "window-expiry-or-process-restart",
                "success_resets": false, "rejections_extend_window": false,
                "scopes_max": admin_core::LOGIN_ATTEMPTS_SCOPES_MAX,
                "http_status": 429, "error_code": admin_core::LOGIN_ATTEMPTS_ERROR_CODE,
                "retry_after": "ceil-remaining-window-seconds"
            },
            "body_max_bytes": admin_core::ADMIN_BODY_MAX_BYTES,
            "body_timeout_millis": admin_core::ADMIN_BODY_TIMEOUT.as_millis(),
            "body_readers_per_process": admin_core::ADMIN_BODY_READERS_GLOBAL,
            "identifier_max_bytes": admin_core::IDENTIFIER_MAX_BYTES,
            "digest_bytes": admin_core::DIGEST_BYTES,
            "administrators_required": admin_core::ADMINISTRATORS_REQUIRED,
            "audit_details_max_bytes": admin_core::AUDIT_DETAILS_MAX_BYTES
        },
        "ddl": ADMIN_PERSISTENT_DDL,
        "metadata": {
            "ddl": schema_identity::PRODUCT_METADATA_DDL,
            "insert_sql": schema_identity::PRODUCT_METADATA_INSERT_SQL,
            "insert_parameters": ["application", "application_version", "schema_revision", "schema_sha256"],
            "fingerprint_algorithm_version": schema_identity::SCHEMA_FINGERPRINT_ALGORITHM_VERSION,
            "fingerprint_query": schema_identity::SQLITE_SCHEMA_ROWS_QUERY,
            "fingerprint_vectors": serde_json::from_str::<serde_json::Value>(schema_identity::SCHEMA_FINGERPRINT_GOLDEN_VECTORS_JSON).expect("embedded fingerprint vectors are JSON")
        },
        "operations": {
            "initialize": {
                "sql": ADMINISTRATOR_INSERT_SQL,
                "parameters": ["administrator_id", "username", "password_hash", "active", "session_version", "created_at_micros", "updated_at_micros", "last_login_at_micros"],
                "precondition": "zero-administrators-in-new-current-schema",
                "active": true, "session_version": 1, "last_login_at_micros": null,
                "audit_action": admin_core::SecurityAction::AdministratorCreated.as_str()
            },
            "reset_password": {
                "sql": ADMINISTRATOR_PASSWORD_RESET_SQL,
                "parameters": ["password_hash", "updated_at_micros", "administrator_id"],
                "precondition": "exactly-one-active-current-administrator",
                "rows_affected": 1,
                "identity_preserved": true,
                "audit_action": admin_core::SecurityAction::AdministratorPasswordChanged.as_str()
            },
            "revoke_sessions": {
                "sql": ADMINISTRATOR_SESSIONS_REVOKE_SQL,
                "parameters": ["revoked_at_micros", "administrator_id"],
                "audit_action": admin_core::SecurityAction::AdministratorSessionsRevoked.as_str()
            },
            "audit": {
                "sql": SECURITY_AUDIT_INSERT_SQL,
                "parameters": ["event_id", "action", "outcome", "actor_administrator_id", "subject_digest", "request_id", "detail_json", "occurred_at_micros"],
                "outcome": admin_core::AuditOutcome::Success.as_str()
            },
            "transaction": "begin-immediate-all-mutations-and-audit-atomic",
            "time": "utc-unix-microseconds-positive-signed-i64",
            "offline_actor_administrator_id": null,
            "offline_request_id": null,
            "offline_detail_json": "{}",
            "subject_digest": "sha256-canonical-username-utf8"
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn credential_export_is_deterministic_and_matches_live_validators() {
        let first = installation_contract().unwrap();
        assert_eq!(first, installation_contract().unwrap());
        let vectors = &first["credential_validation_vectors"];
        for vector in vectors["username"].as_array().unwrap() {
            let input = vector["input"].as_str().unwrap();
            assert_eq!(
                admin_auth::normalize_administrator_username(input)
                    .ok()
                    .as_deref(),
                vector["canonical"].as_str()
            );
            assert_eq!(
                admin_auth::require_canonical_administrator_username(input).is_ok(),
                vector["stored_valid"].as_bool().unwrap()
            );
        }
        for vector in vectors["password"].as_array().unwrap() {
            assert_eq!(
                admin_auth::validate_password(vector["input"].as_str().unwrap()).is_ok(),
                vector["valid"].as_bool().unwrap()
            );
        }
        let vector = &vectors["password_hash"];
        let encoded = vector["encoded"].as_str().unwrap();
        assert!(admin_auth::verify_password(
            vector["password"].as_str().unwrap(),
            encoded
        ));
        assert!(!admin_auth::verify_password(
            vector["incorrect_password"].as_str().unwrap(),
            encoded
        ));
        assert_eq!(
            first["operations"]["reset_password"]["sql"],
            ADMINISTRATOR_PASSWORD_RESET_SQL
        );
        assert_eq!(first["ddl"], ADMIN_PERSISTENT_DDL);
        assert_eq!(
            first["credential_policy"]["password_hash"]["version"],
            u32::from(argon2::Version::V0x13)
        );
        assert_eq!(
            first["credential_policy"]["password_hash"]["algorithm"],
            argon2::Algorithm::Argon2id.to_string()
        );
    }
}
