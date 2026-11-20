#!/usr/bin/env python3
"""Build and verify Biorouter Copilot's engine payload (build-time only).

The engine is agent-desktop (AD-01 in docs/design/copilot-agent-desktop.md), built
with cargo from the source vendored at vendor/agent-desktop/source. A payload is
the folder `target/computer-use/<target>/`: the binary (inside `Biorouter
Copilot.app` on macOS), upstream's licence, the list of third-party crates in it,
and `manifest.json`, which records the pin, the target and the SHA-256 of every
other file. Packaging copies that folder to `computer-use/` in each install.

    computer-use-runtime.py build    <target> [--signing-identity ID] [--offline-deps DIR]
    computer-use-runtime.py verify   <target> [--directory DIR] [--require-signed]
    computer-use-runtime.py stage    <target> [--directory DIR] [--require-signed]
    computer-use-runtime.py contract <target> [--directory DIR]
    computer-use-runtime.py vendor-deps [--directory DIR]

`build` runs on the target's own OS for darwin targets (Xcode command line tools)
and on Linux for the linux and win32 targets (`scripts/build-computer-use-helper-cross.sh`
runs it inside the pinned rust:1.92-bullseye image, with mingw for win32). Cargo is
`cargo` on PATH unless BIOROUTER_HELPER_CARGO names another; the Rust toolchain is
the repository's own (rust-toolchain.toml), never upstream's 1.89 pin (AD-03).
"""
import argparse
import hashlib
import importlib.util as _importlib_util
import json
import os
from pathlib import Path
import platform
import plistlib
import re
import shutil
import stat
import struct
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[1]
VENDOR = ROOT / "vendor/agent-desktop"
PIN = json.loads((VENDOR / "pin.json").read_text(encoding="utf-8"))
OUTPUT = ROOT / "target/computer-use"
BUILD = ROOT / "target/agent-desktop"
OFFLINE_DEPS = ROOT / "target/agent-desktop-vendor"
SIGN_IDENTITY = "Developer ID Application: University of California at San Francisco (F3YYBXAFJ8)"
TEAM_IDENTIFIER = "F3YYBXAFJ8"
MANIFEST_SCHEMA = 2
LICENSE_FILE = "LICENSE.agent-desktop"
DEPENDENCIES_FILE = "DEPENDENCIES.agent-desktop.txt"
DISPLAY_NAME = "Biorouter Copilot"
# Shared libraries a Linux helper may need. They are part of glibc (or, for
# libgcc_s, of every system that has glibc), so `at-spi2-core` stays the only
# package dependency (AD-06). Anything else would need a new package dependency.
LINUX_SYSTEM_LIBRARIES = {
    "libc.so.6", "libm.so.6", "libdl.so.2", "libpthread.so.0", "librt.so.1", "libutil.so.1",
    "libgcc_s.so.1", "ld-linux-x86-64.so.2", "ld-linux-aarch64.so.1",
}

# Loaded by path because the file name contains dashes and is not importable.
_vendor_spec = _importlib_util.spec_from_file_location(
    "vendor_computer_use_source", Path(__file__).with_name("vendor-computer-use-source.py")
)
vendor_source = _importlib_util.module_from_spec(_vendor_spec)
_vendor_spec.loader.exec_module(vendor_source)


