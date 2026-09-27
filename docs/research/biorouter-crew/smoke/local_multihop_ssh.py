#!/usr/bin/env python3
"""Exercise a test-owned, localhost-only OpenSSH multi-hop fixture.

The fixture uses three independent sshd processes and one unprivileged local
account. It does not inspect or modify the Crew daemon and cleans up every
process and temporary file it creates.
"""

from __future__ import annotations

import getpass
import hashlib
import json
import os
import pathlib
import shutil
import signal
import subprocess
import tempfile
import time


def run(args: list[str], *, check: bool = False, timeout: int = 15) -> subprocess.CompletedProcess[str]:
    return subprocess.run(args, text=True, capture_output=True, check=check, timeout=timeout)


def main() -> int:
    user = getpass.getuser()
    root = pathlib.Path(tempfile.mkdtemp(prefix="biorouter-crew-multihop-", dir="/private/tmp"))
    pids: list[int] = []
    daemons: dict[str, subprocess.Popen[str]] = {}
    try:
        (root / "keys").mkdir()
        (root / "hosts").mkdir()
        (root / "run").mkdir()
        client_key = root / "keys" / "client"
        keygen = run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(client_key)])
        if keygen.returncode:
            raise RuntimeError(keygen.stderr)
        authorized = root / "authorized_keys"
        authorized.write_text((client_key.with_suffix(".pub")).read_text())
        authorized.chmod(0o600)
        known_hosts = root / "known_hosts"
        ports = {"entry": 57101, "gate_a": 57102, "gate_b": 57103, "target": 57104, "sftp": 57105}

        for name in ports:
            host_key = root / "hosts" / f"{name}_ed25519"
            keygen = run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(host_key)])
            if keygen.returncode:
                raise RuntimeError(keygen.stderr)
            config = root / f"{name}.sshd_config"
            config.write_text(
                "\n".join(
                    [
                        f"Port {ports[name]}",
                        "ListenAddress 127.0.0.1",
                        f"HostKey {host_key}",
                        f"PidFile {root / 'run' / (name + '.pid')}",
                        f"AuthorizedKeysFile {authorized}",
                        f"AllowUsers {user}",
                        "StrictModes no",
                        "UsePAM no",
                        "PasswordAuthentication no",
                        "KbdInteractiveAuthentication no",
                        "PubkeyAuthentication yes",
                        "AllowTcpForwarding no" if name == "target" else "AllowTcpForwarding yes",
                        "ForceCommand internal-sftp" if name == "sftp" else "",
                        "Subsystem sftp internal-sftp" if name == "sftp" else "",
                        "GatewayPorts no",
                        "X11Forwarding no",
                        "PermitTunnel no",
                        "LogLevel ERROR",
                        "UsePrivilegeSeparation no",
                    ]
                )
                + "\n"
            )
            daemon = subprocess.Popen(
                ["/usr/sbin/sshd", "-D", "-e", "-f", str(config)],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            pids.append(daemon.pid)
            daemons[name] = daemon

        time.sleep(0.5)
        for name, port in ports.items():
            scan = run(["ssh-keyscan", "-p", str(port), "127.0.0.1"])
            if scan.returncode:
                raise RuntimeError(f"keyscan {name}: {scan.stderr}")
            expected = (root / "hosts" / f"{name}_ed25519.pub").read_text().split()
            scanned = [line.split() for line in scan.stdout.splitlines() if line and not line.startswith("#")]
            if not any(len(fields) >= 3 and fields[1] == expected[0] and fields[2] == expected[1] for fields in scanned):
                raise RuntimeError(f"keyscan {name}: host key did not match generated fixture key")
            matching = next(fields for fields in scanned if fields[1] == expected[0] and fields[2] == expected[1])
            known_hosts.write_text((known_hosts.read_text() if known_hosts.exists() else "") + " ".join(matching) + "\n")
        known_hosts.chmod(0o600)

        config = root / "ssh_config"
        common = [
            "Host entry gate-a gate-b target sftp",
            f"  User {user}",
            f"  IdentityFile {client_key}",
            "  IdentitiesOnly yes",
            "  IdentityAgent none",
            "  BatchMode yes",
            "  StrictHostKeyChecking yes",
            f"  UserKnownHostsFile {known_hosts}",
            "  ForwardAgent no",
            "  ForwardX11 no",
            "  ControlMaster no",
            "  ControlPath none",
            "  LogLevel ERROR",
            "  ConnectionAttempts 1",
            "  ConnectTimeout 3",
            "Host entry",
            "  HostName 127.0.0.1",
            f"  Port {ports['entry']}",
            "Host gate-a",
            "  HostName 127.0.0.1",
            f"  Port {ports['gate_a']}",
            "Host gate-b",
            "  HostName 127.0.0.1",
            f"  Port {ports['gate_b']}",
            "Host target",
            "  HostName 127.0.0.1",
            f"  Port {ports['target']}",
            "  ProxyJump gate-a,gate-b",
            "Host sftp",
            "  HostName 127.0.0.1",
            f"  Port {ports['sftp']}",
        ]
        config.write_text("\n".join(common) + "\n")
        direct = run(["ssh", "-F", str(config), "-o", "ProxyJump=none", "target", "id -u; uname -srm"])
        multi = run(["ssh", "-F", str(config), "target", "printf '%s\\n' multihop-ok"])
        forwarding_disabled_exec = run(["ssh", "-F", str(config), "target", "printf '%s\\n' forwarding-disabled-exec"])
        sftp_only = run(["ssh", "-F", str(config), "sftp", "printf should-not-run"])
        active = subprocess.Popen(
            ["ssh", "-F", str(config), "target", "sleep 30; printf should-not-run"],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        time.sleep(0.4)
        active.terminate()
        active_stdout, active_stderr = active.communicate(timeout=3)

        gate_a_key = root / "hosts" / "gate_a_ed25519"
        gate_a_original = root / "hosts" / "gate_a_original_ed25519"
        gate_a_original.write_bytes(gate_a_key.read_bytes())
        gate_a_original.with_suffix(".pub").write_bytes(gate_a_key.with_suffix(".pub").read_bytes())
        gate_a_replacement = root / "hosts" / "gate_a_replacement_ed25519"
        keygen = run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(gate_a_replacement)])
        if keygen.returncode:
            raise RuntimeError(keygen.stderr)
        gate_a_key.write_bytes(gate_a_replacement.read_bytes())
        gate_a_key.with_suffix(".pub").write_bytes(gate_a_replacement.with_suffix(".pub").read_bytes())
        daemons["gate_a"].terminate()
        daemons["gate_a"].wait(timeout=3)
        daemons["gate_a"] = subprocess.Popen(
            ["/usr/sbin/sshd", "-D", "-e", "-f", str(root / "gate_a.sshd_config")],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        pids.append(daemons["gate_a"].pid)
        time.sleep(0.3)
        changed_gateway = run(["ssh", "-F", str(config), "target", "printf should-not-run"])
        changed_gateway_text = changed_gateway.stderr + changed_gateway.stdout

        gate_a_key.write_bytes(gate_a_original.read_bytes())
        gate_a_key.with_suffix(".pub").write_bytes(gate_a_original.with_suffix(".pub").read_bytes())
        daemons["gate_a"].terminate()
        daemons["gate_a"].wait(timeout=3)
        daemons["gate_a"] = subprocess.Popen(
            ["/usr/sbin/sshd", "-D", "-e", "-f", str(root / "gate_a.sshd_config")],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        pids.append(daemons["gate_a"].pid)
        time.sleep(0.3)

        target_key = root / "hosts" / "target_ed25519"
        replacement = root / "hosts" / "replacement_ed25519"
        keygen = run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-f", str(replacement)])
        if keygen.returncode:
            raise RuntimeError(keygen.stderr)
        target_key.write_bytes(replacement.read_bytes())
        target_key.with_suffix(".pub").write_bytes(replacement.with_suffix(".pub").read_bytes())
        daemons["target"].terminate()
        daemons["target"].wait(timeout=3)
        target_daemon = subprocess.Popen(
            ["/usr/sbin/sshd", "-D", "-e", "-f", str(root / "target.sshd_config")],
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        pids.append(target_daemon.pid)
        daemons["target"] = target_daemon
        time.sleep(0.3)
        changed_host = run(["ssh", "-F", str(config), "target", "printf should-not-run"])
        changed_text = changed_host.stderr + changed_host.stdout

        cancelled = run(
            [
                "ssh",
                "-F",
                str(config),
                "-o",
                "ConnectTimeout=1",
                "-p",
                "57999",
                "127.0.0.1",
                "printf should-not-run",
            ],
            timeout=5,
        )
        result = {
            "fixture": "test-owned localhost OpenSSH",
            "user": user,
            "ports": ports,
            "host_key_sha256": {
                name: hashlib.sha256((root / "hosts" / f"{name}_ed25519").read_bytes()).hexdigest()
                for name in ports
            },
            "direct_target": {"returncode": direct.returncode, "stdout": direct.stdout, "stderr": direct.stderr},
            "proxyjump_target": {"returncode": multi.returncode, "stdout": multi.stdout, "stderr": multi.stderr},
            "forwarding_disabled_exec": {
                "returncode": forwarding_disabled_exec.returncode,
                "approved_exec": forwarding_disabled_exec.returncode == 0 and forwarding_disabled_exec.stdout == "forwarding-disabled-exec\n",
                "stdout": forwarding_disabled_exec.stdout,
                "stderr": forwarding_disabled_exec.stderr,
            },
            "forced_sftp_only": {
                "returncode": sftp_only.returncode,
                "incompatible": sftp_only.returncode != 0 and "this service allows sftp connections only" in (sftp_only.stdout + sftp_only.stderr).lower() and "should-not-run" not in sftp_only.stdout,
                "stdout": sftp_only.stdout,
                "stderr": sftp_only.stderr,
            },
            "changed_host_key": {
                "returncode": changed_host.returncode,
                "rejected": changed_host.returncode != 0 and "REMOTE HOST IDENTIFICATION HAS CHANGED" in changed_text,
                "stdout": changed_host.stdout,
                "stderr": changed_host.stderr,
            },
            "changed_gateway_host_key": {
                "returncode": changed_gateway.returncode,
                "rejected": changed_gateway.returncode != 0 and "REMOTE HOST IDENTIFICATION HAS CHANGED" in changed_gateway_text,
                "stdout": changed_gateway.stdout,
                "stderr": changed_gateway.stderr,
            },
            "cancelled_unreachable": {
                "returncode": cancelled.returncode,
                "side_effect_free": "should-not-run" not in cancelled.stdout,
                "stderr": cancelled.stderr,
            },
            "cancelled_active": {
                "returncode": active.returncode,
                "side_effect_free": "should-not-run" not in active_stdout,
                "stdout": active_stdout,
                "stderr": active_stderr,
            },
            "keyboard_interactive": {
                "tested": False,
                "reason": "macOS sshd fixture uses UsePAM=no; no synthetic prompt provider was installed",
            },
        }
        print(json.dumps(result, indent=2, sort_keys=True))
        if direct.returncode or multi.returncode or not result["forwarding_disabled_exec"]["approved_exec"] or not result["forced_sftp_only"]["incompatible"] or not result["changed_host_key"]["rejected"] or not result["changed_gateway_host_key"]["rejected"] or not result["cancelled_unreachable"]["side_effect_free"] or not result["cancelled_active"]["side_effect_free"]:
            return 1
        return 0
    finally:
        for pid in pids:
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        for pid in pids:
            try:
                os.waitpid(pid, 0)
            except ChildProcessError:
                pass
        shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    raise SystemExit(main())
