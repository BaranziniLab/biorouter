#!/usr/bin/env python3
"""Packaging tests for Biorouter Copilot's engine payload (agent-desktop).

Mutation tests for the payload integrity, target and identity checks shared by
scripts/computer-use-runtime.py and ui/desktop/scripts/computer-use-resources.js,
the binary inspection behind them (Mach-O minimum macOS, ELF glibc floor and
shared libraries, PE subsystem and imports), the cargo command line, the
container recipe, the vendored-tree guard, the release attestation and the Linux
package dependencies. None of them builds anything; a payload built on this host
is exercised too when one exists (`computer-use-runtime.py build darwin-<arch>`).
"""
import importlib.util
import json
import os
from pathlib import Path
import platform
import shutil
import struct
import subprocess
import sys
import tempfile
import unittest
from unittest import mock
import zipfile

HERE = Path(__file__).resolve().parent


def load(name, file):
    spec = importlib.util.spec_from_file_location(name, HERE / file)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


runtime = load("runtime", "computer-use-runtime.py")
vendor_source = runtime.vendor_source
acceptance = load("acceptance", "computer-use-package-acceptance.py")
archive_module = load("archive", "verify-computer-use-artifact.py")
NODE = shutil.which("node")


# ── synthetic binaries ──────────────────────────────────────────────────────

def elf(glibc=(), needed=(), machine=62):
    """A 64-bit ELF with a .gnu.version_r naming `glibc` versions and DT_NEEDED `needed`."""
    strings = bytearray(b"\0")

    def string(text):
        offset = len(strings)
        strings.extend(text.encode() + b"\0")
        return offset

    needed_offsets = [string(name) for name in needed]
    libc = string("libc.so.6")
    version_offsets = [string("GLIBC_" + version) for version in glibc]
    verneed = bytearray()
    if glibc:
        verneed += struct.pack("<HHIII", 1, len(glibc), libc, 16, 0)
        for index, offset in enumerate(version_offsets):
            following = 16 if index + 1 < len(glibc) else 0
            verneed += struct.pack("<IHHII", 0, 0, index + 2, offset, following)
    dynamic = b"".join(struct.pack("<qQ", 1, offset) for offset in needed_offsets) + struct.pack("<qQ", 0, 0)
    header_size = 64
    strings_at = header_size
    verneed_at = strings_at + len(strings)
    dynamic_at = verneed_at + len(verneed)
    sections_at = dynamic_at + len(dynamic)
    sections = [struct.pack("<IIQQQQIIQQ", 0, 0, 0, 0, 0, 0, 0, 0, 0, 0),
                struct.pack("<IIQQQQIIQQ", 0, 3, 0, 0, strings_at, len(strings), 0, 0, 1, 0),
                struct.pack("<IIQQQQIIQQ", 0, 0x6FFFFFFE, 0, 0, verneed_at, len(verneed), 1,
                            1 if glibc else 0, 8, 0),
                struct.pack("<IIQQQQIIQQ", 0, 6, 0, 0, dynamic_at, len(dynamic), 1, 0, 8, 16)]
    header = bytearray(header_size)
    header[:6] = b"\x7fELF\x02\x01"
    struct.pack_into("<H", header, 18, machine)
    struct.pack_into("<Q", header, 0x28, sections_at)
    struct.pack_into("<HHH", header, 0x3A, 64, len(sections), 1)
    return bytes(header + strings + verneed + dynamic + b"".join(sections))


def pe(subsystem=3, dlls=("KERNEL32.dll",)):
    """A PE32+ x64 image with one section holding an import table for `dlls`."""
    data = bytearray(0x400)
    data[:2] = b"MZ"
    struct.pack_into("<I", data, 0x3C, 0x40)
    data[0x40:0x44] = b"PE\0\0"
    struct.pack_into("<HH", data, 0x44, 0x8664, 1)
    struct.pack_into("<H", data, 0x44 + 16, 240)
    optional = 0x58
    struct.pack_into("<H", data, optional, 0x20B)
    struct.pack_into("<H", data, optional + 68, subsystem)
    struct.pack_into("<I", data, optional + 108, 16)
    section_rva, section_raw = 0x1000, 0x200
    descriptors = 20 * (len(dlls) + 1)
    names, cursor = [], descriptors
    for dll in dlls:
        names.append((cursor, dll))
        cursor += len(dll) + 1
    struct.pack_into("<II", data, optional + 112 + 8, section_rva, descriptors)
    table = optional + 240
    data[table:table + 8] = b".idata\0\0"
    struct.pack_into("<IIII", data, table + 8, 0x200, section_rva, 0x200, section_raw)
    for index, (offset, dll) in enumerate(names):
        struct.pack_into("<IIIII", data, section_raw + index * 20, 0, 0, 0, section_rva + offset, 0)
        data[section_raw + offset:section_raw + offset + len(dll)] = dll.encode()
    return bytes(data)