def run(args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def targets():
    return list(PIN["targets"])


def triple(target):
    return PIN["targets"][target]


def biorouter_version():
    """The workspace version, which the macOS bundle carries and the manifest records."""
    text = (ROOT / "Cargo.toml").read_text(encoding="utf-8")
    section = re.search(r"^\[workspace\.package\](.*?)(?=^\[|\Z)", text, flags=re.MULTILINE | re.DOTALL)
    match = section and re.search(r'^version\s*=\s*"([^"]+)"', section.group(1), flags=re.MULTILINE)
    if not match:
        raise ValueError("cannot read [workspace.package] version from Cargo.toml")
    return match.group(1)


def toolchain_channel():
    match = re.search(r'^channel\s*=\s*"([^"]+)"', (ROOT / "rust-toolchain.toml").read_text(encoding="utf-8"),
                      flags=re.MULTILINE)
    if not match:
        raise ValueError("cannot read the toolchain channel from rust-toolchain.toml")
    return match.group(1)


def executable(target):
    if target.startswith("darwin-"):
        return f"{PIN['macos_bundle']}/Contents/MacOS/agent-desktop"
    return "agent-desktop.exe" if target.startswith("win32-") else "agent-desktop"


def minimum_os(target):
    if target.startswith("darwin-"):
        return {"macos": PIN["minimum_macos"]}
    if target.startswith("linux-"):
        return {"glibc": PIN["glibc_floor"]}
    return {"windows": "10"}


def payload_files(directory):
    """Every payload file but the manifest, sorted by its `/`-separated path.

    ⚠ Sorted by the path STRING, not by Path objects: a WindowsPath compares
    case-insensitively, so a payload built on Linux (`DEPENDENCIES…`, `LICENSE…`,
    `agent-desktop.exe`) would list in another order when verified on Windows.
    """
    directory = Path(directory)
    result = []
    for path in directory.rglob("*"):
        if path.is_symlink():
            raise ValueError(f"Unexpected symlink in helper payload: {path}")
        if path.is_file() and path != directory / "manifest.json":
            result.append({"path": path.relative_to(directory).as_posix(), "sha256": digest(path)})
    return sorted(result, key=lambda entry: entry["path"])


# ── binary inspection, pure Python so a macOS host can check a Linux or Windows payload ──

def _version_tuple(text):
    return tuple(int(part) for part in text.split("."))


def macho_minimum_macos(data):
    """The macOS minimum a thin 64-bit Mach-O declares, as "13.0", or None."""
    if data[:4] != b"\xcf\xfa\xed\xfe":
        return None
    ncmds = struct.unpack_from("<I", data, 16)[0]
    offset = 32
    for _ in range(ncmds):
        cmd, size = struct.unpack_from("<II", data, offset)
        if cmd == 0x32:  # LC_BUILD_VERSION
            platform_id, minos = struct.unpack_from("<II", data, offset + 8)
            if platform_id == 1:  # PLATFORM_MACOS
                return f"{minos >> 16}.{(minos >> 8) & 0xFF}" + (f".{minos & 0xFF}" if minos & 0xFF else "")
        elif cmd == 0x24:  # LC_VERSION_MIN_MACOSX
            version = struct.unpack_from("<I", data, offset + 8)[0]
            return f"{version >> 16}.{(version >> 8) & 0xFF}" + (f".{version & 0xFF}" if version & 0xFF else "")
        offset += size
    return None


def _elf_sections(data):
    if data[:6] != b"\x7fELF\x02\x01":
        raise ValueError("not a 64-bit little-endian ELF file")
    shoff = struct.unpack_from("<Q", data, 0x28)[0]
    entsize, count = struct.unpack_from("<HH", data, 0x3A)
    sections = []
    for index in range(count):
        base = shoff + index * entsize
        _name, kind, _flags, _addr, offset, size, link, info = struct.unpack_from("<IIQQQQII", data, base)
        sections.append({"type": kind, "offset": offset, "size": size, "link": link, "info": info})
    return sections


def _c_string(data, offset):
    end = data.index(b"\0", offset)
    return data[offset:end].decode("ascii", "replace")


def elf_glibc_versions(data):
    """Every GLIBC_x.y version the ELF's .gnu.version_r section requires."""
    sections = _elf_sections(data)
    versions = set()
    for section in sections:
        if section["type"] != 0x6FFFFFFE:  # SHT_GNU_verneed
            continue
        strings = sections[section["link"]]["offset"]
        entry = section["offset"]
        for _ in range(section["info"]):
            _version, count, _file, aux, following = struct.unpack_from("<HHIII", data, entry)
            item = entry + aux
            for _ in range(count):
                _hash, _flags, _other, name, after = struct.unpack_from("<IHHII", data, item)
                text = _c_string(data, strings + name)
                if text.startswith("GLIBC_") and re.fullmatch(r"GLIBC_[0-9.]+", text):
                    versions.add(text[len("GLIBC_"):])
                item += after
            entry += following
            if not following:
                break
    return versions


def elf_needed_libraries(data):
    """The DT_NEEDED shared libraries of an ELF file."""
    sections = _elf_sections(data)
    needed = []
    for section in sections:
        if section["type"] != 6:  # SHT_DYNAMIC
            continue
        strings = sections[section["link"]]["offset"]
        for entry in range(section["offset"], section["offset"] + section["size"], 16):
            tag, value = struct.unpack_from("<qQ", data, entry)
            if tag == 0:
                break
            if tag == 1:  # DT_NEEDED
                needed.append(_c_string(data, strings + value))
    return needed


def pe_details(data):
    """(subsystem, imported DLL names) of a PE32+ image."""
    offset = struct.unpack_from("<I", data, 0x3C)[0]
    if data[offset:offset + 4] != b"PE\0\0":
        raise ValueError("not a PE image")
    sections_count = struct.unpack_from("<H", data, offset + 6)[0]
    optional_size = struct.unpack_from("<H", data, offset + 20)[0]
    optional = offset + 24
    if struct.unpack_from("<H", data, optional)[0] != 0x20B:
        raise ValueError("not a PE32+ image")
    subsystem = struct.unpack_from("<H", data, optional + 68)[0]
    import_rva = struct.unpack_from("<I", data, optional + 112 + 8)[0]
    table = optional + optional_size
    sections = []
    for index in range(sections_count):
        base = table + index * 40
        virtual_size, virtual_address, raw_size, raw_pointer = struct.unpack_from("<IIII", data, base + 8)
        sections.append((virtual_address, max(virtual_size, raw_size), raw_pointer))

    def file_offset(rva):
        for virtual_address, size, raw_pointer in sections:
            if virtual_address <= rva < virtual_address + size:
                return raw_pointer + rva - virtual_address
        raise ValueError(f"RVA {rva:#x} lies in no section")

    dlls = []
    if import_rva:
        entry = file_offset(import_rva)
        while True:
            descriptor = struct.unpack_from("<IIIII", data, entry)
            if not any(descriptor):
                break
            dlls.append(_c_string(data, file_offset(descriptor[3])))
            entry += 20
    return subsystem, dlls


def check_binary(path, target, payload_names=()):
    data = Path(path).read_bytes()
    valid = False
    if target.startswith("darwin-") and data[:4] == b"\xcf\xfa\xed\xfe":
        expected = 0x0100000C if target.endswith("arm64") else 0x01000007
        valid = struct.unpack_from("<I", data, 4)[0] == expected
    elif target.startswith("linux-"):
        valid = data[:6] == b"\x7fELF\x02\x01" and struct.unpack_from("<H", data, 18)[0] == (
            183 if target.endswith("arm64") else 62)
    elif target == "win32-x64" and data[:2] == b"MZ":
        offset = struct.unpack_from("<I", data, 60)[0]
        valid = data[offset:offset + 6] == b"PE\0\0\x64\x86"
    if not valid:
        raise ValueError(f"Native helper architecture does not match {target}: {path}")
    if target.startswith("darwin-"):
        found = macho_minimum_macos(data)
        if found is None or _version_tuple(found) != _version_tuple(PIN["minimum_macos"]):
            raise ValueError(f"Native helper declares macOS {found} as its minimum, not {PIN['minimum_macos']}: {path}")
    elif target.startswith("linux-"):
        versions = elf_glibc_versions(data)
        newest = max(versions, key=_version_tuple) if versions else None
        if newest and _version_tuple(newest) > _version_tuple(PIN["glibc_floor"]):
            raise ValueError(f"Native helper needs glibc {newest}, above the {PIN['glibc_floor']} floor: {path}")
        foreign = sorted(set(elf_needed_libraries(data)) - LINUX_SYSTEM_LIBRARIES)
        if foreign:
            raise ValueError(f"Native helper links shared libraries outside glibc {foreign}; each would need "
                             f"a package dependency: {path}")
    else:
        subsystem, dlls = pe_details(data)
        if subsystem != 3:
            raise ValueError(f"Native helper is not a console executable (subsystem {subsystem}): {path}")
        # A MinGW runtime DLL (libgcc_s_seh-1.dll, libwinpthread-1.dll) the payload
        # does not carry would stop the helper from starting at all.
        missing = sorted(dll for dll in dlls if dll.lower().startswith("lib") and dll not in payload_names)
        if missing:
            raise ValueError(f"Native helper imports runtime DLLs the payload does not ship {missing}: {path}")


# ── manifest ──

PINNED_KEYS = ("engine", "upstream_tag", "upstream_commit", "upstream_version", "contract_version")


def write_manifest(directory, target):
    manifest = {"schema_version": MANIFEST_SCHEMA}
    manifest.update({key: PIN[key] for key in PINNED_KEYS})
    manifest.update(
        biorouter_version=biorouter_version(), target=target, triple=triple(target),
        executable=executable(target), minimum_os=minimum_os(target),
        source_tree_sha256=json.loads((VENDOR / "source-manifest.json").read_text(encoding="utf-8"))["tree_sha256"],
        files=payload_files(directory), linux_dependencies=PIN["linux_dependencies"],
    )
    if target.startswith("darwin-"):
        manifest["bundle_identifier"] = PIN["bundle_identifier"]
    (Path(directory) / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="utf-8")


def info_plist(target):
    version = biorouter_version()
    return dict(
        CFBundleIdentifier=PIN["bundle_identifier"], CFBundleName=DISPLAY_NAME,
        CFBundleDisplayName=DISPLAY_NAME, CFBundleExecutable="agent-desktop",
        CFBundlePackageType="APPL", CFBundleShortVersionString=version, CFBundleVersion=version,
        CFBundleIconFile="Biorouter.icns", CFBundleInfoDictionaryVersion="6.0",
        LSMinimumSystemVersion=PIN["minimum_macos"], LSUIElement=True, NSHighResolutionCapable=True,
    )


def verify(directory, target, signed=False):
    directory = Path(directory)
    manifest_path = directory / "manifest.json"
    if not manifest_path.exists():
        raise ValueError(f"Missing Biorouter Copilot payload manifest: {manifest_path}")
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if manifest.get("schema_version") != MANIFEST_SCHEMA:
        raise ValueError("Helper payload predates the agent-desktop engine; rebuild the native runtime")
    for key in PINNED_KEYS:
        if manifest.get(key) != PIN[key]:
            raise ValueError(f"Helper {key} does not match the source pin")
    if manifest.get("biorouter_version") != biorouter_version():
        raise ValueError(f"Helper was built for Biorouter {manifest.get('biorouter_version')}, "
                         f"not {biorouter_version()}; rebuild the native runtime")
    if manifest.get("target") != target or manifest.get("executable") != executable(target) \
            or manifest.get("triple") != triple(target):
        raise ValueError(f"Stale/foreign helper payload: expected {target}")
    if manifest.get("minimum_os") != minimum_os(target):
        raise ValueError("Helper payload records an unexpected minimum OS")
    files = payload_files(directory)
    recorded = manifest.get("files")
    if not isinstance(recorded, list) or sorted(recorded, key=lambda entry: str(entry.get("path"))) != files:
        raise ValueError("Helper payload was modified, incomplete, or contains unrecorded files")
    names = {entry["path"] for entry in files}
    for required in (executable(target), LICENSE_FILE, DEPENDENCIES_FILE):
        if required not in names:
            raise ValueError(f"Helper payload was modified, incomplete, or contains unrecorded files: no {required}")
    if (directory / LICENSE_FILE).read_bytes() != (VENDOR / "LICENSE").read_bytes():
        raise ValueError("Helper payload licence is not upstream's Apache License text")
    check_binary(directory / executable(target), target, names)
    if os.name != "nt" and not target.startswith("win32-") and not os.access(directory / executable(target), os.X_OK):
        raise ValueError("Native helper is not executable")
    if target.startswith("darwin-"):
        app = directory / PIN["macos_bundle"]
        info = plistlib.loads((app / "Contents/Info.plist").read_bytes())
        if info != info_plist(target):
            raise ValueError("Helper bundle Info.plist does not match the identity Biorouter ships")
        if manifest.get("bundle_identifier") != PIN["bundle_identifier"]:
            raise ValueError("Helper payload records a different bundle identifier")
        if platform.system() == "Darwin":
            run(["codesign", "--verify", "--strict", "--deep", app])
            details = subprocess.check_output(["codesign", "-dv", "--verbose=4", str(app)],
                                              stderr=subprocess.STDOUT).decode()
            if "Identifier=" + PIN["bundle_identifier"] not in details:
                raise ValueError("Helper bundle is not signed under its bundle identifier")
            if signed and (f"TeamIdentifier={TEAM_IDENTIFIER}" not in details
                           or not re.search(r"flags=0x[0-9a-f]*\(runtime\)", details)):
                raise ValueError("Helper is not signed with the Biorouter release identity and hardened runtime")
        elif signed:
            raise ValueError("A release signature can only be checked on macOS")
    elif "bundle_identifier" in manifest:
        raise ValueError("Only a macOS helper payload carries a bundle identifier")
    return manifest


def remove_tree(path):
    r"""`shutil.rmtree`, but able to delete read-only files on Windows.

    ⚠ A bare `shutil.rmtree` cannot remove a read-only file on Windows (git pack
    files, and anything a build copied with its mode), because Windows refuses to
    unlink it. On Linux and macOS deletion is governed by the directory's write
    bit, so this is invisible off Windows. Clearing the bit and retrying is the
    documented remedy; anything the retry still cannot remove is re-raised.
    """
    def drop_readonly(func, target, _exception):
        os.chmod(target, stat.S_IWRITE)
        func(target)

    if sys.version_info >= (3, 12):
        shutil.rmtree(path, onexc=drop_readonly)
    else:  # pragma: no cover - the repo pins 3.12; keeps older hosts working
        shutil.rmtree(path, onerror=lambda f, target, _e: drop_readonly(f, target, None))


# ── build ──

def cargo_home():
    return Path(os.environ.get("CARGO_HOME") or Path.home() / ".cargo")


def cargo_build_command(target, offline_deps=None):
    """The cargo command line and environment that build one target's helper."""
    target_dir = BUILD / target
    command = [
        os.environ.get("BIOROUTER_HELPER_CARGO") or "cargo", "build", "--release", "--locked",
        "--manifest-path", vendor_source.SOURCE / "Cargo.toml", "-p", "agent-desktop",
        "--target", triple(target), "--target-dir", target_dir,
        "-j", os.environ.get("BIOROUTER_BUILD_JOBS", "2"),
    ]
    remaps = {vendor_source.SOURCE: "/agent-desktop", cargo_home(): "/cargo", target_dir: "/build"}
    if offline_deps:
        offline_deps = Path(offline_deps).resolve()
        if not (offline_deps / ".biorouter-vendor-complete").exists():
            raise ValueError(f"{offline_deps} holds no complete dependency set; run `vendor-deps` first")
        command += ["--offline", "--config", 'source.crates-io.replace-with="biorouter-agent-desktop"',
                    "--config", f'source.biorouter-agent-desktop.directory="{offline_deps.as_posix()}"']
        remaps[offline_deps] = "/crates"
    env = dict(os.environ)
    # The repository's toolchain, never upstream's 1.89 pin (AD-03). Harmless when
    # cargo is not rustup's proxy.
    env["RUSTUP_TOOLCHAIN"] = toolchain_channel()
    # Two builds from different folders differed byte for byte through embedded
    # paths (measured 2026-11-20). RUSTFLAGS is set, not appended to, so a
    # developer's own flags cannot leak into a shipped helper.
    env["RUSTFLAGS"] = " ".join(f"--remap-path-prefix={Path(path).as_posix()}={alias}"
                                for path, alias in remaps.items())
    for key in ("CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_RUSTFLAGS", "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET"):
        env.pop(key, None)
    if target.startswith("darwin-"):
        # Wins over .cargo/config.toml's 12.0 for Biorouter's own binaries; 13.0 is
        # upstream's tested floor (AD-03). Upstream itself never sets it, so its
        # Objective-C bridges compile at their build script's 10.15 default, where
        # `-Werror` never meets a deprecation. At 13.0 one call
        # (`fullPathForApplication:`, deprecated since 11.0 and still present) fails
        # `-Werror,-Wdeprecated-declarations` (measured 2026-11-20 with the 26 and 27
        # SDKs). The build script takes no extra flags, so clang's own
        # CCC_OVERRIDE_OPTIONS keeps that one warning a warning without patching the
        # vendored source; the leading `#` keeps clang from announcing the edit.
        env["MACOSX_DEPLOYMENT_TARGET"] = PIN["minimum_macos"]
        env["CCC_OVERRIDE_OPTIONS"] = "# +-Wno-error=deprecated-declarations"
    else:
        env.pop("MACOSX_DEPLOYMENT_TARGET", None)
    built = target_dir / triple(target) / "release" / ("agent-desktop.exe" if target.startswith("win32-") else "agent-desktop")
    return command, env, built


def check_build_host(target):
    system = platform.system()
    if target.startswith("darwin-") and system != "Darwin":
        raise ValueError("macOS helpers are built on macOS with the Xcode command line tools")
    if not target.startswith("darwin-") and system != "Linux":
        raise ValueError(f"The {target} helper is built on Linux; use scripts/build-computer-use-helper-cross.sh, "
                         "which runs this build inside the pinned rust:1.92-bullseye image")


def sign_bundle(app, signing_identity):
    identity = signing_identity or (SIGN_IDENTITY if os.environ.get("APPLE_ID") else "-")
    args = ["codesign", "--force", "--sign", identity]
    if identity != "-":
        args += ["--options", "runtime", "--timestamp"]
    run(args + [app])


def build(target, signing_identity=None, offline_deps=None):
    check_build_host(target)
    # Hash every vendored file against its manifest BEFORE it becomes build input,
    # and again after: a build that wrote into the source would otherwise go unseen.
    vendor_source.verify()
    command, env, built = cargo_build_command(target, offline_deps)
    run(command, env=env, cwd=ROOT)
    vendor_source.verify()
    destination = OUTPUT / target
    # Failed builds must never leave an older payload eligible for packaging.
    if destination.exists():
        remove_tree(destination)
    destination.mkdir(parents=True)
    binary = destination / executable(target)
    binary.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(built, binary)
    binary.chmod(0o755)
    if target.startswith("darwin-"):
        contents = binary.parent.parent
        resources = contents / "Resources"
        resources.mkdir()
        shutil.copyfile(ROOT / "ui/desktop/src/images/icon.icns", resources / "Biorouter.icns")
        (contents / "Info.plist").write_bytes(plistlib.dumps(info_plist(target)))
        sign_bundle(contents.parent, signing_identity)
    elif target == "win32-x64" and os.environ.get("WINDOWS_CERTIFICATE_FILE"):
        signed_binary = binary.with_suffix(".signed.exe")
        run(["osslsigncode", "sign", "-pkcs12", os.environ["WINDOWS_CERTIFICATE_FILE"],
             "-readpass", os.environ["BIOROUTER_WINDOWS_SIGN_PASSWORD_FILE"], "-h", "sha256",
             "-ts", "http://timestamp.digicert.com", "-in", binary, "-out", signed_binary])
        signed_binary.replace(binary)
    shutil.copyfile(VENDOR / "LICENSE", destination / LICENSE_FILE)
    shutil.copyfile(VENDOR / "DEPENDENCIES.txt", destination / DEPENDENCIES_FILE)
    write_manifest(destination, target)
    verify(destination, target)
    print(destination)


def vendor_deps(directory):
    """Download every locked crate into DIRECTORY, so a later build needs no network."""
    directory = Path(directory).resolve()
    if directory.exists():
        remove_tree(directory)
    directory.parent.mkdir(parents=True, exist_ok=True)
    env = dict(os.environ, RUSTUP_TOOLCHAIN=toolchain_channel())
    run([os.environ.get("BIOROUTER_HELPER_CARGO") or "cargo", "vendor", "--locked",
         "--manifest-path", vendor_source.SOURCE / "Cargo.toml", directory], env=env, cwd=ROOT,
        stdout=subprocess.DEVNULL)
    (directory / ".biorouter-vendor-complete").write_text(PIN["upstream_commit"] + "\n", encoding="utf-8")
    print(directory)


# ── contract smoke: the binary runs and is the pinned engine, with no desktop access ──

def contract(directory, target):
    """Run two commands that never touch the desktop and check their JSON envelopes.

    `version` reports the pinned release, and a bare `@e1` reference is refused as
    INVALID_ARGS and not delivered. Both run with a throwaway HOME and
    AGENT_DESKTOP_HOME and an emptied environment, so nothing of the machine's own
    state is read or written.
    """
    directory = Path(directory)
    verify(directory, target)
    helper = directory / executable(target)
    with tempfile.TemporaryDirectory(prefix="biorouter-copilot-contract-") as home:
        env = {"HOME": home, "USERPROFILE": home, "AGENT_DESKTOP_HOME": str(Path(home) / "state"),
               "PATH": os.environ.get("PATH", ""), "SYSTEMROOT": os.environ.get("SYSTEMROOT", ""),
               "TMPDIR": home, "TEMP": home, "TMP": home}

        def envelope(*args):
            result = subprocess.run([str(helper), *args], env=env, capture_output=True, text=True,
                                    timeout=60, stdin=subprocess.DEVNULL)
            lines = [line for line in result.stdout.splitlines() if line.strip()]
            if len(lines) != 1:
                raise ValueError(f"`agent-desktop {' '.join(args)}` printed {len(lines)} lines, expected one "
                                 f"JSON envelope (exit {result.returncode}): {result.stdout[:400]!r}")
            return result.returncode, json.loads(lines[0])

        code, version = envelope("version")
        if code != 0 or version.get("ok") is not True or version.get("version") != PIN["contract_version"] \
                or version.get("data", {}).get("version") != PIN["upstream_version"]:
            raise ValueError(f"`agent-desktop version` is not the pinned {PIN['upstream_version']} "
                             f"with contract {PIN['contract_version']}: {version}")
        code, refused = envelope("click", "@e1")
        error = refused.get("error", {})
        if code == 0 or refused.get("ok") is not False or error.get("code") != "INVALID_ARGS" \
                or error.get("disposition", {}).get("delivery") != "not_delivered":
            raise ValueError(f"a bare @e1 reference was not refused before delivery: {refused}")
        if (Path(home) / "state").exists() and any((Path(home) / "state").iterdir()):
            raise ValueError("the contract commands wrote agent-desktop state")
    print(f"Biorouter Copilot {target}: agent-desktop {PIN['upstream_version']}, contract {PIN['contract_version']}")


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("action", choices=("build", "verify", "stage", "contract", "vendor-deps"))
    parser.add_argument("target", nargs="?", choices=targets())
    parser.add_argument("--directory", type=Path)
    parser.add_argument("--signing-identity")
    parser.add_argument("--require-signed", action="store_true")
    parser.add_argument("--offline-deps", type=Path,
                        help="build from a dependency set `vendor-deps` wrote, with no network")
    args = parser.parse_args()
    if args.action == "vendor-deps":
        vendor_deps(args.directory or OFFLINE_DEPS)
        return
    if not args.target:
        parser.error(f"{args.action} needs a target")
    if args.action == "build":
        build(args.target, args.signing_identity, args.offline_deps)
    elif args.action == "verify":
        verify(args.directory or OUTPUT / args.target, args.target, args.require_signed)
        print(f"Verified Biorouter Copilot {args.target}")
    elif args.action == "contract":
        contract(args.directory or OUTPUT / args.target, args.target)
    else:
        source = OUTPUT / args.target
        verify(source, args.target, args.require_signed)
        destination = args.directory or ROOT / "ui/desktop/src/computer-use"
        if destination.exists():
            remove_tree(destination)
        shutil.copytree(source, destination)
        verify(destination, args.target, args.require_signed)
        print(destination)


if __name__ == "__main__":
    main()
