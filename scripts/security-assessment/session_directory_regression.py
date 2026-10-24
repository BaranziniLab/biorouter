#!/usr/bin/env python3
"""Check the production Unix directory guard in a keyless two-UID Linux fixture.

This extracts the guard verbatim, builds only that function with the locked
libc dependency, and checks a synthetic file. It is not a rebuilt-app test.
"""

import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


ROOT = Path(__file__).resolve().parents[2]
OUT = ROOT / "docs/security/assessment-2026-10/environment/session-guard-regression"
IMAGE = os.environ.get("BRSEC_RUST_IMAGE", "rust:1.92-bullseye")
CONTAINER = "brsec-session-guard"


def command(args, *, check=True):
    return subprocess.run(args, check=check, capture_output=True, text=True)


def inside(uid, args, *, check=True):
    return command(["docker", "exec", "--user", f"{uid}:{uid}", CONTAINER, *args], check=check)


def main():
    OUT.mkdir(parents=True, exist_ok=True)
    source = (ROOT / "crates/biorouter/src/session/session_manager.rs").read_text()
    start = source.index("    fn protect_session_directory(path:")
    end = source.index("    #[cfg(windows)]", start)
    guard = source[start:end].strip()
    image = json.loads(command(["docker", "image", "inspect", IMAGE]).stdout)[0]
    pinned = image["Id"]
    with tempfile.TemporaryDirectory(prefix="brsec-session-helper-") as temporary:
        fixture = Path(temporary)
        fixture.chmod(0o755)
        package = fixture / "probe"
        cargo = shutil.which("cargo") or str(Path.home() / ".cargo/bin/cargo")
        command([cargo, "new", "--bin", "--vcs", "none", "--name", "brsec-session-helper", str(package)])
        command([cargo, "add", "--offline", "--manifest-path", str(package / "Cargo.toml"), "libc@=0.2.186"])
        main_source = (
            "use std::{fs, path::Path};\n" + guard + "\n"
            'fn main() { let path = std::env::args_os().nth(1).expect("directory"); '
            'if let Err(error) = protect_session_directory(Path::new(&path)) {'
            'eprintln!("{error}"); std::process::exit(2); }}\n'
        )
        (package / "src/main.rs").write_text(main_source)
        (OUT / "extracted-guard.rs").write_text(main_source)
        output = fixture / "output"
        output.mkdir(mode=0o777)
        output.chmod(0o777)
        registry = Path.home() / ".cargo/registry"
        build = command([
            "docker", "run", "--rm", "--network", "none", "--read-only",
            "--user", "10001:10001", "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
            "--pids-limit", "64", "--cpus", "0.5", "--memory", "512m", "--memory-swap", "512m",
            "--tmpfs", "/tmp:rw,exec,nosuid,nodev,size=256m,mode=1777",
            "--mount", f"type=bind,src={package},dst=/probe,readonly",
            "--mount", f"type=bind,src={registry},dst=/registry,readonly",
            "--mount", f"type=bind,src={output},dst=/output",
            "--env", "HOME=/tmp", "--env", "CARGO_HOME=/tmp/cargo",
            pinned, "sh", "-c",
            "mkdir -p /tmp/cargo; ln -s /registry /tmp/cargo/registry; "
            "cargo build --offline --locked --release --jobs 1 --manifest-path /probe/Cargo.toml --target-dir /output",
        ], check=False)
        (OUT / "build.log").write_text(build.stdout + build.stderr)
        build.check_returncode()
        binary = output / "release/brsec-session-helper"
        command([
            "docker", "run", "-d", "--name", CONTAINER,
            "--label", "org.biorouter.assessment=security-2026-10",
            "--network", "none", "--read-only", "--user", "10001:10001",
            "--cap-drop", "ALL", "--security-opt", "no-new-privileges",
            "--pids-limit", "32", "--cpus", "0.25", "--memory", "128m", "--memory-swap", "128m",
            "--tmpfs", "/tmp:rw,nosuid,nodev,size=16m,mode=1777",
            "--mount", f"type=bind,src={binary},dst=/guard,readonly",
            pinned, "sleep", "600",
        ])
        rows = []
        try:
            for run in range(1, 4):
                base = f"/tmp/audit-{run}"
                history = f"{base}/sessions/synthetic-history-canary.txt"
                inside(10001, ["sh", "-c", f"umask 022; mkdir -p {base}/sessions; printf SYNTHETIC_HISTORY > {history}"])
                before = inside(10002, ["cat", history], check=False)
                guard_result = inside(10001, ["/guard", f"{base}/sessions"])
                owner = inside(10001, ["cat", history])
                observer = inside(10002, ["cat", history], check=False)
                mode = inside(10001, ["stat", "-c", "%a", f"{base}/sessions"]).stdout.strip()
                row = {"run": run, "owner_uid": 10001, "observer_uid": 10002,
                       "permissive_fixture_readable_before_guard": before.returncode == 0,
                       "guard_exit": guard_result.returncode, "session_mode": mode,
                       "owner_control_allowed": owner.stdout == "SYNTHETIC_HISTORY",
                       "observer_denied": observer.returncode != 0 and "SYNTHETIC_HISTORY" not in observer.stdout,
                       "observer_stderr": observer.stderr.strip()}
                assert row["permissive_fixture_readable_before_guard"] and row["owner_control_allowed"]
                assert row["observer_denied"] and mode == "700"
                rows.append(row)
            inside(10002, ["mkdir", "/tmp/foreign-owned"])
            foreign = inside(10001, ["/guard", "/tmp/foreign-owned"], check=False)
            inside(10001, ["sh", "-c", "mkdir /tmp/target; chmod 755 /tmp/target; ln -s /tmp/target /tmp/session-link"])
            symlink = inside(10001, ["/guard", "/tmp/session-link"], check=False)
            target_mode = inside(10001, ["stat", "-c", "%a", "/tmp/target"]).stdout.strip()
            assert foreign.returncode != 0 and symlink.returncode != 0 and target_mode == "755"
            summary = {"image_id": pinned, "architecture": image["Architecture"],
                       "guard_sha256": hashlib.sha256(guard.encode()).hexdigest(),
                       "extracted_source_sha256": hashlib.sha256(main_source.encode()).hexdigest(),
                       "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
                       "runs": rows, "foreign_owned_directory_rejected": foreign.returncode != 0,
                       "final_symlink_rejected_without_chmod_target": symlink.returncode != 0 and target_mode == "755",
                       "limitation": "Verbatim production Unix helper compiled on Linux; synthetic file fixture, not a rebuilt BioRouter/SQLite end-to-end test."}
            (OUT / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
            print(json.dumps(summary, indent=2))
        finally:
            command(["docker", "rm", "-f", CONTAINER])


if __name__ == "__main__":
    main()
