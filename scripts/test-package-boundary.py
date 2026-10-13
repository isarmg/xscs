#!/usr/bin/env python3
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SCRIPT=Path(__file__).with_name("package-release.py")
spec=importlib.util.spec_from_file_location("packager",SCRIPT)
module=importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)

class PackageBoundaryTests(unittest.TestCase):
    def test_mutable_or_foreign_installer_sources_fail_before_building(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary)
            for pin in [{"repository":"https://github.com/isarmg/xssi.git","revision":None},{"repository":"https://github.com/isarmg/xssi.git","revision":"main"},{"repository":"https://example.test/xssi.git","revision":"a"*40}]:
                (root/"installer-source.json").write_text(json.dumps(pin))
                with self.assertRaises(ValueError):module.pinned_installer(root)
            (root/"installer-source.json").write_text(json.dumps({"repository":"https://github.com/isarmg/xssi.git","revision":"a"*40}))
            self.assertEqual(module.pinned_installer(root),("https://github.com/isarmg/xssi.git","a"*40))
    def test_private_fetch_credentials_are_scoped_and_removed_from_builds(self):
        inherited={"XSSI_REPOSITORY_TOKEN":"fixture-read-only-token","GIT_CONFIG_COUNT":"1","GIT_CONFIG_KEY_0":"http.proxy","GIT_CONFIG_VALUE_0":"http://proxy.example.invalid"}
        with patch.dict(module.os.environ,inherited,clear=True):
            self.assertNotIn("XSSI_REPOSITORY_TOKEN",module.build_environment())
            environment=module.installer_fetch_environment("fixture-read-only-token")
            self.assertNotIn("XSSI_REPOSITORY_TOKEN",environment)
            self.assertEqual(environment["GIT_CONFIG_COUNT"],"2")
            self.assertEqual(environment["GIT_CONFIG_KEY_0"],"http.proxy")
            self.assertEqual(environment["GIT_CONFIG_VALUE_0"],inherited["GIT_CONFIG_VALUE_0"])
            self.assertEqual(environment["GIT_CONFIG_KEY_1"],"http.https://github.com/isarmg/xssi.git.extraHeader")
            self.assertNotIn("fixture-read-only-token",environment["GIT_CONFIG_VALUE_1"])
            for unsafe in ["credential with spaces","credential\nheader"]:
                with self.assertRaises(ValueError):module.installer_fetch_environment(unsafe)

    def test_staged_server_rejects_replaced_build_bytes_or_a_foreign_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);binary=root/"server";staged=root/"staged"
            reviewed=b"\x7fELF-reviewed-build";binary.write_bytes(reviewed)
            receipt={"format":"xssi-web-build-receipt-v1","binary_sha256":module.hashlib.sha256(reviewed).hexdigest(),
                     "identity":{"source_revision":"a"*40,"product":module.PRODUCT}}
            path=Path(str(binary)+".xssi-web-build.json");path.write_text(json.dumps(receipt))
            module.stage_verified_server(binary,staged,"a"*40)
            self.assertEqual(staged.read_bytes(),reviewed)
            with self.assertRaises(FileExistsError):
                module.stage_verified_server(binary,staged,"a"*40)
            self.assertEqual(staged.read_bytes(),reviewed)
            staged.unlink();binary.write_bytes(b"\x7fELF-replaced-build")
            with self.assertRaisesRegex(ValueError,"staged server"):
                module.stage_verified_server(binary,staged,"a"*40)
            self.assertFalse(staged.exists())
            binary.write_bytes(reviewed);receipt["identity"]["product"]="foreign-server"
            path.write_text(json.dumps(receipt))
            with self.assertRaisesRegex(ValueError,"receipt"):
                module.stage_verified_server(binary,staged,"a"*40)
            self.assertFalse(staged.exists())

    def test_installer_remains_external_to_the_runtime_repository(self):
        root=SCRIPT.parents[1]
        self.assertFalse((root/"install.sh").exists())
        self.assertFalse((root/"crates/server/src/release_bundle.rs").exists())
        text=SCRIPT.read_text()
        self.assertIn("scripts/build-package.sh",text)
        self.assertIn('"--public-key"',text)
        self.assertNotIn('"verify-release"',text)
        self.assertNotIn('"release-identity"',text)

if __name__=="__main__":unittest.main()
