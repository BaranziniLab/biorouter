#!/usr/bin/env python3
"""Run a bounded real-UID Unix-socket soak in a disposable Linux workspace.

This harness is intentionally separate from the three-user QA fixture. It
creates synthetic accounts only when invoked as root, starts a rootless broker
for one owner, and keeps a bounded number of hello connections per profile.
It does not use AWS, private keys, or the production user's home directory.
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import signal
import socket
import subprocess
import sys
import tempfile
import time


BOOTSTRAP_KEY = "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c"


def user_spec(index: int) -> tuple[str, int]:
    return f"crewsoak{index:03d}", 12000 + index


def run_root(*args: str) -> None:
    subprocess.run(list(args), check=True)


def ensure_accounts(owner: str, owner_uid: int, count: int) -> list[tuple[str, int]]:
    if os.geteuid() != 0:
        raise RuntimeError("run the Linux soak setup as container root")
    run_root("groupadd", "--gid", str(owner_uid), owner)
    run_root(
        "useradd",
        "--uid",
        str(owner_uid),
        "--gid",
        str(owner_uid),
        "--create-home",
        "--shell",
        "/bin/sh",
        owner,
    )
    profiles = []
    for index in range(1, count + 1):
        name, uid = user_spec(index)
        run_root("groupadd", "--gid", str(uid), name)
        run_root(
            "useradd",
            "--uid",
            str(uid),
            "--gid",
            str(uid),
            "--create-home",
            "--shell",
            "/bin/sh",
            name,
        )
        profiles.append((name, uid))
    return profiles


def profile_main(socket_path: str, connections: int, duration: int, marker: str) -> int:
    sockets: list[socket.socket] = []
    try:
        for _ in range(connections):
            stream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
            stream.settimeout(5)
            stream.connect(socket_path)
            stream.sendall(b'{"version":1,"id":"soak-hello","method":"hello","params":{}}\n')
            response = stream.recv(1_048_576)
            if not response:
                raise RuntimeError("broker closed hello connection")
            sockets.append(stream)
        pathlib.Path(marker).write_text(json.dumps({"uid": os.geteuid(), "connections": len(sockets)}))
        deadline = time.monotonic() + duration
        while time.monotonic() < deadline:
            time.sleep(5)
        return 0
    except Exception as exc:
        pathlib.Path(marker).write_text(json.dumps({"uid": os.geteuid(), "error": str(exc)}))
        return 1
    finally:
        for stream in sockets:
            stream.close()


def parse_runtime(state_dir: pathlib.Path) -> dict[str, object]:
    runtime = state_dir / "runtime.json"
    deadline = time.monotonic() + 15
    while time.monotonic() < deadline:
        try:
            data = json.loads(runtime.read_text())
            if data.get("socket") and data.get("workspace_id"):
                return data
        except (FileNotFoundError, json.JSONDecodeError):
            pass
        time.sleep(0.25)
    raise RuntimeError("broker runtime metadata did not become ready")


def soak(args: argparse.Namespace) -> int:
    if sys.platform != "linux":
        raise RuntimeError("real-UID broker soak requires Linux")
    if args.profiles < 1 or args.profiles > 50:
        raise RuntimeError("profiles must be between 1 and 50")
    if args.connections_per_user < 1 or args.connections_per_user > 4:
        raise RuntimeError("connections-per-user must be between 1 and 4")
    if args.connections_per_user * args.profiles > 200:
        raise RuntimeError("keep total held connections below the 256 global ceiling")
    binary = pathlib.Path(args.binary).resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise RuntimeError(f"executable not found: {binary}")
    owner = "crewsoakowner"
    owner_uid = 11900
    profiles = ensure_accounts(owner, owner_uid, args.profiles)
    owner_home = pathlib.Path("/home") / owner
    state_dir = pathlib.Path(args.state_dir).resolve()
    state_dir.mkdir(mode=0o700, parents=True, exist_ok=False)
    os.chown(state_dir, owner_uid, owner_uid)
    bootstrap = subprocess.Popen(
        [
            "runuser",
            "-u",
            owner,
            "--",
            "env",
            f"HOME={owner_home}",
            str(binary),
            "start",
            "--state-dir",
            str(state_dir),
            "--bootstrap-key",
            BOOTSTRAP_KEY,
        ],
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    if bootstrap.wait(timeout=15) != 0:
        raise RuntimeError(bootstrap.stderr.read())
    runtime = parse_runtime(state_dir)
    socket_path = str(runtime["socket"])
    marker_dir = pathlib.Path(tempfile.mkdtemp(prefix="crew-soak-markers-"))
    os.chmod(marker_dir, 0o777)
    children: list[subprocess.Popen[bytes]] = []
    try:
        for name, _uid in profiles:
            marker = marker_dir / f"{name}.json"
            child = subprocess.Popen(
                [
                    "runuser",
                    "-u",
                    name,
                    "--",
                    sys.executable,
                    __file__,
                    "--profile",
                    "--socket",
                    socket_path,
                    "--connections",
                    str(args.connections_per_user),
                    "--duration",
                    str(args.duration_seconds),
                    "--marker",
                    str(marker),
                ],
                stdout=subprocess.DEVNULL,
                stderr=subprocess.DEVNULL,
            )
            children.append(child)
        ready_deadline = time.monotonic() + min(args.duration_seconds, 60)
        while time.monotonic() < ready_deadline and len(list(marker_dir.glob("*.json"))) < len(profiles):
            time.sleep(0.25)
        markers = [json.loads(path.read_text()) for path in sorted(marker_dir.glob("*.json"))]
        failures = [marker for marker in markers if "error" in marker]
        print(json.dumps({"profiles": len(profiles), "connections_per_user": args.connections_per_user, "ready": len(markers) - len(failures), "failures": failures, "workspace_id": runtime["workspace_id"], "duration_seconds": args.duration_seconds}, sort_keys=True))
        if failures or len(markers) != len(profiles):
            return 1
        deadline = time.monotonic() + args.duration_seconds
        while time.monotonic() < deadline:
            if any(child.poll() is not None for child in children):
                return 1
            time.sleep(5)
        return 0
    finally:
        for child in children:
            child.send_signal(signal.SIGTERM)
        for child in children:
            child.wait(timeout=10)
        subprocess.run(
            ["runuser", "-u", owner, "--", "env", f"HOME={owner_home}", str(binary), "stop", "--state-dir", str(state_dir)],
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default="/usr/local/bin/biorouter-crew")
    parser.add_argument("--state-dir", default="/tmp/crew-soak-state")
    parser.add_argument("--profiles", type=int, default=50)
    parser.add_argument("--connections-per-user", type=int, default=4)
    parser.add_argument("--duration-seconds", type=int, default=1800)
    parser.add_argument("--profile", action="store_true")
    parser.add_argument("--socket")
    parser.add_argument("--connections", type=int)
    parser.add_argument("--duration", type=int)
    parser.add_argument("--marker")
    args = parser.parse_args()
    if args.profile:
        if not all((args.socket, args.connections, args.duration, args.marker)):
            parser.error("profile mode requires socket, connections, duration, and marker")
        return profile_main(args.socket, args.connections, args.duration, args.marker)
    try:
        return soak(args)
    except Exception as exc:
        print(f"soak setup failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
