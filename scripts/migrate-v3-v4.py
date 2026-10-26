#!/usr/bin/env python3
"""Explicit offline XSCS v3 -> v4 migration; never run by Server startup."""
from __future__ import annotations

import argparse
import contextlib
import fcntl
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import stat
import sys

PRODUCT_ROOT = Path(__file__).resolve().parents[1]
V3_HASH = "06af04920d4709289c0194f2d9c972dec19a703495bef08d06e3b94b6ee3c676"


class MigrationError(RuntimeError):
    pass


def fingerprint(connection: sqlite3.Connection) -> str:
    digest = hashlib.sha256()
    for row in connection.execute("SELECT type,name,tbl_name,COALESCE(sql,'') FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*' AND name <> 'product_metadata' ORDER BY type,name,tbl_name"):
        for value in row:
            data = value.encode("utf-8")
            digest.update(len(data).to_bytes(8, "big"))
            digest.update(data)
    return digest.hexdigest()


def identity(connection: sqlite3.Connection, version: str, revision: int, digest: str) -> None:
    expected_metadata = (PRODUCT_ROOT / "crates/foundation/schemas/product-metadata/v1.sql").read_text().strip().removesuffix(";")
    metadata = connection.execute("SELECT sql FROM sqlite_schema WHERE type='table' AND name='product_metadata'").fetchone()
    normalize = lambda sql: "".join(char.lower() for char in sql if char not in " \t\n\r\v\f")
    if metadata is None or normalize(metadata[0]) != normalize(expected_metadata):
        raise MigrationError("database product_metadata definition is invalid")
    row = connection.execute("SELECT application,application_version,schema_revision,schema_sha256 FROM product_metadata WHERE singleton=1").fetchone()
    if row != ("xscs", version, revision, digest) or fingerprint(connection) != digest:
        raise MigrationError("database schema/metadata does not match the required XSCS identity")
    if connection.execute("PRAGMA integrity_check").fetchall() != [("ok",)]:
        raise MigrationError("database integrity check failed")
    if connection.execute("PRAGMA foreign_key_check").fetchall():
        raise MigrationError("database foreign-key check failed")


def sync_file(path: Path) -> None:
    descriptor = os.open(path, os.O_RDONLY | os.O_CLOEXEC)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)


def migrate(database: Path, backup: Path) -> None:
    database = database.absolute()
    info = database.lstat()
    if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
        raise MigrationError("database must be a regular file with one link")
    if any(Path(str(database) + suffix).exists() for suffix in ("-wal", "-shm", "-journal")):
        raise MigrationError("stop the v3 service cleanly and checkpoint/close it first; preserve existing SQLite sidecars")
    release = json.loads((PRODUCT_ROOT / "release.json").read_text())
    if release["schema_revision"] != 4:
        raise MigrationError("this migration requires the v4 product schema")
    target_hash = release["schema_sha256"]
    with contextlib.closing(sqlite3.connect(":memory:")) as target:
        target.executescript((PRODUCT_ROOT / "schema/generated/current_schema.sql").read_text())
        if fingerprint(target) != target_hash:
            raise MigrationError("current product DDL does not match release schema identity")
        table = target.execute("SELECT sql FROM sqlite_schema WHERE type='table' AND name='devices'").fetchone()[0]
        triggers = [row[0] for row in target.execute("SELECT sql FROM sqlite_schema WHERE type='trigger' AND tbl_name='devices' ORDER BY name")]
    uri = database.as_uri() + "?mode=ro&immutable=1"
    with contextlib.closing(sqlite3.connect(uri, uri=True)) as source:
        current = source.execute("SELECT application_version,schema_revision FROM product_metadata WHERE singleton=1").fetchone()
        if current == ("v4", 4):
            identity(source, "v4", 4, target_hash)
            raise MigrationError("database is already v4; no changes made")
        identity(source, "v3", 3, V3_HASH)
    with contextlib.ExitStack() as locks:
        for name in (".state-instance.lock", f".{database.name}.xscs.instance.lock"):
            descriptor = os.open(database.parent / name, os.O_RDWR | os.O_CREAT | os.O_CLOEXEC | os.O_NOFOLLOW, 0o600)
            locks.callback(os.close, descriptor)
            fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        with contextlib.closing(sqlite3.connect(database, isolation_level=None, timeout=0)) as connection:
            connection.execute("PRAGMA foreign_keys=OFF")
            connection.execute("PRAGMA temp_store=MEMORY")
            connection.execute("BEGIN IMMEDIATE")
            try:
                identity(connection, "v3", 3, V3_HASH)
                descriptor = os.open(backup, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_CLOEXEC, 0o600)
                os.close(descriptor)
                with contextlib.closing(sqlite3.connect(database.as_uri() + "?mode=ro", uri=True)) as source, contextlib.closing(sqlite3.connect(backup)) as destination:
                    source.backup(destination)
                    identity(destination, "v3", 3, V3_HASH)
                sync_file(backup)
                sync_file(backup.parent)
                columns = ",".join('"' + row[1] + '"' for row in connection.execute("PRAGMA table_info(devices)"))
                connection.execute("CREATE TEMP TABLE migration_devices AS SELECT * FROM devices")
                connection.execute("DROP TABLE devices")
                connection.execute(table)
                connection.execute(f"INSERT INTO devices({columns},backend) SELECT {columns},'rustdesk' FROM migration_devices")
                connection.execute("DROP TABLE migration_devices")
                for trigger in triggers:
                    connection.execute(trigger)
                connection.execute("UPDATE product_metadata SET application_version='v4',schema_revision=4,schema_sha256=? WHERE singleton=1", (target_hash,))
                identity(connection, "v4", 4, target_hash)
                connection.execute("COMMIT")
                connection.execute("PRAGMA wal_checkpoint(TRUNCATE)")
            except BaseException:
                if connection.in_transaction:
                    connection.execute("ROLLBACK")
                raise
        sync_file(database)
        sync_file(database.parent)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", type=Path, required=True)
    parser.add_argument("--backup", type=Path, required=True, help="new private backup file; existing paths are refused")
    options = parser.parse_args()
    try:
        migrate(options.database, options.backup)
    except (MigrationError, OSError, sqlite3.Error) as error:
        print(f"XSCS migration refused: {error}", file=sys.stderr)
        return 1
    print(f"Migrated XSCS v3 -> v4. Original database backup: {options.backup}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
