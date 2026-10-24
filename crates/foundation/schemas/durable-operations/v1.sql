CREATE TABLE _common_operations (
    operation_id TEXT PRIMARY KEY NOT NULL,
    namespace TEXT NOT NULL,
    target_key TEXT NOT NULL,
    action TEXT NOT NULL,
    idempotency_digest BLOB NOT NULL CHECK (length(idempotency_digest) = 32),
    request_fingerprint BLOB NOT NULL CHECK (length(request_fingerprint) = 32),
    request_payload BLOB NOT NULL,
    result_payload BLOB,
    state TEXT NOT NULL CHECK (
        state IN ('pending','running','succeeded','failed','unknown','dead_letter','resolved')
    ),
    attempt INTEGER NOT NULL CHECK (attempt >= 0),
    max_attempts INTEGER NOT NULL CHECK (max_attempts > 0),
    not_before_micros INTEGER NOT NULL,
    lease_owner TEXT,
    lease_expiry_micros INTEGER,
    error_code TEXT,
    resolution_code TEXT,
    created_at_micros INTEGER NOT NULL,
    updated_at_micros INTEGER NOT NULL,
    UNIQUE (namespace, idempotency_digest),
    CHECK ((lease_owner IS NULL) = (lease_expiry_micros IS NULL)),
    CHECK (state = 'running' OR lease_owner IS NULL),
    CHECK (updated_at_micros >= created_at_micros)
);

CREATE INDEX _common_operations_claimable
    ON _common_operations(namespace, not_before_micros, created_at_micros, operation_id)
    WHERE state = 'pending';

CREATE UNIQUE INDEX _common_operations_active_target
    ON _common_operations(namespace, target_key)
    WHERE state IN ('running', 'unknown');

CREATE TABLE _common_operation_audit_outbox (
    event_id TEXT PRIMARY KEY NOT NULL,
    operation_id TEXT NOT NULL REFERENCES _common_operations(operation_id) ON DELETE RESTRICT,
    from_state TEXT NOT NULL,
    to_state TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    created_at_micros INTEGER NOT NULL,
    delivered_at_micros INTEGER
);

CREATE INDEX _common_operation_audit_outbox_pending
    ON _common_operation_audit_outbox(created_at_micros, event_id)
    WHERE delivered_at_micros IS NULL;
