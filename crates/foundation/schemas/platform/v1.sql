CREATE TABLE _common_platform_metadata (
    singleton                INTEGER PRIMARY KEY
                                      CHECK (singleton = 1),
    platform_generation      INTEGER NOT NULL
                                      CHECK (platform_generation = 1),
    platform_schema_revision INTEGER NOT NULL
                                      CHECK (platform_schema_revision > 0),
    profile                  TEXT NOT NULL,
    created_at_micros        INTEGER NOT NULL
                                      CHECK (created_at_micros >= 0)
);