def macho(minos="13.0", cpu=0x0100000C):
    """A thin 64-bit Mach-O header with one LC_BUILD_VERSION for macOS `minos`."""
    major, minor = (int(part) for part in minos.split("."))
    command = struct.pack("<IIIIII", 0x32, 24, 1, (major << 16) | (minor << 8), 0, 0)
    header = struct.pack("<IIIIIIII", 0xFEEDFACF, cpu, 0, 2, 1, len(command), 0, 0)
    return header + command


def stage_payload(directory, target, binary):
    directory = Path(directory)
    path = directory / runtime.executable(target)
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_bytes(binary)
    path.chmod(0o755)
    shutil.copyfile(runtime.VENDOR / "LICENSE", directory / runtime.LICENSE_FILE)
    shutil.copyfile(runtime.VENDOR / "DEPENDENCIES.txt", directory / runtime.DEPENDENCIES_FILE)
    runtime.write_manifest(directory, target)


def stage_helper(root, *relative):
    """Create <root>/<relative>/computer-use/manifest.json and return its resources dir."""
    helper = Path(root).joinpath(*relative) / "computer-use"
    helper.mkdir(parents=True)
    (helper / "manifest.json").write_text("{}")
    return helper.parent


# ── binary inspection ───────────────────────────────────────────────────────

class BinaryInspectionTests(unittest.TestCase):
    def test_macho_minimum_is_read_from_lc_build_version(self):
        self.assertEqual(runtime.macho_minimum_macos(macho("13.0")), "13.0")
        self.assertEqual(runtime.macho_minimum_macos(macho("12.0")), "12.0")
        self.assertIsNone(runtime.macho_minimum_macos(b"\x7fELF"))

    def test_elf_glibc_versions_and_needed_libraries(self):
        data = elf(glibc=("2.2.5", "2.30", "2.28"), needed=("libc.so.6", "libgcc_s.so.1"))
        self.assertEqual(runtime.elf_glibc_versions(data), {"2.2.5", "2.30", "2.28"})
        self.assertEqual(runtime.elf_needed_libraries(data), ["libc.so.6", "libgcc_s.so.1"])
        self.assertEqual(runtime.elf_glibc_versions(elf()), set())

    def test_pe_subsystem_and_imports(self):
        subsystem, dlls = runtime.pe_details(pe(3, ("KERNEL32.dll", "USER32.dll")))
        self.assertEqual((subsystem, dlls), (3, ["KERNEL32.dll", "USER32.dll"]))

    def test_linux_floor_is_the_pinned_glibc(self):
        with tempfile.TemporaryDirectory() as temp:
            binary = Path(temp) / "agent-desktop"
            binary.write_bytes(elf(glibc=("2.17", runtime.PIN["glibc_floor"]), needed=("libc.so.6", "libm.so.6")))
            runtime.check_binary(binary, "linux-x64")
            binary.write_bytes(elf(glibc=("2.34",), needed=("libc.so.6",)))
            with self.assertRaisesRegex(ValueError, "above the 2.31 floor"):
                runtime.check_binary(binary, "linux-x64")

    def test_linux_helper_may_link_glibc_alone(self):
        with tempfile.TemporaryDirectory() as temp:
            binary = Path(temp) / "agent-desktop"
            binary.write_bytes(elf(needed=("libc.so.6", "libdbus-1.so.3")))
            with self.assertRaisesRegex(ValueError, r"outside glibc \['libdbus-1.so.3'\]"):
                runtime.check_binary(binary, "linux-x64")

    def test_linux_arm64_machine_is_checked(self):
        with tempfile.TemporaryDirectory() as temp:
            binary = Path(temp) / "agent-desktop"
            binary.write_bytes(elf(machine=183))
            runtime.check_binary(binary, "linux-arm64")
            with self.assertRaisesRegex(ValueError, "architecture"):
                runtime.check_binary(binary, "linux-x64")

    def test_windows_helper_is_a_console_program_without_mingw_runtime_dlls(self):
        with tempfile.TemporaryDirectory() as temp:
            binary = Path(temp) / "agent-desktop.exe"
            binary.write_bytes(pe(3, ("KERNEL32.dll", "ntdll.dll", "api-ms-win-core-synch-l1-2-0.dll")))
            runtime.check_binary(binary, "win32-x64")
            binary.write_bytes(pe(2))
            with self.assertRaisesRegex(ValueError, "not a console executable"):
                runtime.check_binary(binary, "win32-x64")
            binary.write_bytes(pe(3, ("KERNEL32.dll", "libwinpthread-1.dll")))
            with self.assertRaisesRegex(ValueError, "libwinpthread-1.dll"):
                runtime.check_binary(binary, "win32-x64")
            # A runtime DLL the payload itself carries is acceptable.
            runtime.check_binary(binary, "win32-x64", {"libwinpthread-1.dll"})

    def test_macos_minimum_must_be_the_pin(self):
        with tempfile.TemporaryDirectory() as temp:
            binary = Path(temp) / "agent-desktop"
            binary.write_bytes(macho(runtime.PIN["minimum_macos"]))
            runtime.check_binary(binary, "darwin-arm64")
            binary.write_bytes(macho("12.0"))
            with self.assertRaisesRegex(ValueError, "declares macOS 12.0"):
                runtime.check_binary(binary, "darwin-arm64")
            binary.write_bytes(macho(runtime.PIN["minimum_macos"], cpu=0x01000007))
            with self.assertRaisesRegex(ValueError, "architecture"):
                runtime.check_binary(binary, "darwin-arm64")
            runtime.check_binary(binary, "darwin-x64")


