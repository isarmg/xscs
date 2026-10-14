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
    def test_package_retains_original_repository_license(self):
        with tempfile.TemporaryDirectory() as temporary:
            staged=Path(temporary)
            module.stage_original_notices(staged)
            self.assertEqual((staged/"LICENSE").read_bytes(),(module.SOURCE/"LICENSE-APACHE").read_bytes())

    def test_mutable_ambiguous_or_foreign_installer_sources_fail_before_building(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);(root/"config").mkdir()
            pin=root/"config/xssi.lock"
            for invalid in ["git_rev=main\n","git_rev="+"0"*40+"\n",
                            "git_rev="+"a"*40+"\n\n",
                            "repository=https://example.test/xssi.git\ngit_rev="+"a"*40+"\n",
                            "git_rev="+"a"*40+"\ngit_rev="+"b"*40+"\n"]:
                pin.write_text(invalid)
                with self.assertRaises(ValueError):module.pinned_installer(root)
            pin.write_text("git_rev="+"a"*40+"\n")
            self.assertEqual(module.pinned_installer(root),("https://github.com/isarmg/xssi.git","a"*40))
    def test_public_fetch_preserves_transport_without_repository_tokens(self):
        inherited={"XSSI_REPOSITORY_TOKEN":"fixture-read-only-token",
                   "HTTPS_PROXY":"http://proxy.example.invalid",
                   "SSL_CERT_FILE":"/fixture/trust.pem"}
        with patch.dict(module.os.environ,inherited,clear=True):
            environment=module.installer_fetch_environment()
            self.assertNotIn("XSSI_REPOSITORY_TOKEN",environment)
            self.assertEqual(environment["HTTPS_PROXY"],inherited["HTTPS_PROXY"])
            self.assertEqual(environment["SSL_CERT_FILE"],inherited["SSL_CERT_FILE"])
            self.assertEqual(environment["GIT_TERMINAL_PROMPT"],"0")
            self.assertFalse(any("extraHeader" in value for value in environment.values()))

    def test_build_and_fetch_children_do_not_receive_signing_authority(self):
        secrets={"XSSI_SIGNING_KEY":"/private/fixture-signing.pem",
                 "XSSI_SIGNING_KEY_PEM":"fixture-private-pem",
                 "XSSI_PACKAGE_SIGNING_KEY":"fixture-package-pem",
                 "PACKAGE_SIGNING_KEY":"fixture-step-pem",
                 "RELEASE_SIGNING_KEY_PEM":"fixture-release-pem",
                 "XSSC_SIGNING_KEY_PEM":"fixture-upgrader-pem"}
        trust={"HTTPS_PROXY":"http://fixture-proxy.invalid",
               "SSL_CERT_FILE":"/fixture/trust.pem",
               "REQUESTS_CA_BUNDLE":"/fixture/requests.pem",
               "NODE_EXTRA_CA_CERTS":"/fixture/node.pem"}
        with patch.dict(module.os.environ,{**secrets,**trust},clear=True):
            environments=[module.build_environment(),module.installer_fetch_environment()]
            with patch.object(module.subprocess,"run") as child:
                child.return_value.stdout="fixture build output"
                module.run(["cargo","build"])
                environments.append(child.call_args.kwargs["env"])
            for environment in environments:
                self.assertTrue(set(secrets).isdisjoint(environment))
                self.assertEqual({name:environment[name] for name in trust},trust)

    def test_staged_server_rejects_replaced_build_bytes_or_a_foreign_receipt(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=Path(temporary);binary=root/"server";staged=root/"staged"
            reviewed=b"\x7fELF-reviewed-build";binary.write_bytes(reviewed)
            receipt={"format":"xssi-web-build-receipt-v1","binary_sha256":module.hashlib.sha256(reviewed).hexdigest(),
                     "identity":{"source_revision":"a"*40,"product":module.PRODUCT,"target":module.TARGET}}
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
            binary.write_bytes(reviewed);receipt["identity"]["target"]="foreign-platform"
            path.write_text(json.dumps(receipt))
            with self.assertRaisesRegex(ValueError,"receipt"):
                module.stage_verified_server(binary,staged,"a"*40)
            self.assertFalse(staged.exists())
            receipt["identity"]["target"]=module.TARGET
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
        self.assertNotIn('"verify-release"',text)
        self.assertNotIn('"release-identity"',text)

if __name__=="__main__":unittest.main()
