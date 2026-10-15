CREATE TABLE manager_identity (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    manager_id TEXT NOT NULL UNIQUE CHECK(length(manager_id) = 36)
);
CREATE TABLE devices (
    device_id TEXT PRIMARY KEY CHECK(length(device_id) = 36),
    name TEXT NOT NULL CHECK(length(trim(name)) BETWEEN 1 AND 32),
    installation_id TEXT UNIQUE CHECK(installation_id IS NULL OR length(installation_id) = 36),
    credential_hash BLOB UNIQUE CHECK(credential_hash IS NULL OR length(credential_hash) = 32),
    enrollment_hash BLOB UNIQUE CHECK(enrollment_hash IS NULL OR length(enrollment_hash) = 32),
    authorization_code_enc TEXT NOT NULL CHECK(length(authorization_code_enc) BETWEEN 80 AND 1024),
    revoked_at_micros INTEGER,
    session_id TEXT,
    last_seen_at_micros INTEGER,
    health_at_micros INTEGER,
    capabilities_json TEXT,
    diagnostics_json TEXT,
    created_at_micros INTEGER NOT NULL,
    updated_at_micros INTEGER NOT NULL
);
CREATE TABLE client_observations (
    operation_id TEXT PRIMARY KEY REFERENCES _common_operations(operation_id),
    report_json TEXT NOT NULL,
    observed_at_micros INTEGER NOT NULL
);
CREATE TABLE audit_logs (
    audit_id INTEGER PRIMARY KEY AUTOINCREMENT,
    action TEXT NOT NULL CHECK(length(action) BETWEEN 1 AND 128),
    target TEXT NOT NULL CHECK(length(target) BETWEEN 1 AND 255),
    detail TEXT,
    actor TEXT NOT NULL CHECK(length(actor) BETWEEN 1 AND 128),
    created_at_micros INTEGER NOT NULL,
    outbox_id TEXT UNIQUE
);
CREATE INDEX audit_logs_created_at_idx ON audit_logs(created_at_micros DESC);