# ── payload manifest and verification ──────────────────────────────────────

class PayloadTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="Biorouter payload spaces ")
        self.path = Path(self.temp.name)
        stage_payload(self.path, "linux-x64", elf(glibc=("2.30",), needed=("libc.so.6",)))
        self.helper = self.path / "agent-desktop"

    def tearDown(self):
        self.temp.cleanup()

    def rewrite(self, **changes):
        manifest = json.loads((self.path / "manifest.json").read_text())
        manifest.update(changes)
        (self.path / "manifest.json").write_text(json.dumps(manifest))

    def test_valid_payload(self):
        manifest = runtime.verify(self.path, "linux-x64")
        self.assertEqual(manifest["engine"], "agent-desktop")
        self.assertEqual(manifest["upstream_version"], runtime.PIN["upstream_version"])
        self.assertEqual(manifest["minimum_os"], {"glibc": runtime.PIN["glibc_floor"]})
        self.assertNotIn("bundle_identifier", manifest)

    def test_modified_helper(self):
        with self.helper.open("ab") as file:
            file.write(b"changed")
        with self.assertRaisesRegex(ValueError, "modified"):
            runtime.verify(self.path, "linux-x64")

    def test_foreign_target(self):
        with self.assertRaisesRegex(ValueError, "foreign"):
            runtime.verify(self.path, "linux-arm64")

    def test_missing_helper(self):
        self.helper.unlink()
        with self.assertRaisesRegex(ValueError, "modified"):
            runtime.verify(self.path, "linux-x64")

    def test_extra_file(self):
        (self.path / "agent-desktop.exe").write_bytes(b"foreign")
        with self.assertRaisesRegex(ValueError, "modified"):
            runtime.verify(self.path, "linux-x64")

    def test_symlink_rejected(self):
        try:
            (self.path / "link").symlink_to("agent-desktop")
        except OSError as error:
            self.skipTest(f"Host cannot create test symlinks: {error}")
        with self.assertRaisesRegex(ValueError, "symlink"):
            runtime.verify(self.path, "linux-x64")

    def test_claimed_target_cannot_mask_binary_architecture(self):
        self.helper.write_bytes(b"MZ" + bytes(100))
        runtime.write_manifest(self.path, "linux-x64")
        with self.assertRaisesRegex(ValueError, "architecture"):
            runtime.verify(self.path, "linux-x64")

    def test_stale_pin(self):
        self.rewrite(upstream_commit="0" * 40)
        with self.assertRaisesRegex(ValueError, "pin"):
            runtime.verify(self.path, "linux-x64")

    def test_another_contract_version_is_refused(self):
        self.rewrite(contract_version="2.7")
        with self.assertRaisesRegex(ValueError, "contract_version does not match"):
            runtime.verify(self.path, "linux-x64")

    def test_a_payload_of_the_retired_engine_is_refused(self):
        self.rewrite(schema_version=1, patch_revision="biorouter-3")
        with self.assertRaisesRegex(ValueError, "predates the agent-desktop engine"):
            runtime.verify(self.path, "linux-x64")

    def test_a_helper_built_for_another_biorouter_version_is_refused(self):
        self.rewrite(biorouter_version="0.0.1")
        with self.assertRaisesRegex(ValueError, "built for Biorouter 0.0.1"):
            runtime.verify(self.path, "linux-x64")

    def test_licence_and_dependency_list_are_required(self):
        (self.path / runtime.LICENSE_FILE).write_text("MIT, honestly\n")
        runtime.write_manifest(self.path, "linux-x64")
        with self.assertRaisesRegex(ValueError, "Apache License"):
            runtime.verify(self.path, "linux-x64")
        shutil.copyfile(runtime.VENDOR / "LICENSE", self.path / runtime.LICENSE_FILE)
        (self.path / runtime.DEPENDENCIES_FILE).unlink()
        runtime.write_manifest(self.path, "linux-x64")
        with self.assertRaisesRegex(ValueError, "no DEPENDENCIES.agent-desktop.txt"):
            runtime.verify(self.path, "linux-x64")

    def test_only_macos_payloads_carry_a_bundle_identifier(self):
        self.rewrite(bundle_identifier=runtime.PIN["bundle_identifier"])
        with self.assertRaisesRegex(ValueError, "Only a macOS helper payload"):
            runtime.verify(self.path, "linux-x64")

    def test_glibc_floor_is_enforced_on_the_payload(self):
        self.helper.write_bytes(elf(glibc=("2.35",), needed=("libc.so.6",)))
        runtime.write_manifest(self.path, "linux-x64")
        with self.assertRaisesRegex(ValueError, "glibc 2.35"):
            runtime.verify(self.path, "linux-x64")

    @unittest.skipUnless(NODE, "node is not on PATH")
    def test_node_staging_guard_matches_python_manifest(self):
        script = runtime.ROOT / "ui/desktop/scripts/computer-use-resources.js"
        check = "require(process.argv[1]).verifyComputerUse(process.argv[2], process.argv[3])"
        result = subprocess.run([NODE, "-e", check, str(script), str(self.path), "linux-x64"],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode, 0, result.stderr)
        for mutate, expected in [
            (lambda: self.rewrite(target="linux-arm64"), "foreign"),
            (lambda: self.rewrite(schema_version=1), "predates the agent-desktop engine"),
            (lambda: self.rewrite(biorouter_version="0.0.1"), "was built for 0.0.1"),
            (lambda: (self.path / "stray.txt").write_text("x"), "files changed or are missing"),
        ]:
            with self.subTest(expected=expected):
                stage_payload(self.path, "linux-x64", elf(glibc=("2.30",), needed=("libc.so.6",)))
                for stray in self.path.glob("stray.txt"):
                    stray.unlink()
                runtime.write_manifest(self.path, "linux-x64")
                mutate()
                result = subprocess.run([NODE, "-e", check, str(script), str(self.path), "linux-x64"],
                                        capture_output=True, text=True)
                self.assertNotEqual(result.returncode, 0)
                self.assertIn(expected, result.stderr)

    def test_final_archive_checks_embedded_bytes(self):
        with tempfile.TemporaryDirectory(prefix="Biorouter archive spaces ") as directory:
            artifact = Path(directory) / "Biorouter test.zip"
            with zipfile.ZipFile(artifact, "w") as archive:
                for file in sorted(self.path.rglob("*")):
                    if file.is_file():
                        info = zipfile.ZipInfo("Biorouter/resources/computer-use/" + file.relative_to(self.path).as_posix())
                        info.external_attr = (file.stat().st_mode & 0o777) << 16
                        archive.writestr(info, file.read_bytes())
            evidence = archive_module.verify_artifact(artifact, "linux-x64")
            self.assertEqual(evidence["manifest_sha256"], runtime.digest(self.path / "manifest.json"))
            self.assertEqual(evidence["executable_sha256"], runtime.digest(self.helper))
            self.assertEqual((evidence["engine"], evidence["upstream_version"]),
                             ("agent-desktop", runtime.PIN["upstream_version"]))
            with zipfile.ZipFile(artifact, "a") as archive:
                archive.writestr("Biorouter/resources/computer-use/foreign.exe", b"MZ")
            with self.assertRaisesRegex(ValueError, "modified"):
                archive_module.verify_artifact(artifact, "linux-x64")


