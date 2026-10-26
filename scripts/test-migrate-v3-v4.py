#!/usr/bin/env python3
"""Offline migration behavior, including unchanged rejection paths."""
import contextlib
import fcntl
import hashlib
import importlib.util
import json
from pathlib import Path
import sqlite3
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("xscs_migration", ROOT / "scripts/migrate-v3-v4.py")
MIGRATION = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MIGRATION)


class MigrationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="xscs-v3-migration-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.database = self.root / "xscs.sqlite3"
        self.backup = self.root / "v3-backup.sqlite3"
        with contextlib.closing(sqlite3.connect(self.database)) as connection:
            connection.executescript((ROOT / "schema/legacy/v3.sql").read_text())
            self.assertEqual(MIGRATION.fingerprint(connection), MIGRATION.V3_HASH)
            connection.execute("INSERT INTO product_metadata VALUES(1,'xscs','v3',3,?)", (MIGRATION.V3_HASH,))
            connection.execute("INSERT INTO manager_identity VALUES(1,?)", ("1" * 36,))
            self.token = "controller-secret"
            digest = hashlib.sha256(self.token.encode()).digest()
            for number, role in [("2", "controller"), ("3", "controlled")]:
                connection.execute("INSERT INTO devices(device_id,name,role,installation_id,credential_hash,authorization_code_enc,created_at_micros,updated_at_micros) VALUES(?,?,?,?,?,?,1,1)", (number * 36, role, role, number * 36, digest if role == "controller" else b'h' * 32, "opaque-envelope" * 8))
            connection.execute("INSERT INTO device_access VALUES(?,?)", ("2" * 36, "3" * 36))
            connection.execute("INSERT INTO _common_operations(operation_id,namespace,target_key,action,idempotency_digest,request_fingerprint,request_payload,state,attempt,max_attempts,not_before_micros,created_at_micros,updated_at_micros) VALUES('operation','native.read',?,'rustdesk.diagnostics.read',?,?,?,'pending',0,1,1,1,1)", ("3" * 36, b'i' * 32, b'f' * 32, b'original encrypted request'))
            connection.execute("INSERT INTO audit_logs(action,target,actor,created_at_micros) VALUES('device.create',?,'admin',1)", ("3" * 36,))
            connection.commit()
        self.database.chmod(0o600)

    def snapshot(self):
        return {path.name: path.read_bytes() for path in self.root.iterdir()}

    def test_migration_preserves_native_bindings_acl_task_audit_and_backup(self):
        with contextlib.closing(sqlite3.connect(self.database)) as connection:
            original = {name: connection.execute(f'SELECT * FROM "{name}"').fetchall() for (name,) in connection.execute("SELECT name FROM sqlite_schema WHERE type='table' AND name NOT GLOB 'sqlite_*' ORDER BY name")}
        MIGRATION.migrate(self.database, self.backup)
        self.assertEqual(self.backup.stat().st_mode & 0o777, 0o600)
        with contextlib.closing(sqlite3.connect(self.backup)) as connection:
            MIGRATION.identity(connection, "v3", 3, MIGRATION.V3_HASH)
            self.assertEqual({name: connection.execute(f'SELECT * FROM "{name}"').fetchall() for name in original}, original)
        release = json.loads((ROOT / "release.json").read_text())
        with contextlib.closing(sqlite3.connect(self.database)) as connection:
            MIGRATION.identity(connection, "v4", 4, release["schema_sha256"])
            self.assertEqual(connection.execute("SELECT device_id,backend FROM devices ORDER BY device_id").fetchall(), [("2" * 36, "rustdesk"), ("3" * 36, "rustdesk")])
            self.assertEqual(connection.execute("SELECT device_id FROM devices WHERE credential_hash=?", (hashlib.sha256(self.token.encode()).digest(),)).fetchone(), ("2" * 36,))
            self.assertEqual(connection.execute("SELECT * FROM device_access").fetchall(), [("2" * 36, "3" * 36)])
            self.assertEqual(connection.execute("SELECT request_payload FROM _common_operations").fetchone(), (b'original encrypted request',))
            self.assertEqual(connection.execute("SELECT action FROM audit_logs").fetchone(), ("device.create",))
        before = self.snapshot()
        with self.assertRaisesRegex(MIGRATION.MigrationError, "already v4"):
            MIGRATION.migrate(self.database, self.root / "second-backup.sqlite3")
        self.assertEqual(self.snapshot(), before)

    def test_schema_drift_is_rejected_before_any_writes(self):
        with contextlib.closing(sqlite3.connect(self.database)) as connection:
            connection.execute("CREATE TABLE unexpected(value)")
            connection.commit()
        before = self.snapshot()
        with self.assertRaises(MIGRATION.MigrationError):
            MIGRATION.migrate(self.database, self.backup)
        self.assertEqual(self.snapshot(), before)

    def test_existing_backup_is_never_overwritten(self):
        self.backup.write_bytes(b"existing backup")
        before = self.database.read_bytes()
        with self.assertRaises(FileExistsError):
            MIGRATION.migrate(self.database, self.backup)
        self.assertEqual(self.database.read_bytes(), before)
        self.assertEqual(self.backup.read_bytes(), b"existing backup")

    def test_metadata_definition_is_checked_separately_from_its_excluded_fingerprint(self):
        with contextlib.closing(sqlite3.connect(self.database)) as connection:
            connection.executescript("ALTER TABLE product_metadata RENAME TO old_metadata; CREATE TABLE product_metadata AS SELECT * FROM old_metadata; DROP TABLE old_metadata;")
            self.assertEqual(MIGRATION.fingerprint(connection), MIGRATION.V3_HASH)
        before = self.snapshot()
        with self.assertRaisesRegex(MIGRATION.MigrationError, "product_metadata definition"):
            MIGRATION.migrate(self.database, self.backup)
        self.assertEqual(self.snapshot(), before)

    def test_running_instance_lock_is_respected(self):
        lock = self.root / ".state-instance.lock"
        with lock.open("w+b") as handle:
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            before = self.snapshot()
            with self.assertRaises(BlockingIOError):
                MIGRATION.migrate(self.database, self.backup)
            self.assertEqual(self.snapshot(), before)


if __name__ == "__main__":
    unittest.main()
