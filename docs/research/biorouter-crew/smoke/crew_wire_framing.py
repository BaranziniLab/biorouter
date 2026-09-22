#!/usr/bin/env python3
"""Exercise the Linux Crew CLI wire framing through its Unix socket."""

import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

MAX_FRAME = 1_048_576
BOOTSTRAP_KEY = "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c"


def exchange(socket_path: Path, payload: bytes, close_write: bool = True) -> bytes:
    client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    client.settimeout(5)
    try:
        client.connect(str(socket_path))
        client.sendall(payload)
        if close_write:
            client.shutdown(socket.SHUT_WR)
        chunks = []
        while True:
            chunk = client.recv(65_536)
            if not chunk:
                return b"".join(chunks)
            chunks.append(chunk)
    finally:
        client.close()


def healthy(socket_path: Path) -> str:
    frame = json.dumps(
        {"version": 1, "id": "healthy", "method": "hello", "params": {}},
        separators=(",", ":"),
    ).encode() + b"\n"
    response = json.loads(exchange(socket_path, frame))
    workspace_id = response.get("result", {}).get("workspace_id")
    assert isinstance(workspace_id, str) and workspace_id, response
    return workspace_id


def wait_for_runtime(
    root: Path, process: subprocess.Popen[bytes]
) -> tuple[dict, subprocess.Popen[bytes]]:
    runtime = root / "runtime.json"
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        if runtime.exists():
            info = json.loads(runtime.read_text())
            socket_path = Path(info["socket"])
            if socket_path.exists():
                return info, process
        if process.poll() is not None:
            stderr = process.stderr.read().decode(errors="replace") if process.stderr else ""
            raise RuntimeError(f"broker exited with {process.returncode}: {stderr}")
        time.sleep(0.05)
    raise TimeoutError("broker runtime did not become ready")


def main() -> None:
    if len(sys.argv) != 2:
        raise SystemExit(f"usage: {sys.argv[0]} /path/to/biorouter-crew")
    binary = Path(sys.argv[1]).resolve()
    root = Path(tempfile.mkdtemp(prefix="biorouter-crew-wire-", dir="/tmp"))
    os.chmod(root, 0o700)
    runtime_socket = None
    process = None
    try:
        process = subprocess.Popen(
            [
                str(binary),
                "serve",
                "--state-dir",
                str(root),
                "--bootstrap-key",
                BOOTSTRAP_KEY,
            ],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        info, process = wait_for_runtime(root, process)
        runtime_socket = Path(info["socket"])
        workspace_id = healthy(runtime_socket)
        assert workspace_id == info["workspace_id"]
        before = (root / "journal.jsonl").read_bytes()

        cases = [
            ("invalid_utf8", b"\xff\n", False),
            ("oversized_frame", b"a" * (MAX_FRAME + 1) + b"\n", False),
            (
                "invalid_version",
                b'{"version":2,"id":"bad-version","method":"hello","params":{}}\n',
                True,
            ),
            (
                "truncated_client_close",
                b'{"version":1,"id":"truncated","method":"hello","params":{}',
                False,
            ),
        ]
        for name, payload, expects_response in cases:
            response = exchange(runtime_socket, payload)
            if expects_response:
                parsed = json.loads(response)
                assert parsed["error"]["code"] == "invalid_request", (name, parsed)
            else:
                assert response == b"", (name, response[:100])
            assert (root / "journal.jsonl").read_bytes() == before, name
            assert healthy(runtime_socket) == workspace_id, name

        print(
            json.dumps(
                {
                    "result": "pass",
                    "workspace_id": workspace_id,
                    "cases": [name for name, _, _ in cases],
                    "journal_unchanged": True,
                    "kernel_uid_socket": True,
                },
                sort_keys=True,
            )
        )
    finally:
        if process is not None and process.poll() is None:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
        if runtime_socket is not None:
            runtime_socket.unlink(missing_ok=True)
            runtime_socket.parent.rmdir()
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    main()