class WindowsPayloadTests(unittest.TestCase):
    def test_windows_payload_round_trip(self):
        with tempfile.TemporaryDirectory() as temp:
            stage_payload(temp, "win32-x64", pe(3, ("KERNEL32.dll", "bcryptprimitives.dll")))
            manifest = runtime.verify(temp, "win32-x64")
            self.assertEqual(manifest["executable"], "agent-desktop.exe")
            self.assertEqual(manifest["triple"], "x86_64-pc-windows-gnu")


@unittest.skipUnless(platform.system() == "Darwin", "codesign and the built bundle need macOS")
class BuiltMacPayloadTests(unittest.TestCase):
    """Run against payloads this host built, if any; a fresh checkout has none."""

    def payloads(self):
        found = [target for target in ("darwin-arm64", "darwin-x64")
                 if (runtime.OUTPUT / target / "manifest.json").exists()]
        if not found:
            self.skipTest("no darwin payload built on this host")
        return found

    def test_built_bundle_carries_the_copilot_identity(self):
        for target in self.payloads():
            with self.subTest(target=target):
                manifest = runtime.verify(runtime.OUTPUT / target, target)
                self.assertEqual(manifest["bundle_identifier"], "org.biorouter.computer-use")
                info = runtime.plistlib.loads(
                    (runtime.OUTPUT / target / runtime.PIN["macos_bundle"] / "Contents/Info.plist").read_bytes())
                self.assertEqual(info["CFBundleExecutable"], "agent-desktop")
                self.assertEqual(info["CFBundleDisplayName"], "Biorouter Copilot")
                self.assertEqual(info["LSMinimumSystemVersion"], "13.0")
                self.assertIs(info["LSUIElement"], True)

    @unittest.skipUnless(NODE, "node is not on PATH")
    def test_node_guard_accepts_the_built_bundle(self):
        script = runtime.ROOT / "ui/desktop/scripts/computer-use-resources.js"
        for target in self.payloads():
            with self.subTest(target=target):
                result = subprocess.run([NODE, "-e", "require(process.argv[1]).verifyComputerUse(process.argv[2], process.argv[3])",
                                         str(script), str(runtime.OUTPUT / target), target], capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)

    def test_a_resealed_bundle_is_refused(self):
        target = self.payloads()[0]
        with tempfile.TemporaryDirectory() as temp:
            copy = Path(temp) / target
            shutil.copytree(runtime.OUTPUT / target, copy)
            plist = copy / runtime.PIN["macos_bundle"] / "Contents/Info.plist"
            info = runtime.plistlib.loads(plist.read_bytes())
            info["CFBundleIdentifier"] = "org.example.not-copilot"
            plist.write_bytes(runtime.plistlib.dumps(info))
            runtime.write_manifest(copy, target)
            with self.assertRaisesRegex(ValueError, "Info.plist"):
                runtime.verify(copy, target)


