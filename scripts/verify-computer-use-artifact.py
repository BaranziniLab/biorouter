#!/usr/bin/env python3
"""Verify the actual helper bytes inside a release package, after all signing."""
import argparse
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import zipfile

spec = importlib.util.spec_from_file_location("runtime", Path(__file__).with_name("computer-use-runtime.py"))
runtime = importlib.util.module_from_spec(spec)
spec.loader.exec_module(runtime)


def verify_artifact(artifact, target, require_signed=False):
    with tempfile.TemporaryDirectory(prefix="biorouter-helper-verify-") as temp:
        directory = Path(temp)
        mounted = False
        try:
            if artifact.suffix == ".zip":
                with zipfile.ZipFile(artifact) as archive:
                    # Extract only the helper tree, avoiding multi-GB app copies.
                    for member in archive.infolist():
                        # .NET Framework's ZipFile.CreateFromDirectory (cross-zip under powershell.exe,
                        # so the Windows CI zip) writes '\' separators: 799 of the 1.91.0 zip's 818
                        # entries. ZipInfo converts only os.sep, so on macOS each name stayed one literal
                        # part and no payload was found. Normalize BEFORE the traversal guard, or a '..\'
                        # name passes it. extract() then writes the normalized path; the local-header
                        # check compares orig_filename, which this leaves untouched.
                        member.filename = member.filename.replace("\\", "/")
                        parts = Path(member.filename).parts
                        if "computer-use" in parts:
                            if member.filename.startswith("/") or ".." in parts:
                                raise ValueError("Unsafe archive path")
                            extracted = Path(archive.extract(member, directory))
                            mode = member.external_attr >> 16
                            if mode and not member.is_dir():
                                extracted.chmod(mode & 0o777)
            elif artifact.suffix == ".dmg":
                runtime.run(["hdiutil", "attach", "-nobrowse", "-readonly", "-mountpoint", directory, artifact], stdout=subprocess.DEVNULL)
                mounted = True
            elif artifact.suffix == ".deb":
                runtime.run(["ar", "x", artifact], cwd=directory)
                archives = list(directory.glob("data.tar.*"))
                if len(archives) != 1:
                    raise ValueError("DEB must contain one data archive")
                runtime.run(["tar", "-xf", archives[0]], cwd=directory)
            elif artifact.suffix == ".rpm":
                if shutil.which("bsdtar"):
                    runtime.run(["bsdtar", "-xf", artifact], cwd=directory)
                elif runtime.platform.system() == "Darwin":
                    runtime.run(["tar", "-xf", artifact], cwd=directory)
                else:
                    with tempfile.TemporaryFile() as cpio:
                        runtime.run(["rpm2cpio", artifact], stdout=cpio)
                        cpio.seek(0)
                        runtime.run(["cpio", "-id", "--no-absolute-filenames"], stdin=cpio, cwd=directory)
            else:
                raise ValueError(f"Unsupported release archive: {artifact}")
            manifests = [p for p in directory.rglob("manifest.json") if p.parent.name == "computer-use"]
            if len(manifests) != 1:
                raise ValueError(f"Expected exactly one Biorouter Copilot payload, found {len(manifests)}")
            runtime.verify(manifests[0].parent, target, require_signed)
            return {"artifact": artifact.name, "target": target, "manifest_sha256": runtime.digest(manifests[0]),
                    "upstream_commit": runtime.PIN["upstream_commit"], "patches": runtime.patches()}
        finally:
            if mounted:
                runtime.run(["hdiutil", "detach", directory], stdout=subprocess.DEVNULL)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("artifact", type=Path)
    parser.add_argument("target", choices=runtime.PIN["targets"])
    parser.add_argument("--require-signed", action="store_true")
    args = parser.parse_args()
    print(json.dumps(verify_artifact(args.artifact.resolve(), args.target, args.require_signed), sort_keys=True))
