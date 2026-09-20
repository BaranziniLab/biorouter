#!/usr/bin/env python3
"""Build and verify BioRouter's pinned native Computer Use payload (build-time only)."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import shutil
import struct
import subprocess
import importlib.util as _importlib_util
import zlib

ROOT = Path(__file__).resolve().parents[1]
VENDOR = ROOT / "vendor/computer-use"
PIN = json.loads((VENDOR / "pin.json").read_text())
OUTPUT = ROOT / "target/computer-use"
SIGN_IDENTITY = "Developer ID Application: University of California at San Francisco (F3YYBXAFJ8)"

# Loaded by path because the file name contains dashes and is not importable.
_vendor_spec = _importlib_util.spec_from_file_location(
    "vendor_computer_use_source", Path(__file__).with_name("vendor-computer-use-source.py")
)
vendor_source = _importlib_util.module_from_spec(_vendor_spec)
_vendor_spec.loader.exec_module(vendor_source)


def run(args, **kwargs):
    return subprocess.run([str(a) for a in args], check=True, **kwargs)


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def patches():
    return [{"path": p.name, "sha256": digest(p)} for p in sorted((VENDOR / "patches").glob("*.patch"))]


def payload_files(directory):
    result = []
    for path in sorted(directory.rglob("*")):
        if path.is_symlink():
            raise ValueError(f"Unexpected symlink in helper payload: {path}")
        if path.is_file() and path != directory / "manifest.json":
            result.append({"path": path.relative_to(directory).as_posix(), "sha256": digest(path)})
    return result


def executable(target):
    if target.startswith("darwin-"):
        return "BioRouter Computer Use.app/Contents/MacOS/ocu"
    return "ocu.exe" if target.startswith("win32-") else "ocu"


def check_binary(path, target):
    data = path.read_bytes()[:4096]
    valid = False
    if target.startswith("darwin-") and data[:4] == b"\xcf\xfa\xed\xfe":
        expected = 0x0100000C if target.endswith("arm64") else 0x01000007
        valid = struct.unpack_from("<I", data, 4)[0] == expected
    elif target.startswith("linux-"):
        valid = data[:6] == b"\x7fELF\x02\x01" and struct.unpack_from("<H", data, 18)[0] == (183 if target.endswith("arm64") else 62)
    elif target == "win32-x64" and data[:2] == b"MZ":
        offset = struct.unpack_from("<I", data, 60)[0]
        valid = data[offset:offset + 6] == b"PE\0\0\x64\x86"
    if not valid:
        raise ValueError(f"Native helper architecture does not match {target}: {path}")


def write_manifest(directory, target):
    manifest = {key: PIN[key] for key in ("schema_version", "upstream_commit", "upstream_version", "patch_revision")}
    manifest.update(target=target, executable=executable(target), args=["mcp"], patches=patches(),
                    files=payload_files(directory), linux_dependencies=PIN["linux_dependencies"])
    (directory / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")


def verify(directory, target, signed=False):
    manifest = json.loads((directory / "manifest.json").read_text())
    for key in ("schema_version", "upstream_commit", "upstream_version", "patch_revision"):
        if manifest.get(key) != PIN[key]:
            raise ValueError(f"Helper {key} does not match the source pin")
    if manifest.get("target") != target or manifest.get("executable") != executable(target):
        raise ValueError(f"Stale/foreign helper payload: expected {target}")
    if manifest.get("patches") != patches():
        raise ValueError("Helper patches changed; rebuild the native runtime")
    if manifest.get("files") != payload_files(directory):
        raise ValueError("Helper payload was modified, incomplete, or contains unrecorded files")
    if manifest.get("args") != ["mcp"]:
        raise ValueError("Unexpected native helper launch arguments")
    check_binary(directory / executable(target), target)
    if os.name != "nt" and not target.startswith("win32-") and not os.access(directory / executable(target), os.X_OK):
        raise ValueError("Native helper is not executable")
    if signed and target.startswith("darwin-"):
        app = directory / "BioRouter Computer Use.app"
        run(["codesign", "--verify", "--strict", "--deep", app])
        info = subprocess.check_output(["codesign", "-dv", "--verbose=4", str(app)], stderr=subprocess.STDOUT).decode()
        if "TeamIdentifier=F3YYBXAFJ8" not in info or "Identifier=" + PIN["bundle_identifier"] not in info:
            raise ValueError("Helper is not signed with the BioRouter release identity")
    return manifest


def source_checkout(source):
    """Stage the upstream tree and apply the reviewed patches to it.

    The default source is the tree VENDORED in this repository at
    `vendor/computer-use/source/`, so a build needs no network and
    cannot be changed by anything happening upstream. That is the point: an
    upstream repository that is deleted, force-pushed or altered can no longer
    affect a BioRouter build, and the bytes that go into a shipped helper are
    reviewable in this repository's own history.

    ⚠ The vendored tree is PRISTINE — the patches are not applied to it, they are
    applied here, to a throwaway copy. Keeping the two apart is what makes an
    upstream update tractable: the tree is replaced wholesale and the reviewed
    patches are re-applied on top, so a conflict is a real conflict rather than a
    merge of our own edits with themselves.

    `--source <clone>` still takes an external checkout, which is how you build
    against a candidate upstream before vendoring it. It is pinned exactly as
    before; only the origin of the bytes differs.
    """
    destination = OUTPUT / "source.noindex"
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        shutil.rmtree(destination)
    if source:
        run(["git", "clone", "--no-checkout", source, destination])
        run(["git", "checkout", "--detach", PIN["upstream_commit"]], cwd=destination)
        actual = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=destination).decode().strip()
        if actual != PIN["upstream_commit"]:
            raise ValueError("Upstream source pin mismatch")
    else:
        # Hash every vendored file against the manifest BEFORE it becomes build
        # input. Without this the vendored tree is only files on disk, and an
        # accidental edit — or a deliberate one — would be baked into a signed
        # helper with nothing to notice.
        vendor_source.verify()
        shutil.copytree(vendor_source.SOURCE, destination)
        # ⚠ Give the staged copy its own repository, and do not remove this as
        # redundant. `git apply` resolves against the repository CONTAINING the
        # working directory, and this directory sits inside BioRouter's own — so
        # without an inner repo the patches are applied in BioRouter's context,
        # where `target/` is ignored. Measured: the edits to tracked upstream
        # files landed, every `new file mode` hunk was silently dropped, and the
        # staged tree stayed at 406 files instead of 415. The build then
        # succeeded and produced a helper 28,672 bytes smaller, built from
        # effectively unpatched upstream, with nothing failing.
        #
        # The clone path never had this problem because a clone IS a repository.
        # `git init` restores exactly those semantics.
        run(["git", "init", "--quiet"], cwd=destination)
    for patch in sorted((VENDOR / "patches").glob("*.patch")):
        run(["git", "apply", "--check", patch], cwd=destination)
        run(["git", "apply", patch], cwd=destination)
    return destination


def draw_cursor(path):
    # Original blue arrow geometry; no upstream/reference artwork is copied.
    width = height = 126
    polygon = [(60, 56), (60, 80), (66, 74), (72, 86), (77, 83), (71, 71), (83, 71)]
    def inside(x, y):
        crossings = 0
        for i, (ax, ay) in enumerate(polygon):
            bx, by = polygon[(i + 1) % len(polygon)]
            if (ay > y) != (by > y) and x < (bx - ax) * (y - ay) / (by - ay) + ax:
                crossings += 1
        return crossings % 2
    raw = b"".join(b"\0" + b"".join(bytes((30, 112, 220, 255)) if inside(x, y) else bytes(4)
                                    for x in range(width)) for y in range(height))
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
                     + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def build(target, source, signing_identity):
    checkout = source_checkout(source)
    destination = OUTPUT / target
    # Failed builds must never leave an older payload eligible for packaging.
    if destination.exists():
        shutil.rmtree(destination)
    destination.mkdir(parents=True)
    binary = destination / executable(target)
    binary.parent.mkdir(parents=True, exist_ok=True)
    if target.startswith("darwin-"):
        if platform.system() != "Darwin":
            raise ValueError("macOS helpers must be built on macOS with Swift 6.2+")
        arch = "arm64" if target.endswith("arm64") else "x86_64"
        scratch = OUTPUT / ("swift-" + target + ".noindex")
        args = ["swift", "build", "-c", "release", "--jobs", os.environ.get("BIOROUTER_BUILD_JOBS", "2"), "--triple", f"{arch}-apple-macosx14.0", "--scratch-path", scratch]
        run(args + ["--product", "OpenComputerUse"], cwd=checkout)
        built = Path(subprocess.check_output([str(a) for a in args + ["--show-bin-path"]], cwd=checkout).decode().strip())
        shutil.copy2(built / "OpenComputerUse", binary)
        contents = binary.parent.parent
        resources = contents / "Resources"
        resources.mkdir()
        draw_cursor(resources / "biorouter-cursor.png")
        shutil.copy2(ROOT / "ui/desktop/src/images/icon.icns", resources / "BioRouter.icns")
        info = dict(CFBundleIdentifier=PIN["bundle_identifier"], CFBundleName="BioRouter Computer Use",
                    CFBundleDisplayName="BioRouter Computer Use", CFBundleExecutable="ocu",
                    CFBundlePackageType="APPL", CFBundleShortVersionString=PIN["upstream_version"],
                    CFBundleVersion="1", LSMinimumSystemVersion=PIN["minimum_macos"], LSUIElement=True,
                    NSHighResolutionCapable=True, NSPrincipalClass="NSApplication", CFBundleIconFile="BioRouter.icns")
        (contents / "Info.plist").write_bytes(plistlib.dumps(info))
        binary.chmod(0o755)
        identity = signing_identity or (SIGN_IDENTITY if os.environ.get("APPLE_ID") else "-")
        args = ["codesign", "--force", "--sign", identity]
        if identity != "-":
            args += ["--options", "runtime", "--timestamp"]
        run(args + [contents.parent])
    else:
        app = "OpenComputerUseWindows" if target == "win32-x64" else "OpenComputerUseLinux"
        env = dict(os.environ, GOOS="windows" if target == "win32-x64" else "linux", GOARCH="arm64" if target.endswith("arm64") else "amd64", CGO_ENABLED="0", GOMAXPROCS=os.environ.get("BIOROUTER_BUILD_JOBS", "2"))
        run(["go", "build", "-trimpath", "-buildvcs=false", "-ldflags=-s -w", "-o", binary, "."], cwd=checkout / "apps" / app, env=env)
        binary.chmod(0o755)
        if target == "win32-x64" and os.environ.get("WINDOWS_CERTIFICATE_FILE"):
            signed_binary = binary.with_suffix(".signed.exe")
            run(["osslsigncode", "sign", "-pkcs12", os.environ["WINDOWS_CERTIFICATE_FILE"],
                 "-readpass", os.environ["BIOROUTER_WINDOWS_SIGN_PASSWORD_FILE"], "-h", "sha256",
                 "-ts", "http://timestamp.digicert.com", "-in", binary, "-out", signed_binary])
            signed_binary.replace(binary)
    for name in ("LICENSE", "NOTICE", "pin.json"):
        shutil.copy2(VENDOR / name, destination / name)
    write_manifest(destination, target)
    verify(destination, target)
    print(destination)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("build", "verify", "stage"))
    parser.add_argument("target", choices=PIN["targets"])
    parser.add_argument("--source", help="Local upstream git clone; the pinned commit is always checked out afresh")
    parser.add_argument("--directory", type=Path)
    parser.add_argument("--signing-identity")
    parser.add_argument("--require-signed", action="store_true")
    args = parser.parse_args()
    if args.action == "build":
        build(args.target, args.source, args.signing_identity)
    elif args.action == "verify":
        verify(args.directory or OUTPUT / args.target, args.target, args.require_signed)
        print(f"Verified Computer Use {args.target}")
    else:
        source = OUTPUT / args.target
        verify(source, args.target, args.require_signed)
        destination = args.directory or ROOT / "ui/desktop/src/computer-use"
        if destination.exists():
            shutil.rmtree(destination)
        shutil.copytree(source, destination)
        verify(destination, args.target, args.require_signed)
        print(destination)


if __name__ == "__main__":
    main()