# ── the build command line and its container ───────────────────────────────

class BuildCommandTests(unittest.TestCase):
    def command(self, target, **env):
        with mock.patch.dict(os.environ, env, clear=False):
            return runtime.cargo_build_command(target)

    def test_locked_pinned_and_built_from_the_vendored_tree(self):
        command, env, built = self.command("linux-x64", RUSTFLAGS="-C target-cpu=native")
        command = [str(part) for part in command]
        for flag in ("build", "--release", "--locked", "-p", "agent-desktop", "--target", "x86_64-unknown-linux-gnu"):
            self.assertIn(flag, command)
        self.assertEqual(command[command.index("--manifest-path") + 1],
                         str(runtime.ROOT / "vendor/agent-desktop/source/Cargo.toml"))
        self.assertEqual(Path(command[command.index("--target-dir") + 1]), runtime.BUILD / "linux-x64")
        self.assertEqual(env["RUSTUP_TOOLCHAIN"], "1.92")
        self.assertNotIn("target-cpu=native", env["RUSTFLAGS"], "a developer's own flags reached the helper")
        self.assertIn(f"--remap-path-prefix={vendor_source.SOURCE.as_posix()}=/agent-desktop", env["RUSTFLAGS"])
        self.assertNotIn("MACOSX_DEPLOYMENT_TARGET", env)
        self.assertEqual(built, runtime.BUILD / "linux-x64/x86_64-unknown-linux-gnu/release/agent-desktop")

    def test_macos_build_declares_the_pinned_minimum(self):
        _command, env, built = self.command("darwin-x64", MACOSX_DEPLOYMENT_TARGET="12.0")
        self.assertEqual(env["MACOSX_DEPLOYMENT_TARGET"], runtime.PIN["minimum_macos"])
        self.assertEqual(env["CCC_OVERRIDE_OPTIONS"], "# +-Wno-error=deprecated-declarations")
        self.assertEqual(built.parent.parent.name, "x86_64-apple-darwin")

    def test_windows_build_is_the_gnu_target(self):
        command, _env, built = self.command("win32-x64")
        self.assertIn("x86_64-pc-windows-gnu", [str(part) for part in command])
        self.assertEqual(built.name, "agent-desktop.exe")

    def test_offline_build_uses_a_complete_dependency_set_only(self):
        with tempfile.TemporaryDirectory() as temp:
            with self.assertRaisesRegex(ValueError, "no complete dependency set"):
                runtime.cargo_build_command("linux-x64", temp)
            (Path(temp) / ".biorouter-vendor-complete").write_text("x")
            command, env, _built = runtime.cargo_build_command("linux-x64", temp)
            command = [str(part) for part in command]
            self.assertIn("--offline", command)
            self.assertIn('source.crates-io.replace-with="biorouter-agent-desktop"', command)
            self.assertIn(f"--remap-path-prefix={Path(temp).resolve().as_posix()}=/crates", env["RUSTFLAGS"])

    def test_build_host_rules(self):
        with mock.patch.object(runtime.platform, "system", return_value="Linux"):
            runtime.check_build_host("win32-x64")
            with self.assertRaisesRegex(ValueError, "built on macOS"):
                runtime.check_build_host("darwin-arm64")
        with mock.patch.object(runtime.platform, "system", return_value="Darwin"):
            with self.assertRaisesRegex(ValueError, "build-computer-use-helper-cross.sh"):
                runtime.check_build_host("linux-x64")

    def test_info_plist_is_the_copilot_identity(self):
        info = runtime.info_plist("darwin-arm64")
        self.assertEqual(info["CFBundleIdentifier"], "org.biorouter.computer-use")
        self.assertEqual(info["CFBundleExecutable"], "agent-desktop")
        self.assertEqual(info["CFBundleName"], "Biorouter Copilot")
        self.assertEqual(info["CFBundleShortVersionString"], runtime.biorouter_version())
        self.assertEqual(info["LSMinimumSystemVersion"], runtime.PIN["minimum_macos"])


