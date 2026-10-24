CREATE TABLE _common_administrators (
    administrator_id TEXT PRIMARY KEY
                          CHECK (length(administrator_id) BETWEEN 1 AND 64),
    username TEXT NOT NULL UNIQUE
                  CHECK (
                      length(username) BETWEEN 3 AND 64
                      AND username = lower(trim(username))
                      AND username NOT GLOB '*[^a-z0-9._-]*'
                      AND substr(username, 1, 1) GLOB '[a-z0-9]'
                      AND substr(username, -1, 1) GLOB '[a-z0-9]'
                  ),
    password_hash TEXT NOT NULL
                       CHECK (length(password_hash) > 0),
    active INTEGER NOT NULL DEFAULT 1
                   CHECK (active IN (0, 1)),
    session_version INTEGER NOT NULL DEFAULT 1
                            CHECK (session_version > 0),
    created_at_micros INTEGER NOT NULL
                              CHECK (created_at_micros >= 0),
    updated_at_micros INTEGER NOT NULL
                              CHECK (updated_at_micros >= created_at_micros),
    last_login_at_micros INTEGER
);

CREATE TABLE _common_admin_sessions (
    session_id TEXT PRIMARY KEY
                    CHECK (length(session_id) BETWEEN 1 AND 64),
    administrator_id TEXT NOT NULL
                          REFERENCES _common_administrators(administrator_id)
                          ON DELETE RESTRICT,
    token_hash BLOB NOT NULL UNIQUE
                    CHECK (length(token_hash) = 32),
    csrf_hash BLOB NOT NULL
                   CHECK (length(csrf_hash) = 32),
    administrator_session_version INTEGER NOT NULL
                                          CHECK (
                                              administrator_session_version > 0
                                          ),
    created_at_micros INTEGER NOT NULL,
    last_seen_at_micros INTEGER NOT NULL,
    idle_expires_at_micros INTEGER NOT NULL,
    absolute_expires_at_micros INTEGER NOT NULL,
    revoked_at_micros INTEGER,
    CHECK (last_seen_at_micros >= created_at_micros),
    CHECK (idle_expires_at_micros > created_at_micros),
    CHECK (
        absolute_expires_at_micros >= idle_expires_at_micros
    )
);

CREATE INDEX _common_admin_sessions_administrator_idx
    ON _common_admin_sessions(
        administrator_id,
        revoked_at_micros
    );

CREATE INDEX _common_admin_sessions_expiry_idx
    ON _common_admin_sessions(
        idle_expires_at_micros,
        absolute_expires_at_micros
    )
    WHERE revoked_at_micros IS NULL;

CREATE TABLE _common_security_audit_events (
    event_id TEXT PRIMARY KEY,
    action TEXT NOT NULL,
    outcome TEXT NOT NULL
                 CHECK (outcome IN ('success', 'failure')),
    actor_administrator_id TEXT,
    subject_digest BLOB,
    request_id TEXT,
    detail_json TEXT NOT NULL DEFAULT '{}',
    occurred_at_micros INTEGER NOT NULL
);
