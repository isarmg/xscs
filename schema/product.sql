CREATE TABLE manager_identity (
    singleton INTEGER PRIMARY KEY CHECK(singleton = 1),
    manager_id TEXT NOT NULL UNIQUE CHECK(length(manager_id) = 36)
);
CREATE TABLE devices (
    device_id TEXT PRIMARY KEY CHECK(length(device_id) = 36),
    name TEXT NOT NULL CHECK(length(trim(name)) BETWEEN 1 AND 32),
    role TEXT NOT NULL CHECK(role IN ('controller','controlled')),
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
    addresses_json TEXT,
    addresses_at_micros INTEGER,
    created_at_micros INTEGER NOT NULL,
    updated_at_micros INTEGER NOT NULL
);
CREATE TRIGGER devices_role_immutable BEFORE UPDATE OF role ON devices
WHEN NEW.role != OLD.role BEGIN
    SELECT RAISE(ABORT, 'instance role is immutable');
END;
CREATE TABLE device_access (
    controller_id TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    controlled_id TEXT NOT NULL REFERENCES devices(device_id) ON DELETE CASCADE,
    PRIMARY KEY(controller_id, controlled_id),
    CHECK(controller_id != controlled_id)
);
CREATE INDEX device_access_controlled_idx ON device_access(controlled_id, controller_id);
CREATE TRIGGER device_access_roles BEFORE INSERT ON device_access BEGIN
    SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM devices WHERE device_id=NEW.controller_id AND role='controller')
        OR NOT EXISTS(SELECT 1 FROM devices WHERE device_id=NEW.controlled_id AND role='controlled')
        THEN RAISE(ABORT, 'access requires controller and controlled roles') END;
END;
CREATE TRIGGER device_access_immutable BEFORE UPDATE ON device_access BEGIN
    SELECT RAISE(ABORT, 'replace access using delete and insert');
END;
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