@unittest.skipUnless(shutil.which("bash") and platform.system() != "Windows", "the recipe is bash")
class ContainerRecipeTests(unittest.TestCase):
    """cross_copilot_helper, with `docker` replaced by a recorder."""

    def recipe(self, target):
        with tempfile.TemporaryDirectory() as temp:
            inner = Path(temp) / "inner.sh"
            args = Path(temp) / "args.txt"
            script = f'''
. "{runtime.ROOT}/scripts/cross-env.sh"
docker() {{
  case "$1" in info|volume) return 0 ;; esac
  printf '%s\\n' "${{@: -1}}" > "{inner}"
  printf '%s\\n' "$@" > "{args}"
}}
cross_copilot_helper {target}
'''
            result = subprocess.run(["bash", "-c", script], capture_output=True, text=True)
            if result.returncode:
                return result, None, None
            return result, inner.read_text(), args.read_text().splitlines()

    def test_linux_and_windows_recipes_build_the_target_in_the_pinned_image(self):
        for target, platform_name in [("linux-x64", "linux/amd64"), ("linux-arm64", "linux/arm64"),
                                      ("win32-x64", "linux/amd64")]:
            with self.subTest(target=target):
                result, inner, args = self.recipe(target)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(subprocess.run(["sh", "-n"], input=inner, text=True).returncode, 0)
                self.assertIn(f"python3 scripts/computer-use-runtime.py build {target}", inner)
                self.assertIn("rust:1.92-bullseye", args)
                self.assertEqual(args[args.index("--platform") + 1], platform_name)
                self.assertNotIn("winpthread", inner, "the Windows helper must not take cross_windows's linker wrap")
                self.assertIn('chown -R "$HOST_UID:$HOST_GID"', inner)
        _result, inner, _args = self.recipe("win32-x64")
        self.assertIn("gcc-mingw-w64-x86-64", inner)

    def test_macos_targets_have_no_container_recipe(self):
        result, _inner, _args = self.recipe("darwin-arm64")
        self.assertEqual(result.returncode, 2)
        self.assertIn("darwin targets build on macOS", result.stderr)


class ContractTests(unittest.TestCase):
    """`contract` parses the binary's envelopes; a fake helper stands in for it."""

    def run_contract(self, version_line, click_line, click_code=1):
        with tempfile.TemporaryDirectory() as temp:
            fake = Path(temp) / "agent-desktop"
            fake.write_text(
                "#!/usr/bin/env python3\nimport sys\n"
                f"if sys.argv[1:] == ['version']:\n    print({version_line!r}); sys.exit(0)\n"
                f"print({click_line!r}); sys.exit({click_code})\n")
            fake.chmod(0o755)
            with mock.patch.object(runtime, "verify"), mock.patch.object(runtime, "executable", return_value="agent-desktop"):
                runtime.contract(temp, "linux-x64")

    def envelope(self, **fields):
        return json.dumps(fields)

    @unittest.skipIf(platform.system() == "Windows", "the fake helper is a script")
    def test_pinned_version_and_refused_bare_reference_pass(self):
        ok = self.envelope(version="2.6", ok=True, command="version", data={"version": runtime.PIN["upstream_version"]})
        refused = self.envelope(version="2.6", ok=False, command="click", error={
            "code": "INVALID_ARGS", "disposition": {"delivery": "not_delivered", "retry": "safe"}})
        self.run_contract(ok, refused)
        with self.assertRaisesRegex(ValueError, "is not the pinned"):
            self.run_contract(ok.replace(runtime.PIN["upstream_version"], "9.9.9"), refused)
        with self.assertRaisesRegex(ValueError, "is not the pinned"):
            self.run_contract(ok.replace('"2.6"', '"3.0"'), refused)
        with self.assertRaisesRegex(ValueError, "not refused before delivery"):
            self.run_contract(ok, refused.replace("not_delivered", "delivered_unverified"))
        with self.assertRaisesRegex(ValueError, "not refused before delivery"):
            self.run_contract(ok, refused, click_code=0)


