#!/usr/bin/env python3
"""Provision a persistent, disposable three-user Linux Crew fixture.

This is intentionally separate from the historical two-user smoke runner.  It
creates one EC2 bootstrap key and three independently generated user keys;
product activity after setup is expected to use only the latter identities.
The fixture remains alive until cleanup_fixture.py is run or the deadline is
reached.
"""

from __future__ import annotations

import argparse
import base64
import datetime as dt
import hashlib
import ipaddress
import json
from pathlib import Path
import shlex
import subprocess
import tempfile
import time
import urllib.request
import uuid

REGION = "us-west-2"
USERS = (("crew_alice", 10001), ("crew_bob", 10002), ("crew_carol", 10003))
REPO = Path(__file__).resolve().parents[5]


def run(argv: list[str], *, input: bytes | None = None, check: bool = True, timeout: int = 60) -> subprocess.CompletedProcess[bytes]:
    result = subprocess.run(argv, input=input, capture_output=True, timeout=timeout)
    if check and result.returncode:
        raise RuntimeError(f"{argv[:3]} failed: {result.stderr.decode(errors='replace')}")
    return result


def aws(region: str, *args: str, check: bool = True) -> dict:
    result = run(["aws", "--region", region, *args, "--output", "json"], check=check)
    if not check:
        return {"returncode": result.returncode, "stdout": result.stdout.decode(), "stderr": result.stderr.decode()}
    return json.loads(result.stdout) if result.stdout.strip() else {}


def b64(data: bytes) -> str:
    return base64.b64encode(data).decode("ascii")


