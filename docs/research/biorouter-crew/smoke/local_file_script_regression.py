#!/usr/bin/env python3
"""Run the Crew file-script/seccomp regression in a disposable Linux fixture.

The fixture is intentionally separate from the three-user acceptance container.
Use ``--expect-file-script deny`` with the pre-fix Linux artifact to retain the
original CPython FIOCLEX failure, then use ``allow`` with the guarded artifact.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import pathlib
import select
import subprocess
import time
import uuid
from typing import Any

from cryptography.hazmat.primitives import serialization
from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey


FIXTURE = pathlib.Path(__file__).with_name("fixtures") / "summarize_csv.py"
PRIVATE_KEY = Ed25519PrivateKey.from_private_bytes(bytes([7]) * 32)
PUBLIC_KEY = PRIVATE_KEY.public_key().public_bytes(
    encoding=serialization.Encoding.Raw,
    format=serialization.PublicFormat.Raw,
).hex()
OWNER = "alice"
OWNER_UID = 1101
WORKSPACE = "b104bab2-9463-49de-bbd7-3c0c22d0ed59"
INPUT_CSV = b"sample_id,value\nA,10\nB,20\nC,30\n"
EXPECTED_OUTPUT = b"row_count,total\n3,60\n"


def docker_exec(docker: str, container: str, *args: str, input_data: bytes | None = None) -> str:
    result = subprocess.run(
        [docker, "exec", "-i", container, *args],
        input=input_data,
        capture_output=True,
        check=True,
        timeout=20,
    )
    return result.stdout.decode()


def docker_owner_exec(
    docker: str, container: str, *args: str, input_data: bytes | None = None
) -> str:
    result = subprocess.run(
        [
            docker,
            "exec",
            "-i",
            "-u",
            OWNER,
            "-e",
            "HOME=/home/alice",
            container,
            *args,
        ],
        input=input_data,
        capture_output=True,
        check=True,
        timeout=20,
    )
    return result.stdout.decode()


def wait_for_runtime(docker: str, container: str, path: str) -> dict[str, Any]:
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        try:
            return json.loads(docker_exec(docker, container, "cat", path))
        except (subprocess.CalledProcessError, json.JSONDecodeError):
            time.sleep(0.1)
    raise TimeoutError(f"runtime metadata did not appear: {path}")


def canonical(value: Any) -> Any:
    if isinstance(value, dict):
        return {key: canonical(value[key]) for key in sorted(value)}
    if isinstance(value, list):
        return [canonical(item) for item in value]
    return value


def signed_payload(workspace: str, nonce: str, method: str, params: dict[str, Any]) -> bytes:
    return json.dumps(
        [workspace, OWNER_UID, nonce, method, canonical(params)],
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode()


def request(proc: subprocess.Popen[str], value: dict[str, Any]) -> dict[str, Any]:
    assert proc.stdin is not None and proc.stdout is not None
    proc.stdin.write(json.dumps(value, separators=(",", ":")) + "\n")
    proc.stdin.flush()
    if not select.select([proc.stdout], [], [], 15)[0]:
        raise TimeoutError(f"bridge response timed out while sending {value['id']}")
    line = proc.stdout.readline()
    if not line:
        raise RuntimeError(f"bridge closed while sending {value['id']}")
    return json.loads(line)


def signed(
    proc: subprocess.Popen[str], number: int, workspace: str, method: str, params: dict[str, Any]
) -> dict[str, Any]:
    challenge = request(
        proc,
        {
            "version": 1,
            "id": f"challenge-{number}",
            "method": "auth.challenge",
            "params": {"device_id": hashlib.sha256(bytes.fromhex(PUBLIC_KEY)).hexdigest()},
        },
    )
    nonce = challenge["result"]["nonce"]
    auth = {
        "device_id": hashlib.sha256(bytes.fromhex(PUBLIC_KEY)).hexdigest(),
        "nonce": nonce,
        "signature": PRIVATE_KEY.sign(signed_payload(workspace, nonce, method, params)).hex(),
    }
    return request(
        proc,
        {
            "version": 1,
            "id": f"request-{number}",
            "method": method,
            "params": params,
            "auth": auth,
        },
    )


def remote(
    proc: subprocess.Popen[str], number: int, method: str, params: dict[str, Any], credential: str
) -> dict[str, Any]:
    return request(
        proc,
        {
            "version": 1,
            "id": f"remote-{number}",
            "method": method,
            "params": params,
            "credential": credential,
        },
    )


def wait_for_job(
    proc: subprocess.Popen[str], job_id: str, credential: str, first_number: int
) -> dict[str, Any]:
    deadline = time.monotonic() + 15
    for offset in range(100):
        if time.monotonic() >= deadline:
            break
        status = remote(proc, first_number + offset, "remote.job_status", {"job_id": job_id}, credential)
        current = status.get("result", {})
        if current.get("status") not in {"running", "starting"}:
            return current
        time.sleep(min(0.1, max(0, deadline - time.monotonic())))
    raise RuntimeError(f"job did not reach a terminal state: {job_id}")


def require_failed(status: dict[str, Any], label: str, *markers: str) -> dict[str, Any]:
    if status.get("status") != "failed":
        raise AssertionError(f"{label} unexpectedly succeeded: {status}")
    stderr = str(status.get("stderr", ""))
    if markers and not any(marker in stderr for marker in markers):
        raise AssertionError(f"{label} lacked refusal marker {markers}: {status}")
    return status


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=pathlib.Path)
    parser.add_argument("--container", default="biorouter-crew-regression-luna")
    parser.add_argument("--docker", default="/usr/local/bin/docker")
    parser.add_argument("--expect-file-script", choices=("deny", "allow"), required=True)
    args = parser.parse_args()
    if not args.binary.is_file() or not args.binary.stat().st_mode & 0o111:
        raise RuntimeError(f"executable not found: {args.binary}")

    suffix = uuid.uuid4().hex[:12]
    state = f"/home/alice/.local/share/biorouter-crew/file-script-regression-{suffix}"
    work = f"/home/alice/work/file-script-regression-{suffix}"
    binary = f"/tmp/biorouter-crew-file-script-{suffix}"
    socket_path = None
    process: subprocess.Popen[str] | None = None
    result: dict[str, Any] = {
        "fixture_sha256": hashlib.sha256(FIXTURE.read_bytes()).hexdigest(),
        "binary": str(args.binary),
        "expect_file_script": args.expect_file_script,
        "container": args.container,
    }
    try:
        docker_exec(
            args.docker,
            args.container,
            "install",
            "-d",
            "-m",
            "700",
            "-o",
            OWNER,
            "-g",
            OWNER,
            "/home/alice/.local/share/biorouter-crew",
            "/home/alice/.local/state",
            "/home/alice/work",
        )
        subprocess.run([args.docker, "cp", str(args.binary), f"{args.container}:{binary}"], check=True)
        docker_exec(args.docker, args.container, "chown", "alice:alice", binary)
        docker_exec(args.docker, args.container, "chmod", "755", binary)
        docker_owner_exec(
            args.docker,
            args.container,
            "sh",
            "-lc",
            f"mkdir -p {state} {work}; chmod 700 {state} {work}; {binary} start --state-dir {state} --bootstrap-key {PUBLIC_KEY}",
        )
        runtime = wait_for_runtime(args.docker, args.container, f"{state}/runtime.json")
        socket_path = runtime["socket"]
        docker_exec(args.docker, args.container, "mkdir", "-p", work)
        subprocess.run(
            [args.docker, "cp", str(FIXTURE), f"{args.container}:{work}/summarize_csv.py"],
            check=True,
        )
        docker_exec(args.docker, args.container, "chown", "alice:alice", f"{work}/summarize_csv.py")
        docker_exec(args.docker, args.container, "chmod", "755", f"{work}/summarize_csv.py")
        docker_owner_exec(
            args.docker,
            args.container,
            "sh",
            "-lc",
            f"cat > {work}/crew-task.csv",
            input_data=INPUT_CSV,
        )
        actual_input = docker_exec(args.docker, args.container, "cat", f"{work}/crew-task.csv").encode()
        if actual_input != INPUT_CSV:
            raise AssertionError(f"fixture CSV bytes were not preserved: {actual_input!r}")
        outside_file = f"/tmp/crew-file-script-outside-{suffix}.txt"
        docker_exec(args.docker, args.container, "sh", "-lc", f"printf '%s' outside-secret > {outside_file}; chmod 644 {outside_file}")
        process = subprocess.Popen(
            [
                args.docker,
                "exec",
                "-i",
                "-u",
                OWNER,
                "-e",
                "HOME=/home/alice",
                args.container,
                binary,
                "bridge",
                "--stdio",
                "--socket",
                socket_path,
                "--owner-uid",
                str(runtime["host_uid"]),
                "--workspace-id",
                runtime["workspace_id"],
            ],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            bufsize=1,
        )
        bootstrap = signed(process, 1, runtime["workspace_id"], "auth.bootstrap", {"public_key": PUBLIC_KEY})
        if "error" in bootstrap:
            raise RuntimeError(f"bootstrap refused: {bootstrap}")
        team = signed(
            process,
            2,
            runtime["workspace_id"],
            "team.create",
            {"name": "file-script-regression", "idempotency_key": "file-script-team"},
        )
        channel = team["result"]["channel"]["id"]
        created = signed(
            process,
            3,
            runtime["workspace_id"],
            "run.create",
            {
                "channel_id": channel,
                "source_channels": [channel],
                "provider_policy_id": "private",
                "personal_mode": "private",
                "public_provider": False,
                "remote_root": work,
                "remote_execution": True,
                "expires_in": 300,
                "idempotency_key": "file-script-run",
            },
        )
        credential = created["result"]["credential"]
        file_job = remote(
            process,
            4,
            "remote.execute",
            {
                "argv": ["python3", "summarize_csv.py", "crew-task.csv", "summary.csv"],
                "timeout_seconds": 20,
                "idempotency_key": "file-script-positive",
            },
            credential,
        )
        if "result" not in file_job:
            raise RuntimeError(f"file script request was refused before job creation: {file_job}")
        file_status = wait_for_job(process, file_job["result"]["job_id"], credential, 100)
        result["file_script"] = file_status
        if args.expect_file_script == "deny":
            require_failed(file_status, "file script", "Operation not permitted")
        else:
            if file_status.get("status") != "completed" or "row_count=3 total=60" not in file_status.get("stdout", ""):
                raise AssertionError(f"file script did not complete with expected summary: {file_status}")
            summary = remote(process, 200, "remote.read", {"path": "summary.csv"}, credential)
            data = bytes.fromhex(summary["result"]["data_hex"])
            if data != EXPECTED_OUTPUT:
                raise AssertionError(f"unexpected summary bytes: {data!r}")
            result["summary_sha256"] = hashlib.sha256(data).hexdigest()

        for number, path in ((201, "/etc/passwd"), (202, "../.ssh")):
            response = remote(process, number, "remote.read", {"path": path}, credential)
            if "error" not in response:
                raise AssertionError(f"outside path was accepted: {path}: {response}")
        result["outside_paths"] = "refused"

        negatives = {
            "network": ["python3", "-c", "import socket; socket.create_connection(('127.0.0.1', 9), 1)"],
            "subprocess": ["python3", "-c", "import subprocess; subprocess.run(['/bin/echo', 'forbidden'], check=True)"],
            "other_ioctl": ["python3", "-c", "import fcntl; fcntl.ioctl(1, 0x12345678, 0)"],
            # Linux FIONCLEX is 0x5450; Python's fcntl module does not expose it on all builds.
            "fionclex": ["python3", "-c", "import fcntl; fcntl.ioctl(1, 0x5450)"],
            "outside_file": ["python3", "-c", f"open({outside_file!r}, encoding='utf-8').read()"],
        }
        negative_statuses: dict[str, dict[str, Any]] = {}
        for number, (label, argv) in enumerate(negatives.items(), start=300):
            started = remote(
                process,
                number,
                "remote.execute",
                {"argv": argv, "timeout_seconds": 10, "idempotency_key": f"negative-{label}"},
                credential,
            )
            negative_statuses[label] = require_failed(
                wait_for_job(process, started["result"]["job_id"], credential, number + 100),
                label,
                "Operation not permitted",
                "Permission denied",
            )
        result["negative_statuses"] = negative_statuses
        print(json.dumps(result, sort_keys=True))
        return 0
    finally:
        if process is not None:
            if process.stdin:
                process.stdin.close()
            process.wait(timeout=15)
        if socket_path:
            docker_owner_exec(args.docker, args.container, binary, "stop", "--state-dir", state)
        docker_exec(args.docker, args.container, "rm", "-rf", state, work, binary, outside_file if 'outside_file' in locals() else "/nonexistent")


if __name__ == "__main__":
    raise SystemExit(main())