# ── the vendored tree's guard ──────────────────────────────────────────────

class VendoredTreeGuardTests(unittest.TestCase):
    """The guard is exercised on a copy, with each failure it exists to catch."""

    @classmethod
    def setUpClass(cls):
        cls.temp = tempfile.TemporaryDirectory()
        cls.vendor = Path(cls.temp.name) / "agent-desktop"
        shutil.copytree(vendor_source.VENDOR, cls.vendor)

    @classmethod
    def tearDownClass(cls):
        cls.temp.cleanup()

    def setUp(self):
        self.patches = [mock.patch.object(vendor_source, "VENDOR", self.vendor),
                        mock.patch.object(vendor_source, "SOURCE", self.vendor / "source"),
                        mock.patch.object(vendor_source, "MANIFEST", self.vendor / "source-manifest.json"),
                        mock.patch.object(vendor_source, "DEPENDENCIES", self.vendor / "DEPENDENCIES.txt")]
        for patch in self.patches:
            patch.start()
        self.saved = {}

    def tearDown(self):
        for path, data in self.saved.items():
            if data is None:
                path.unlink(missing_ok=True)
            else:
                path.write_bytes(data)
        for patch in self.patches:
            patch.stop()

    def change(self, relative, data):
        path = self.vendor / relative
        self.saved.setdefault(path, path.read_bytes() if path.exists() else None)
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)

    def test_the_committed_tree_verifies(self):
        manifest = vendor_source.verify()
        self.assertEqual(manifest["upstream_commit"], runtime.PIN["upstream_commit"])

    def test_an_edited_file_is_refused(self):
        self.change("source/src/main.rs", b"fn main() {}\n")
        with self.assertRaisesRegex(ValueError, r"changed=\['src/main.rs'\]"):
            vendor_source.verify()

    def test_an_unrecorded_file_is_refused(self):
        self.change("source/src/backdoor.rs", b"// nobody reviewed this\n")
        with self.assertRaisesRegex(ValueError, r"unrecorded=\['src/backdoor.rs'\]"):
            vendor_source.verify()

    def test_a_network_client_in_the_lockfile_is_refused(self):
        lock = (self.vendor / "source/Cargo.lock").read_bytes()
        self.change("source/Cargo.lock", lock + b'\n[[package]]\nname = "reqwest"\nversion = "0.12.0"\n')
        # Re-record the tree so only the network rule can fail.
        manifest = vendor_source.build_manifest()
        self.change("source-manifest.json", json.dumps(manifest).encode())
        with self.assertRaisesRegex(ValueError, r"network client crates \['reqwest'\]"):
            vendor_source.verify()
        self.assertEqual(vendor_source.network_crates(lock.decode() + '\nname = "reqwest"\n'), ["reqwest"])

    def test_the_dependency_list_must_match_the_lockfile(self):
        text = (self.vendor / "DEPENDENCIES.txt").read_text()
        row = next(line for line in text.splitlines() if line.startswith("zbus "))
        self.change("DEPENDENCIES.txt", text.replace(row + "\n", "").encode())
        with self.assertRaisesRegex(ValueError, r"unlisted=\[\('zbus'"):
            vendor_source.verify()

    def test_a_pin_move_without_revendoring_is_refused(self):
        pin = json.loads((self.vendor / "pin.json").read_text())
        pin["upstream_commit"] = "1" * 40
        self.change("pin.json", json.dumps(pin).encode())
        with self.assertRaisesRegex(ValueError, "upstream_commit"):
            vendor_source.verify()

    def test_every_vendored_file_is_inside_the_declared_subset(self):
        self.assertEqual(vendor_source.included_reason("crates/core/src/lib.rs") is not None, True)
        self.assertIsNone(vendor_source.included_reason("rust-toolchain.toml"))
        self.assertIsNone(vendor_source.included_reason("apps/docs/index.mdx"))
        self.assertIsNone(vendor_source.included_reason("tests/e2e/run.sh"))

    def test_the_tree_is_exempt_from_end_of_line_conversion(self):
        attributes = (runtime.ROOT / ".gitattributes").read_text()
        self.assertIn("vendor/agent-desktop/source/** -text", attributes)