def utc_now() -> str:
    return dt.datetime.now(dt.timezone.utc).isoformat()


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--region", default=REGION)
    parser.add_argument("--instance-type", default="t3.small", choices=("t3.small", "t3.medium"))
    parser.add_argument("--hours", type=float, default=4.0)
    parser.add_argument("--state-dir", type=Path, default=None)
    args = parser.parse_args()
    if args.hours <= 0 or args.hours > 12:
        raise SystemExit("--hours must be between 0 and 12")

    run_id = "crew-three-user-" + dt.datetime.now(dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:8]
    state_dir = args.state_dir or Path(tempfile.mkdtemp(prefix=run_id + "-", dir="/tmp"))
    state_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
    state_path = state_dir / "fixture-state.json"
    evidence_path = REPO / "docs" / "research" / "biorouter-crew" / (run_id + ".json")
    state: dict = {
        "run": run_id,
        "region": args.region,
        "started": utc_now(),
        "deadline": (dt.datetime.now(dt.timezone.utc) + dt.timedelta(hours=args.hours)).isoformat(),
        "state_dir": str(state_dir),
        "evidence": str(evidence_path),
        "resources": {},
        "accounts": [],
        "checks": [],
        "cleanup": {"status": "pending", "command": f"python3 {Path(__file__).with_name('cleanup_fixture.py')} {state_path}"},
    }

    def save() -> None:
        state_path.write_text(json.dumps(state, indent=2) + "\n")
        evidence_path.write_text(json.dumps(state, indent=2) + "\n")

    def check(name: str, details: object) -> None:
        state["checks"].append({"name": name, "result": details})
        save()
        print(name + ": " + json.dumps(details), flush=True)

    bootstrap_key_name = run_id + "-bootstrap"
    try:
        caller = aws(args.region, "sts", "get-caller-identity")
        state["aws_account"] = caller.get("Account")
        with urllib.request.urlopen("https://checkip.amazonaws.com", timeout=15) as response:
            source_ip = str(ipaddress.IPv4Address(response.read().decode().strip()))
        state["source_ip"] = source_ip
        ami = aws(args.region, "ssm", "get-parameter", "--name", "/aws/service/ami-amazon-linux-latest/al2023-ami-kernel-default-x86_64")["Parameter"]["Value"]
        vpc = aws(args.region, "ec2", "describe-vpcs", "--filters", "Name=is-default,Values=true")["Vpcs"][0]["VpcId"]
        subnet = aws(args.region, "ec2", "describe-subnets", "--filters", f"Name=vpc-id,Values={vpc}")["Subnets"][0]["SubnetId"]
        state["resources"].update({"ami": ami, "vpc": vpc, "subnet": subnet, "ingress": source_ip + "/32", "bootstrap_key_pair": bootstrap_key_name})
        bootstrap = aws(args.region, "ec2", "create-key-pair", "--key-name", bootstrap_key_name, "--key-type", "ed25519")
        bootstrap_path = state_dir / "bootstrap"
        bootstrap_path.write_text(bootstrap["KeyMaterial"])
        bootstrap_path.chmod(0o600)
        state["bootstrap_key_path"] = str(bootstrap_path)
        user_keys: dict[str, Path] = {}
        user_pubs: dict[str, str] = {}
        for username, uid in USERS:
            key_path = state_dir / username
            run(["ssh-keygen", "-q", "-t", "ed25519", "-N", "", "-C", run_id + ":" + username, "-f", str(key_path)])
            key_path.chmod(0o600)
            user_keys[username] = key_path
            user_pubs[username] = key_path.with_name(key_path.name + ".pub").read_text().strip()
            state["accounts"].append({"name": username, "uid": uid, "home": f"/home/{username}", "ssh_key_path": str(key_path), "ssh_key_id": username + "-ed25519"})
        state["resources"]["user_keys"] = [a["ssh_key_id"] for a in state["accounts"]]
        save()

        group_id = aws(args.region, "ec2", "create-security-group", "--group-name", run_id, "--description", "Disposable BioRouter Crew three-user fixture", "--vpc-id", vpc)["GroupId"]
        state["resources"]["security_group"] = group_id
        aws(args.region, "ec2", "authorize-security-group-ingress", "--group-id", group_id, "--protocol", "tcp", "--port", "22", "--cidr", source_ip + "/32")
        tags = [{"Key": "Name", "Value": run_id}, {"Key": "Purpose", "Value": "biorouter-crew-three-user-devapp-e2e"}, {"Key": "Owner", "Value": "BioRouter-Codex"}, {"Key": "ExpiresAt", "Value": state["deadline"]}]

        fixture_lines = []
        for username, _uid in USERS:
            files = {
                "README.txt": f"Synthetic Crew fixture owned by {username}; fakeSensitive marker; {run_id}\n".encode(),
                "samples.csv": f"owner,value\n{username},1\n".encode(),
                "image.png": b"\x89PNG\r\n\x1a\n" + bytes(range(32)),
                "all-bytes.bin": bytes(range(256)),
            }
            fixture_lines.append(f"install -d -m 700 -o {username} -g {username} /home/{username}/crew-work")
            for filename, content in files.items():
                fixture_lines.append(f"printf %s {shlex.quote(b64(content))} | base64 -d > /home/{username}/crew-work/{filename}")
                fixture_lines.append(f"chown {username}:{username} /home/{username}/crew-work/{filename}; chmod 600 /home/{username}/crew-work/{filename}")
            fixture_lines.append(f"sha256sum /home/{username}/crew-work/* > /home/{username}/crew-work/SHA256SUMS; chown {username}:{username} /home/{username}/crew-work/SHA256SUMS; chmod 600 /home/{username}/crew-work/SHA256SUMS")
        account_lines = []
        for (username, uid), pub in zip(USERS, user_pubs.values()):
            account_lines += [f"useradd --uid {uid} --create-home --shell /bin/bash {username}", f"install -d -m 700 -o {username} -g {username} /home/{username}/.ssh", f"printf '%s\\n' {shlex.quote(pub)} > /home/{username}/.ssh/authorized_keys", f"chown {username}:{username} /home/{username}/.ssh/authorized_keys; chmod 600 /home/{username}/.ssh/authorized_keys"]
        script = """#!/bin/bash
set -euo pipefail
dnf install -y python3 >/dev/null
""" + "\n".join(account_lines + fixture_lines) + """
echo "CREW_HOSTKEY entry $(cat /etc/ssh/ssh_host_ed25519_key.pub)" > /dev/console
echo "CREW_READY "$(date -u +%Y-%m-%dT%H:%M:%SZ) > /dev/console
"""
        user_data = b64(script.encode())
        opts = ["ec2", "run-instances", "--image-id", ami, "--instance-type", args.instance_type, "--count", "1", "--key-name", bootstrap_key_name, "--network-interfaces", json.dumps([{"DeviceIndex": 0, "SubnetId": subnet, "Groups": [group_id], "AssociatePublicIpAddress": True, "DeleteOnTermination": True}]), "--metadata-options", "HttpTokens=required,HttpEndpoint=enabled,HttpPutResponseHopLimit=1", "--block-device-mappings", json.dumps([{"DeviceName": "/dev/xvda", "Ebs": {"VolumeSize": 20, "VolumeType": "gp3", "Encrypted": True, "DeleteOnTermination": True}}]), "--tag-specifications", json.dumps([{"ResourceType": kind, "Tags": tags} for kind in ("instance", "volume")]), "--user-data", "fileb://" + str(state_dir / "user-data.b64")]
        # fileb avoids shell expansion while retaining an auditable local copy.
        (state_dir / "user-data.b64").write_bytes(base64.b64decode(user_data))
        dry = run(["aws", "--region", args.region, *opts, "--dry-run"], check=False)
        if dry.returncode == 0 or b"DryRunOperation" not in dry.stderr:
            raise RuntimeError("EC2 dry-run did not return DryRunOperation")
        check("ec2_dry_run", "DryRunOperation")
        launched = aws(args.region, *opts)
        instance = launched["Instances"][0]["InstanceId"]
        state["resources"]["instance"] = instance
        save()
        deadline = time.monotonic() + 360
        description = None
        while time.monotonic() < deadline:
            description = aws(args.region, "ec2", "describe-instances", "--instance-ids", instance)["Reservations"][0]["Instances"][0]
            if description["State"]["Name"] == "running" and description.get("PublicIpAddress"):
                break
            time.sleep(5)
        if not description or description["State"]["Name"] != "running" or not description.get("PublicIpAddress"):
            raise RuntimeError("instance did not become running with a public IPv4")
        state["resources"].update({"public_ip": description["PublicIpAddress"], "volumes": [m["Ebs"]["VolumeId"] for m in description["BlockDeviceMappings"]]})
        check("instance_controls", {"instance_type": description["InstanceType"], "metadata": description["MetadataOptions"], "delete_on_termination": [m["Ebs"]["DeleteOnTermination"] for m in description["BlockDeviceMappings"]]})
        volume = aws(args.region, "ec2", "describe-volumes", "--volume-ids", *state["resources"]["volumes"])["Volumes"][0]
        if not volume["Encrypted"]:
            raise RuntimeError("root volume is not encrypted")
        check("encrypted_root_volume", True)
        console = ""
        deadline = time.monotonic() + 360
        while time.monotonic() < deadline:
            console = aws(args.region, "ec2", "get-console-output", "--instance-id", instance, "--latest").get("Output", "")
            if "CREW_READY" in console and "CREW_HOSTKEY entry " in console:
                break
            time.sleep(10)
        host_line = next((line.split("CREW_HOSTKEY entry ", 1)[1] for line in console.splitlines() if "CREW_HOSTKEY entry " in line), None)
        if not host_line:
            raise RuntimeError("console output did not expose the expected host key")
        known_hosts = state_dir / "known_hosts"
        known_hosts.write_text(f"crew-entry {host_line}\n")
        known_hosts.chmod(0o600)
        state["host_key"] = {"source": "AWS GetConsoleOutput authenticated control plane", "strict_host_key_checking": True, "known_hosts_path": str(known_hosts), "sha256": hashlib.sha256(host_line.encode()).hexdigest()}
        config = state_dir / "ssh_config"
        config.write_text("""Host *\n  UserKnownHostsFile %s\n  StrictHostKeyChecking yes\n  IdentitiesOnly yes\n  BatchMode yes\n  ConnectTimeout 15\n  ConnectionAttempts 1\n  ForwardAgent no\n  LogLevel ERROR\nHost crew-entry\n  HostName %s\n  User ec2-user\n  IdentityFile %s\n  HostKeyAlias crew-entry\n""" % (known_hosts, description["PublicIpAddress"], bootstrap_path))
        state["connection_config"] = str(config)
        for account in state["accounts"]:
            user_config = state_dir / (account["name"] + ".ssh_config")
            user_config.write_text(config.read_text() + f"Host {account['name']}\n  HostName {description['PublicIpAddress']}\n  User {account['name']}\n  IdentityFile {account['ssh_key_path']}\n  HostKeyAlias crew-entry\n")
            user_config.chmod(0o600)
            identity = run(["ssh", "-F", str(user_config), account["name"], f"id -u; printf ' '; printf '%s' \"$HOME\"; printf '\\n'; test -f /home/{account['name']}/crew-work/README.txt"], timeout=60).stdout.decode().strip()
            uid, home = identity.splitlines()[0].split(" ", 1)
            if int(uid) != account["uid"] or home != account["home"]:
                raise RuntimeError(f"identity mismatch for {account['name']}: {identity}")
            account["connection_config"] = str(user_config)
        check("three_distinct_unprivileged_identities", [{"name": a["name"], "uid": a["uid"], "home": a["home"], "ssh_key_id": a["ssh_key_id"]} for a in state["accounts"]])
        state["status"] = "ready"
        state["product_workflow"] = {"privileged_setup_complete": True, "expected_runtime_users": [a["name"] for a in state["accounts"]], "sudo": False, "ssh_policy_changed": False}
        save()
        print("READY " + str(state_path), flush=True)
        print(json.dumps({"instance": instance, "public_ip": description["PublicIpAddress"], "state": str(state_path), "accounts": [{"name": a["name"], "uid": a["uid"], "home": a["home"], "key_path": a["ssh_key_path"], "ssh_config": a["connection_config"]} for a in state["accounts"]]}, indent=2), flush=True)
        return 0
    except BaseException as error:
        state["status"] = "failed"
        state["error"] = str(error)
        save()
        print("FAILED: " + str(error) + f"; state={state_path}", flush=True)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
