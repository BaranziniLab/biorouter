#!/usr/bin/env python3
"""Safely tear down a state file produced by provision_three_user_fixture.py."""
from __future__ import annotations
import json
from pathlib import Path
import subprocess
import sys
import time


def aws(region: str, *args: str, check: bool = True) -> dict:
    result = subprocess.run(["aws", "--region", region, *args, "--output", "json"], capture_output=True, timeout=60)
    if check and result.returncode:
        raise RuntimeError(result.stderr.decode(errors="replace"))
    if not check:
        payload = {"returncode": result.returncode, "stderr": result.stderr.decode()}
        if result.returncode == 0 and result.stdout.strip():
            decoded = json.loads(result.stdout)
            if isinstance(decoded, dict):
                payload.update(decoded)
        return payload
    return json.loads(result.stdout) if result.stdout.strip() else {}


def wait_not_found(region: str, args: tuple[str, ...], marker: str, label: str) -> None:
    deadline = time.monotonic() + 180
    while time.monotonic() < deadline:
        result = aws(region, *args, check=False)
        if result.get("returncode") and marker in result.get("stderr", ""):
            return
        time.sleep(5)
    raise RuntimeError(f"{label} deletion was not confirmed")


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: cleanup_fixture.py /tmp/.../fixture-state.json")
    state_path = Path(sys.argv[1]).resolve()
    state = json.loads(state_path.read_text())
    region = state["region"]
    resources = state["resources"]
    cleanup = state.setdefault("cleanup", {})
    instance = resources.get("instance")
    try:
        if instance:
            aws(region, "ec2", "terminate-instances", "--instance-ids", instance)
            deadline = time.monotonic() + 300
            while time.monotonic() < deadline:
                data = aws(region, "ec2", "describe-instances", "--instance-ids", instance, check=False)
                if data.get("returncode") and "InvalidInstanceID.NotFound" in data.get("stderr", ""):
                    cleanup["instance"] = "deleted"
                    break
                if data.get("Reservations", [{}])[0].get("Instances", [{}])[0].get("State", {}).get("Name") == "terminated":
                    cleanup["instance"] = "terminated"
                    break
                time.sleep(5)
            if cleanup.get("instance") not in {"terminated", "deleted"}:
                raise RuntimeError("instance termination was not confirmed")
            for volume in resources.get("volumes", []):
                deadline = time.monotonic() + 180
                while time.monotonic() < deadline:
                    result = aws(region, "ec2", "describe-volumes", "--volume-ids", volume, check=False)
                    if result.get("returncode") and "InvalidVolume.NotFound" in result.get("stderr", ""):
                        cleanup[volume] = "deleted"
                        break
                    time.sleep(5)
                if cleanup.get(volume) != "deleted":
                    raise RuntimeError(f"volume cleanup was not confirmed: {volume}")
        if resources.get("security_group"):
            group = resources["security_group"]
            aws(region, "ec2", "delete-security-group", "--group-id", group)
            wait_not_found(
                region,
                ("ec2", "describe-security-groups", "--group-ids", group),
                "InvalidGroup.NotFound",
                f"security group {group}",
            )
            cleanup["security_group"] = "deleted"
        if resources.get("bootstrap_key_pair"):
            key_name = resources["bootstrap_key_pair"]
            aws(region, "ec2", "delete-key-pair", "--key-name", key_name)
            wait_not_found(
                region,
                ("ec2", "describe-key-pairs", "--key-names", key_name),
                "InvalidKeyPair.NotFound",
                f"key pair {key_name}",
            )
            cleanup["bootstrap_key_pair"] = "deleted"
        cleanup["status"] = "verified"
        for key in [state.get("bootstrap_key_path"), state.get("host_key", {}).get("known_hosts_path"), state.get("connection_config")]:
            if key:
                Path(key).unlink(missing_ok=True)
        for account in state.get("accounts", []):
            for key in (account.get("ssh_key_path"), account.get("ssh_key_path", "") + ".pub", account.get("connection_config")):
                if key:
                    Path(key).unlink(missing_ok=True)
        state_path.write_text(json.dumps(state, indent=2) + "\n")
        Path(state["evidence"]).write_text(json.dumps(state, indent=2) + "\n")
        print(json.dumps(cleanup, indent=2))
        return 0
    except BaseException as error:
        cleanup["status"] = "failed"
        cleanup["error"] = str(error)
        state_path.write_text(json.dumps(state, indent=2) + "\n")
        Path(state["evidence"]).write_text(json.dumps(state, indent=2) + "\n")
        print("FAILED: " + str(error), file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