# ── packaged locations and Linux dependencies ──────────────────────────────

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
            found = acceptance.installed_linux_helper_roots(opt=root / "opt", lib=lib, libexec=root / "usr/libexec")
            self.assertEqual(found, [lib / "biorouter/resources/computer-use/manifest.json"])
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            libexec = root / "usr/libexec"
            stage_helper(libexec, "biorouter")
            found = acceptance.installed_linux_helper_roots(opt=root / "opt", lib=root / "usr/lib", libexec=libexec)
            self.assertEqual(found, [libexec / "biorouter/computer-use/manifest.json"])
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            lib, libexec = root / "usr/lib", root / "usr/libexec"
            stage_helper(lib, "biorouter/resources")
            stage_helper(libexec, "biorouter")
            self.assertEqual(len(acceptance.installed_linux_helper_roots(opt=root / "opt", lib=lib, libexec=libexec)), 2)

    def test_linux_paths_resolves_the_cli_beside_each_kind_of_install(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            lib, libexec, opt = root / 'usr/lib', root / 'usr/libexec', root / 'opt'
            stage_helper(lib, 'biorouter/resources')
            cli, helper = acceptance.linux_paths(lib=lib, libexec=libexec, opt=opt)
            self.assertEqual(helper, lib / 'biorouter/resources/computer-use')
            self.assertEqual(cli, lib / 'biorouter/resources/bin/biorouter')
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            lib, libexec, opt = root / 'usr/lib', root / 'usr/libexec', root / 'opt'
            stage_helper(libexec, 'biorouter')
            cli, helper = acceptance.linux_paths(lib=lib, libexec=libexec, opt=opt)
            self.assertEqual(helper, libexec / 'biorouter/computer-use')
            self.assertEqual(cli, Path('/usr/bin/biorouter'))
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            lib, libexec, opt = root / 'usr/lib', root / 'usr/libexec', root / 'opt'
            stage_helper(lib, 'biorouter/resources')
            stage_helper(libexec, 'biorouter')
            with self.assertRaisesRegex(ValueError, 'Expected one installed helper'):
                acceptance.linux_paths(lib=lib, libexec=libexec, opt=opt)

    def test_rpmbuild_cannot_rewrite_the_helper_after_the_build(self):
        """rpm 4.18's brp-strip-comment-note targets ALREADY-stripped ELF -- which the
        helper is (agent-desktop's release profile strips) -- and repacks it, which
        breaks the recorded payload hashes. The deb is unaffected because dpkg does
        not post-process. electron-installer-redhat offers no option for this, so the
        lever is a build-scoped $HOME holding .rpmmacros."""
        desktop = (runtime.ROOT / "ui/desktop/forge.config.ts").read_text()
        self.assertIn("%__os_install_post %{nil}", desktop,
                      "rpmbuild would strip the helper and invalidate its provenance")
        for hook in ["preMake", "postMake"]:
            self.assertIn(hook, desktop, f"the macros are applied and released in {hook}")
        code = "\n".join(line for line in desktop.splitlines() if not line.strip().startswith("//"))
        self.assertNotIn("fpm:", code, "electron-installer-redhat never reads fpm options")

    def test_no_maker_declares_an_inert_install_prefix(self):
        desktop = (runtime.ROOT / "ui/desktop/forge.config.ts").read_text()
        code = "\n".join(line for line in desktop.splitlines() if not line.strip().startswith("//"))
        self.assertNotIn("prefix:", code, "electron-installer-{debian,redhat} have no prefix option")

    def test_osx_sign_leaves_the_copilot_bundle_alone(self):
        desktop = (runtime.ROOT / "ui/desktop/forge.config.ts").read_text()
        self.assertIn("file.includes(`/computer-use/${COPILOT_BUNDLE}`)", desktop)
        self.assertNotIn("BioRouter Computer Use.app", desktop)


class DependencyContractTests(unittest.TestCase):
    """The packages declare what the engine needs, and no longer what the retired one did."""

    RETIRED = ["python3", "python3-gi", "gir1.2-atspi-2.0", "gir1.2-gtk-3.0", "python3-gobject", "gtk3"]

    def sources(self):
        return ((runtime.ROOT / "packaging/biorouter-cli.yaml").read_text(),
                (runtime.ROOT / "ui/desktop/forge.config.ts").read_text(),
                (runtime.ROOT / "Dockerfile").read_text())

    def test_dependency_contract_in_both_package_formats(self):
        cli, desktop, _dockerfile = self.sources()
        for packages in runtime.PIN["linux_dependencies"].values():
            for package in packages:
                self.assertIn("- " + package + "\n", cli)
                self.assertIn("'" + package + "'", desktop)

    def test_the_retired_engines_python_and_gtk_dependencies_are_gone(self):
        cli, desktop, dockerfile = self.sources()
        for package in self.RETIRED:
            with self.subTest(package=package):
                self.assertNotIn("      - " + package + "\n", cli)
                self.assertNotIn("'" + package + "',", desktop)
                self.assertNotIn("    " + package + " \\", dockerfile)


if __name__ == "__main__":
    unittest.main()
