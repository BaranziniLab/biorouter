#!/usr/bin/env python3
"""Synthetic, stdlib-only Linux primitives probe; this is not a Crew server.

Run via `ssh -T host python3 - < host_probe.py`. All writes are confined to a
new private temporary directory and removed before exit. No home files read.
"""

import hashlib
import json
import os
import pathlib
import platform
import pwd
import socket
import stat
import struct
import subprocess
import tempfile
import threading


def filesystem(path):
    return subprocess.check_output(
        ["stat", "-f", "-c", "%T", str(path)], universal_newlines=True
    ).strip()


def main():
    os.umask(0o077)
    results = {
        "scope": "synthetic primitives only, one authenticated Unix account",
        "user": pwd.getpwuid(os.getuid()).pw_name,
        "uid": os.getuid(),
        "platform": platform.system(),
        "python": platform.python_version(),
        "home_filesystem": filesystem(os.path.expanduser("~")),
        "checks": {},
    }
    checks = results["checks"]
    with tempfile.TemporaryDirectory(prefix="biorouter-crew-probe-") as name:
        root = pathlib.Path(name)
        results["fixture_filesystem"] = filesystem(root)
        checks["private_fixture_mode_0700"] = stat.S_IMODE(root.stat().st_mode) == 0o700

        journal = root / "events.jsonl"
        events = [
            {"seq": n, "event_id": "synthetic-%s" % n, "type": "message",
             "body": "Crew synthetic line %s; unicode: \u03b1\u03b2" % n}
            for n in range(1, 4)
        ]
        with journal.open("wb") as handle:
            for event in events:
                handle.write((json.dumps(event, ensure_ascii=False) + "\n").encode())
            handle.flush()
            os.fsync(handle.fileno())
        with journal.open("rb") as handle:
            restored = [json.loads(line) for line in handle]
        checks["jsonl_fsync_reopen_exact"] = restored == events
        checks["resume_after_sequence_two"] = [e["seq"] for e in restored if e["seq"] > 2] == [3]

        # Simulate only a partial last write. Interior corruption must not be skipped.
        with journal.open("ab") as handle:
            handle.write(b'{"seq":4,"body":"unfinished')
            handle.flush()
            os.fsync(handle.fileno())
        raw = journal.read_bytes()
        committed_end = raw.rfind(b"\n") + 1
        committed = [json.loads(line) for line in raw[:committed_end].splitlines()]
        checks["torn_tail_detected_without_losing_complete_records"] = (
            committed == events and committed_end < len(raw)
        )
        with journal.open("r+b") as handle:
            handle.truncate(committed_end)
            handle.flush()
            os.fsync(handle.fileno())
        checks["torn_tail_recovery_reopens"] = [json.loads(x) for x in journal.read_bytes().splitlines()] == events

        attachment = bytes(range(256)) * 4096
        staging = root / "blob.part"
        staging.write_bytes(attachment)
        with staging.open("rb") as handle:
            os.fsync(handle.fileno())
        complete = root / "blob.bin"
        os.replace(str(staging), str(complete))
        directory_fd = os.open(str(root), os.O_RDONLY | os.O_DIRECTORY)
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
        checks["binary_blob_1mib_atomic_publish_hash"] = hashlib.sha256(complete.read_bytes()).digest() == hashlib.sha256(attachment).digest()
        checks["fixture_files_mode_0600"] = all(stat.S_IMODE(p.stat().st_mode) == 0o600 for p in [journal, complete])

        listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        listener.settimeout(5)
        socket_path = str(root / "crew.sock")
        listener.bind(socket_path)
        listener.listen(1)
        observed = {}

        def serve():
            conn, _ = listener.accept()
            with conn:
                conn.settimeout(5)
                pid, uid, gid = struct.unpack("3i", conn.getsockopt(socket.SOL_SOCKET, socket.SO_PEERCRED, struct.calcsize("3i")))
                with conn.makefile("rb") as stream:
                    claim = json.loads(stream.readline(4096))
                actual = pwd.getpwuid(uid).pw_name
                observed.update(uid=uid, gid=gid, authenticated_user=actual)
                conn.sendall((json.dumps({"uid": uid, "claim_matches_authenticated_user": claim["claimed_user"] == actual}) + "\n").encode())

        thread = threading.Thread(target=serve)
        thread.start()
        client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        client.settimeout(5)
        with client:
            client.connect(socket_path)
            client.sendall(b'{"claimed_user":"different-synthetic-user"}\n')
            with client.makefile("rb") as stream:
                response = json.loads(stream.readline(4096))
        thread.join(6)
        listener.close()
        checks["uds_kernel_uid_matches_ssh_account"] = observed.get("uid") == os.getuid()
        checks["claimed_name_differs_from_kernel_identity"] = response.get("claim_matches_authenticated_user") is False
        checks["uds_thread_completed"] = not thread.is_alive()
    checks["temporary_fixture_removed"] = not os.path.exists(name)
    results["passed"] = all(checks.values())
    print(json.dumps(results, sort_keys=True))
    return 0 if results["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
