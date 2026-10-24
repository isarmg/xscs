CREATE TABLE product_metadata (
    singleton INTEGER PRIMARY KEY NOT NULL CHECK(singleton=1),
    application TEXT NOT NULL,
    application_version TEXT NOT NULL,
    schema_revision INTEGER NOT NULL,
    schema_sha256 TEXT NOT NULL
);
