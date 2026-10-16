#!/usr/bin/env python3
"""Exercise the source inventory without invoking compilers or the network."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("builder", Path(__file__).with_name("build-leptos.py"))
builder = importlib.util.module_from_spec(spec)
spec.loader.exec_module(builder)


class InventoryTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.frontend = self.root / "web/leptos"
        self.shared = self.frontend / "vendor/xcss-web"
        for path in (self.frontend / "src", self.shared / "src"):
            path.mkdir(parents=True)
        self.manifest = self.frontend / "Cargo.toml"
        self.manifest.write_text('[package]\nname="fixture"\nversion="1.0.0"\n[dependencies]\nxcss-web={path="vendor/xcss-web"}\n')
        (self.frontend / "Cargo.lock").write_text('version=4\n[[package]]\nname="fixture"\nversion="1.0.0"\n')
        (self.frontend / "src/lib.rs").write_text("pub fn app() {}\n")
        (self.shared / "Cargo.toml").write_text('[package]\nname="xcss-web"\nversion="1.0.0"\n')
        (self.shared / "src/lib.rs").write_text("pub fn shared() {}\n")
        self.declare_shared()

    def declare_shared(self):
        files = {str(path.relative_to(self.shared)): hashlib.sha256(path.read_bytes()).hexdigest()
                 for path in self.shared.rglob("*") if path.is_file() and path.name != "SOURCE.json"}
        (self.shared / "SOURCE.json").write_text(json.dumps({"files": files}))

    def test_inventory_changes_when_browser_source_or_lock_changes(self):
        before = builder.source_inventory(self.manifest)
        self.assertEqual(before, builder.source_inventory(self.manifest))
        (self.frontend / "src/lib.rs").write_text("pub fn app() { /* changed */ }\n")
        after = builder.source_inventory(self.manifest)
        self.assertNotEqual(before["browser_inputs"], after["browser_inputs"])
        self.assertEqual(before["shared_inputs"], after["shared_inputs"])
        (self.frontend / "Cargo.lock").write_text('version=4\n[[package]]\nname="fixture"\nversion="1.0.1"\n')
        self.assertNotEqual(after["cargo_packages"], builder.source_inventory(self.manifest)["cargo_packages"])

    def test_modified_shared_source_requires_updated_inventory(self):
        (self.shared / "src/lib.rs").write_text("pub fn shared() { /* changed */ }\n")
        with self.assertRaisesRegex(SystemExit, "differs"):
            builder.source_inventory(self.manifest)
        self.declare_shared()
        builder.source_inventory(self.manifest)

    def test_added_shared_module_cannot_escape_inventory(self):
        (self.shared / "src/extra.rs").write_text("pub fn extra() {}\n")
        with self.assertRaisesRegex(SystemExit, "differs"):
            builder.source_inventory(self.manifest)


if __name__ == "__main__":
    unittest.main()
