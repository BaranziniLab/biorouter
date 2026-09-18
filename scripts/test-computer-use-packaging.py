#!/usr/bin/env python3
"""Mutation tests for helper integrity/target guards and installer dependencies."""
import importlib.util
import json
from pathlib import Path
import struct
import subprocess
import tempfile
import unittest
import zipfile

spec = importlib.util.spec_from_file_location("runtime", Path(__file__).with_name("computer-use-runtime.py"))
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)

protocol_spec = importlib.util.spec_from_file_location("protocol", Path(__file__).with_name("test-computer-use-protocol.py"))
protocol = importlib.util.module_from_spec(protocol_spec)
protocol_spec.loader.exec_module(protocol)


class NativeContractTests(unittest.TestCase):
    def test_reviewed_schema_accepts_only_documentation_differences(self):
        tools = json.loads(protocol.CONTRACT.read_text())["tools"]
        tools[0]["inputSchema"]["description"] = "Platform-specific documentation"
        self.assertEqual(protocol.validate_tools(tools), protocol.TOOLS)

    def test_native_bounds_drift_is_rejected_before_desktop_actions(self):
        for name, parameter, keyword in [("click", "click_count", "maximum"),
                                         ("click", "click_count", "minimum"),
                                         ("scroll", "pages", "maximum"),
                                         ("scroll", "pages", "exclusiveMinimum")]:
            with self.subTest(name=name, keyword=keyword):
                tools = json.loads(protocol.CONTRACT.read_text())["tools"]
                tool = next(item for item in tools if item["name"] == name)
                del tool["inputSchema"]["properties"][parameter][keyword]
                with self.assertRaisesRegex(ValueError, "Native schema mismatch"):
                    protocol.validate_tools(tools)

    def test_boolean_is_not_a_numeric_schema_bound(self):
        for name, parameter, keyword, boolean in [("click", "click_count", "minimum", True),
                                                  ("scroll", "pages", "exclusiveMinimum", False)]:
            with self.subTest(name=name):
                tools = json.loads(protocol.CONTRACT.read_text())["tools"]
                tool = next(item for item in tools if item["name"] == name)
                tool["inputSchema"]["properties"][parameter][keyword] = boolean
                with self.assertRaisesRegex(ValueError, "Native schema mismatch"):
                    protocol.validate_tools(tools)


class PayloadTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.path = Path(self.temp.name)
        header = bytearray(64)
        header[:6] = b"\x7fELF\x02\x01"
        struct.pack_into("<H", header, 18, 62)
        (self.path / "ocu").write_bytes(header)
        (self.path / "ocu").chmod(0o755)
        runtime.write_manifest(self.path, "linux-x64")

    def tearDown(self):
        self.temp.cleanup()

    def test_valid_payload(self):
        runtime.verify(self.path, "linux-x64")

    def test_modified_helper(self):
        with (self.path / "ocu").open("ab") as file:
            file.write(b"changed")
        with self.assertRaisesRegex(ValueError, "modified"):
            runtime.verify(self.path, "linux-x64")

    def test_foreign_target(self):
        with self.assertRaisesRegex(ValueError, "foreign"):
            runtime.verify(self.path, "win32-x64")

    def test_missing_helper(self):
        (self.path / "ocu").unlink()
        with self.assertRaisesRegex(ValueError, "modified"):
            runtime.verify(self.path, "linux-x64")

    def test_extra_file(self):
        (self.path / "ocu.exe").write_bytes(b"foreign")
        with self.assertRaisesRegex(ValueError, "modified"):
            runtime.verify(self.path, "linux-x64")

    def test_symlink_rejected(self):
        try:
            (self.path / "link").symlink_to("ocu")
        except OSError as error:
            self.skipTest(f"Host cannot create test symlinks: {error}")
        with self.assertRaisesRegex(ValueError, "symlink"):
            runtime.verify(self.path, "linux-x64")

    def test_claimed_target_cannot_mask_binary_architecture(self):
        (self.path / "ocu").write_bytes(b"MZ" + bytes(100))
        runtime.write_manifest(self.path, "linux-x64")
        with self.assertRaisesRegex(ValueError, "architecture"):
            runtime.verify(self.path, "linux-x64")

    def test_stale_pin(self):
        manifest = json.loads((self.path / "manifest.json").read_text())
        manifest["upstream_commit"] = "0" * 40
        (self.path / "manifest.json").write_text(json.dumps(manifest))
        with self.assertRaisesRegex(ValueError, "pin"):
            runtime.verify(self.path, "linux-x64")

    def test_node_staging_guard_matches_python_manifest(self):
        script = runtime.ROOT / "ui/desktop/scripts/computer-use-resources.js"
        result = subprocess.run(["node", "-e", "require(process.argv[1]).verifyComputerUse(process.argv[2], 'linux-x64')",
                                 str(script), str(self.path)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)

    def test_final_archive_checks_embedded_bytes(self):
        spec = importlib.util.spec_from_file_location("archive", Path(__file__).with_name("verify-computer-use-artifact.py"))
        archive_module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(archive_module)
        with tempfile.TemporaryDirectory(prefix="BioRouter archive spaces ") as directory:
            artifact = Path(directory) / "Biorouter test.zip"
            with zipfile.ZipFile(artifact, "w") as archive:
                for file in self.path.iterdir():
                    archive.write(file, "Biorouter/resources/computer-use/" + file.name)
            evidence = archive_module.verify_artifact(artifact, "linux-x64")
            self.assertEqual(evidence["manifest_sha256"], runtime.digest(self.path / "manifest.json"))
            with zipfile.ZipFile(artifact, "a") as archive:
                archive.writestr("Biorouter/resources/computer-use/foreign.exe", b"MZ")
            with self.assertRaisesRegex(ValueError, "modified"):
                archive_module.verify_artifact(artifact, "linux-x64")

    def test_dependency_contract_in_both_package_formats(self):
        cli = (runtime.ROOT / "packaging/cli/nfpm.yaml").read_text()
        desktop = (runtime.ROOT / "ui/desktop/forge.config.ts").read_text()
        for packages in runtime.PIN["linux_dependencies"].values():
            for package in packages:
                self.assertIn("- " + package + "\n", cli)
                self.assertIn("'" + package + "'", desktop)


if __name__ == "__main__":
    unittest.main()
