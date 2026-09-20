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

acceptance_spec = importlib.util.spec_from_file_location("acceptance", Path(__file__).with_name("computer-use-package-acceptance.py"))
acceptance = importlib.util.module_from_spec(acceptance_spec)
acceptance_spec.loader.exec_module(acceptance)


def stage_helper(root, *relative):
    """Create <root>/<relative>/computer-use/manifest.json and return its resources dir."""
    helper = Path(root).joinpath(*relative) / "computer-use"
    helper.mkdir(parents=True)
    (helper / "manifest.json").write_text("{}")
    return helper.parent


class PackagedResourceLocationTests(unittest.TestCase):
    """The Linux GUI packages install under usr/lib/<name>, never /opt.

    electron-installer-debian lowercases <name> and electron-installer-redhat
    preserves its case, and `prefix: '/opt'` in forge.config.ts is inert, so the
    tree is located by its unique helper manifest instead of by install prefix.
    """

    def test_locates_the_real_deb_and_rpm_layouts(self):
        for name in ["biorouter", "Biorouter"]:
            with self.subTest(name=name), tempfile.TemporaryDirectory() as temp:
                expected = stage_helper(temp, "usr/lib", name, "resources")
                self.assertEqual(acceptance.packaged_desktop_resources(Path(temp)), expected)

    def test_uniqueness_and_provenance_still_fail_closed(self):
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaisesRegex(ValueError, "exactly one helper"):
                acceptance.packaged_desktop_resources(Path(temp))
        with tempfile.TemporaryDirectory() as temp:
            stage_helper(temp, "usr/lib/biorouter/resources")
            stage_helper(temp, "opt/Biorouter/resources")
            with self.assertRaisesRegex(ValueError, "exactly one helper"):
                acceptance.packaged_desktop_resources(Path(temp))
        with tempfile.TemporaryDirectory() as temp:
            stage_helper(temp, "usr/lib/biorouter/elsewhere")
            with self.assertRaisesRegex(ValueError, "not inside a desktop resources directory"):
                acceptance.packaged_desktop_resources(Path(temp))

    def test_installed_lookup_finds_gui_and_cli_installs_and_rejects_both_at_once(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            lib = root / "usr/lib"
            stage_helper(lib, "biorouter/resources")
            found = acceptance.installed_linux_helper_roots(opt=root / "opt", lib=lib,
                                                            libexec=root / "usr/libexec")
            self.assertEqual(found, [lib / "biorouter/resources/computer-use/manifest.json"])
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            libexec = root / "usr/libexec"
            stage_helper(libexec, "biorouter")
            found = acceptance.installed_linux_helper_roots(opt=root / "opt", lib=root / "usr/lib",
                                                            libexec=libexec)
            self.assertEqual(found, [libexec / "biorouter/computer-use/manifest.json"])
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            lib, libexec = root / "usr/lib", root / "usr/libexec"
            stage_helper(lib, "biorouter/resources")
            stage_helper(libexec, "biorouter")
            self.assertEqual(len(acceptance.installed_linux_helper_roots(
                opt=root / "opt", lib=lib, libexec=libexec)), 2)

    def test_linux_paths_resolves_the_cli_beside_each_kind_of_install(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            lib, libexec, opt = root / 'usr/lib', root / 'usr/libexec', root / 'opt'
            # GUI package: helper inside the app's resources, CLI beside it.
            stage_helper(lib, 'biorouter/resources')
            cli, helper = acceptance.linux_paths(lib=lib, libexec=libexec, opt=opt)
            self.assertEqual(helper, lib / 'biorouter/resources/computer-use')
            self.assertEqual(cli, lib / 'biorouter/resources/bin/biorouter')
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            lib, libexec, opt = root / 'usr/lib', root / 'usr/libexec', root / 'opt'
            # CLI package: FHS layout, binary on PATH.
            stage_helper(libexec, 'biorouter')
            cli, helper = acceptance.linux_paths(lib=lib, libexec=libexec, opt=opt)
            self.assertEqual(helper, libexec / 'biorouter/computer-use')
            self.assertEqual(cli, Path('/usr/bin/biorouter'))
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            lib, libexec, opt = root / 'usr/lib', root / 'usr/libexec', root / 'opt'
            # Both installed at once is ambiguous and must refuse, not guess.
            stage_helper(lib, 'biorouter/resources')
            stage_helper(libexec, 'biorouter')
            with self.assertRaisesRegex(ValueError, 'Expected one installed helper'):
                acceptance.linux_paths(lib=lib, libexec=libexec, opt=opt)

    def test_rpmbuild_cannot_rewrite_the_helper_after_the_build(self):
        """rpm 4.18's brp-strip-comment-note targets ALREADY-stripped ELF -- which a
        `-ldflags=-s -w` Go binary is -- and repacks it, shrinking the helper by
        2,474 bytes and breaking the recorded payload hashes. The deb is unaffected
        because dpkg does not post-process. electron-installer-redhat offers no
        option for this (fixed rpmbuild argv, hardcoded spec template), so the lever
        is a build-scoped $HOME holding .rpmmacros."""
        desktop = (runtime.ROOT / "ui/desktop/forge.config.ts").read_text()
        self.assertIn("%__os_install_post %{nil}", desktop,
                      "rpmbuild would strip the helper and invalidate its provenance")
        for hook in ["preMake", "postMake"]:
            self.assertIn(hook, desktop, f"the macros are applied and released in {hook}")
        code = "\n".join(line for line in desktop.splitlines() if not line.strip().startswith("//"))
        self.assertNotIn("fpm:", code,
                         "electron-installer-redhat never reads fpm options; it invokes "
                         "rpmbuild with a fixed argv, so the entry was dead configuration")

    def test_no_maker_declares_an_inert_install_prefix(self):
        # Comments are stripped first: the explanatory note deliberately quotes the
        # option it is warning about, and matching that would be a check that can
        # never pass rather than one that can never fail.
        desktop = (runtime.ROOT / "ui/desktop/forge.config.ts").read_text()
        code = "\n".join(line for line in desktop.splitlines() if not line.strip().startswith("//"))
        self.assertNotIn("prefix:", code,
                         "electron-installer-{debian,redhat} have no prefix option; "
                         "declaring one re-seeds the /opt belief this test exists to kill")


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
        cli = (runtime.ROOT / "packaging/biorouter-cli.yaml").read_text()
        desktop = (runtime.ROOT / "ui/desktop/forge.config.ts").read_text()
        for packages in runtime.PIN["linux_dependencies"].values():
            for package in packages:
                self.assertIn("- " + package + "\n", cli)
                self.assertIn("'" + package + "'", desktop)


if __name__ == "__main__":
    unittest.main()
