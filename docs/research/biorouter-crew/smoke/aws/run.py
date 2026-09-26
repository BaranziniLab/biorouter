#!/usr/bin/env python3
"""Launch ONE disposable VM, run synthetic tests, and tear down AWS resources.

Requires an explicitly authorized AWS account, AWS CLI, Python 3 and OpenSSH.
No institutional host or user data is accessed. Estimated runtime: 3-8 minutes.
"""
import base64
import datetime
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import shlex
import subprocess
import tempfile
import time
import urllib.request
import uuid

HERE = Path(__file__).resolve().parent
REGION = "us-west-2"
RUN = "crew-smoke-" + datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ") + "-" + uuid.uuid4().hex[:6]
EVIDENCE = HERE.parent.parent / (RUN + ".json")
records = {"run": RUN, "region": REGION, "started": datetime.datetime.now(datetime.timezone.utc).isoformat(), "checks": [], "resources": {}, "cleanup": {}}


def save():
    EVIDENCE.write_text(json.dumps(records, indent=2) + "\n")


def command(argv, *, input=None, timeout=40, check=True):
    result = subprocess.run(argv, input=input, capture_output=True, timeout=timeout)
    if check and result.returncode:
        raise RuntimeError(f"{argv[0:3]} failed: {result.stderr.decode(errors='replace')}")
    return result


def aws(*arguments, check=True, timeout=40):
    result = command(["aws", "--region", REGION, *arguments, "--output", "json"], check=check, timeout=timeout)
    if not check:
        return result
    return json.loads(result.stdout) if result.stdout.strip() else {}


def record(name, detail):
    records["checks"].append({"name": name, "result": detail})
    save()
    print(name + ": " + json.dumps(detail), flush=True)


instance = group = keyname = None
temporary = tempfile.TemporaryDirectory(prefix=RUN + "-")
directory = Path(temporary.name)
try:
    caller = aws("sts", "get-caller-identity")
    records["aws_authenticated"] = bool(caller.get("Account"))
    with urllib.request.urlopen("https://checkip.amazonaws.com", timeout=15) as response:
        source_ip = str(ipaddress.IPv4Address(response.read().decode().strip()))
    ami = aws("ssm", "get-parameter", "--name", "/aws/service/ami-amazon-linux-latest/al2023-ami-kernel-default-x86_64")["Parameter"]["Value"]
    vpc = aws("ec2", "describe-vpcs", "--filters", "Name=is-default,Values=true")["Vpcs"][0]["VpcId"]
    subnet = aws("ec2", "describe-subnets", "--filters", f"Name=vpc-id,Values={vpc}")["Subnets"][0]["SubnetId"]
    keyname = RUN
    key = aws("ec2", "create-key-pair", "--key-name", keyname, "--key-type", "ed25519")
    keypath = directory / "identity"
    keypath.write_text(key.pop("KeyMaterial"))
    keypath.chmod(0o600)
    records["resources"].update(key_pair=keyname, ami=ami, vpc=vpc, subnet=subnet, ingress_scope="current-client-ipv4/32")
    save()
    group = aws("ec2", "create-security-group", "--group-name", RUN, "--description", "Disposable synthetic BioRouter Crew smoke", "--vpc-id", vpc)["GroupId"]
    records["resources"]["security_group"] = group
    save()
    aws("ec2", "authorize-security-group-ingress", "--group-id", group, "--protocol", "tcp", "--port", "22", "--cidr", source_ip + "/32")
    userdata = (HERE / "user-data.sh").read_text().replace("@@BROKER@@", base64.b64encode((HERE / "broker.py").read_bytes()).decode()).replace("@@CLIENT@@", base64.b64encode((HERE / "client.py").read_bytes()).decode())
    datapath = directory / "user-data.sh"
    datapath.write_text(userdata)
    options = ["ec2", "run-instances", "--image-id", ami, "--instance-type", "t3.micro", "--count", "1", "--key-name", keyname, "--network-interfaces", json.dumps([{"DeviceIndex": 0, "SubnetId": subnet, "Groups": [group], "AssociatePublicIpAddress": True, "DeleteOnTermination": True}]), "--metadata-options", "HttpTokens=required,HttpEndpoint=enabled,HttpPutResponseHopLimit=1", "--block-device-mappings", json.dumps([{"DeviceName": "/dev/xvda", "Ebs": {"VolumeSize": 8, "VolumeType": "gp3", "Encrypted": True, "DeleteOnTermination": True}}]), "--tag-specifications", json.dumps([{"ResourceType": kind, "Tags": [{"Key": "Name", "Value": RUN}, {"Key": "Purpose", "Value": "biorouter-crew-synthetic-smoke"}]} for kind in ("instance", "volume")]), "--user-data", "file://" + str(datapath)]
    dry = aws(*options, "--dry-run", check=False)
    assert dry.returncode and b"DryRunOperation" in dry.stderr, dry.stderr
    record("ec2_dry_run", "DryRunOperation")
    launched = aws(*options)
    instance = launched["Instances"][0]["InstanceId"]
    records["resources"]["instance"] = instance
    save()
    deadline = time.monotonic() + 300
    public_ip = None
    while time.monotonic() < deadline:
        description = aws("ec2", "describe-instances", "--instance-ids", instance)["Reservations"][0]["Instances"][0]
        if description["State"]["Name"] == "running" and description.get("PublicIpAddress"):
            public_ip = description["PublicIpAddress"]
            break
        time.sleep(5)
    assert public_ip, "VM did not become running"
    records["resources"]["volumes"] = [mapping["Ebs"]["VolumeId"] for mapping in description["BlockDeviceMappings"]]
    record("instance_controls", {"type": description["InstanceType"], "metadata": description["MetadataOptions"], "delete_on_termination": [mapping["Ebs"]["DeleteOnTermination"] for mapping in description["BlockDeviceMappings"]]})
    volume = aws("ec2", "describe-volumes", "--volume-ids", *records["resources"]["volumes"])["Volumes"][0]
    assert volume["Encrypted"] is True
    record("encrypted_root_volume", True)
    hostkeys = {}
    deadline = time.monotonic() + 300
    while time.monotonic() < deadline:
        console = aws("ec2", "get-console-output", "--instance-id", instance, "--latest").get("Output", "")
        for name, kind, keydata in re.findall(r"CREW_HOSTKEY (entry|gate-a|gate-b|target) (ssh-ed25519) ([A-Za-z0-9+/=]+)", console):
            hostkeys[name] = kind + " " + keydata
        if "CREW_READY" in console and len(hostkeys) == 4:
            break
        time.sleep(10)
    assert len(hostkeys) == 4, "AWS console did not provide four trusted host public keys"
    known = directory / "known_hosts"
    known.write_text("".join(f"crew-{name} {key}\n" for name, key in hostkeys.items()))
    record("host_key_source", {"source": "AWS GetConsoleOutput authenticated control plane", "aliases": sorted(hostkeys), "strict_host_key_checking": "yes"})
    config = directory / "ssh_config"
    config.write_text(f"""Host *
  IdentityFile {keypath}
  IdentitiesOnly yes
  UserKnownHostsFile {known}
  StrictHostKeyChecking yes
  BatchMode yes
  ConnectTimeout 10
  ConnectionAttempts 1
  ServerAliveInterval 10
  ServerAliveCountMax 2
  ForwardAgent no
  LogLevel ERROR
Host crew-entry
  HostName {public_ip}
  User ec2-user
  HostKeyAlias crew-entry
Host crew-gate-a
  HostName 127.0.0.1
  Port 2222
  User ec2-user
  HostKeyAlias crew-gate-a
Host crew-gate-b
  HostName 127.0.0.1
  Port 2223
  User ec2-user
  HostKeyAlias crew-gate-b
Host crew-alice crew-bob
  HostName 127.0.0.1
  Port 2224
  HostKeyAlias crew-target
  ProxyJump crew-entry,crew-gate-a,crew-gate-b
Host crew-alice
  User crew_alice
Host crew-bob
  User crew_bob
""")

    def ssh(host, remote, *, data=None):
        return command(["ssh", "-F", str(config), host, remote], input=data, timeout=60).stdout

    def request(host, payload):
        return json.loads(ssh(host, "python3 /opt/crew-smoke/client.py " + shlex.quote(json.dumps(payload))))

    record("ssh_versions", {"client": command(["ssh", "-V"]).stderr.decode().strip(), "server": ssh("crew-entry", "/usr/sbin/sshd -V 2>&1").decode().strip()})
    alice = request("crew-alice", {"action": "identity"})
    bob = request("crew-bob", {"action": "identity"})
    assert alice["username"] == "crew_alice" and bob["username"] == "crew_bob" and alice["uid"] != bob["uid"]
    record("distinct_unix_peer_identity_via_two_loopback_gates", {"alice": alice, "bob": bob, "topology": "public SSH entry -> loopback gate A:2222 -> loopback gate B:2223 -> loopback target:2224 on ONE VM"})
    forged = request("crew-alice", {"action": "identity", "username": "crew_bob"})
    assert forged.get("error") == "claimed_identity_mismatch"
    record("forged_username_rejected", forged)
    created = request("crew-alice", {"action": "create", "agent": "alice-agent"})
    assert created["ok"]
    denied = request("crew-bob", {"action": "invoke", "agent": "alice-agent"})
    assert denied.get("error") == "not_agent_owner"
    record("cross_owner_agent_invocation_rejected", denied)
    invoked = request("crew-alice", {"action": "invoke", "agent": "alice-agent"})
    assert invoked["ok"]
    record("owner_agent_invocation_accepted", invoked)
    assert request("crew-bob", {"action": "create", "agent": "bob-agent"})["ok"]
    assert request("crew-bob", {"action": "invoke", "agent": "bob-agent"})["ok"]
    record("second_user_own_agent_accepted", True)
    payload = bytes(range(256)) * 4096 + b"\x00synthetic attachment\xff\n"
    digest = hashlib.sha256(payload).hexdigest()
    ssh("crew-alice", "umask 077; cat > /home/crew_alice/attachment.bin", data=payload)
    remote_hash = ssh("crew-alice", "sha256sum /home/crew_alice/attachment.bin").decode().split()[0]
    downloaded = directory / "download.bin"
    command(["scp", "-F", str(config), "crew-alice:/home/crew_alice/attachment.bin", str(downloaded)], timeout=60)
    assert remote_hash == digest == hashlib.sha256(downloaded.read_bytes()).hexdigest()
    record("binary_attachment_upload_download", {"bytes": len(payload), "sha256": digest, "download_transport": "OpenSSH scp with SFTP over same ProxyJump chain"})
    before = request("crew-alice", {"action": "history", "after": 0})["events"]
    ssh("crew-entry", "sudo systemctl restart crew-smoke.service")
    after = request("crew-alice", {"action": "history", "after": 0})["events"]
    assert before == after and len(after) == 4
    delta = request("crew-alice", {"action": "history", "after": 2})["events"]
    assert [event["seq"] for event in delta] == [3, 4]
    assert request("crew-bob", {"action": "invoke", "agent": "alice-agent"})["error"] == "not_agent_owner"
    record("jsonl_fsync_restart_reconnect_replay", {"events": len(after), "cursor_after_2": [event["seq"] for event in delta], "ownership_survives_restart": True})
    records["outcome"] = "pass"
except BaseException as error:
    records["outcome"] = "failed"
    records["error"] = str(error)
    save()
    print("FAILED: " + str(error), flush=True)
finally:
    if instance:
        try:
            aws("ec2", "terminate-instances", "--instance-ids", instance)
            deadline = time.monotonic() + 180
            while time.monotonic() < deadline:
                state = aws("ec2", "describe-instances", "--instance-ids", instance)["Reservations"][0]["Instances"][0]["State"]["Name"]
                if state == "terminated":
                    records["cleanup"]["instance"] = "terminated"
                    break
                time.sleep(5)
            assert records["cleanup"].get("instance") == "terminated", "termination not confirmed"
            for volume_id in records["resources"].get("volumes", []):
                deadline = time.monotonic() + 60
                while time.monotonic() < deadline:
                    volume_result = aws("ec2", "describe-volumes", "--volume-ids", volume_id, check=False)
                    if volume_result.returncode and b"InvalidVolume.NotFound" in volume_result.stderr:
                        records["cleanup"][volume_id] = "deleted"
                        break
                    time.sleep(5)
                assert records["cleanup"].get(volume_id) == "deleted", "root volume deletion not confirmed"
        except BaseException as error:
            records["cleanup"]["instance_error"] = str(error)
    if group:
        try:
            aws("ec2", "delete-security-group", "--group-id", group)
            records["cleanup"]["security_group"] = "deleted"
        except BaseException as error:
            records["cleanup"]["security_group_error"] = str(error)
    if keyname:
        try:
            aws("ec2", "delete-key-pair", "--key-name", keyname)
            records["cleanup"]["key_pair"] = "deleted"
        except BaseException as error:
            records["cleanup"]["key_pair_error"] = str(error)
    temporary.cleanup()
    records["cleanup"]["local_temporary_directory_removed"] = not directory.exists()
    records["finished"] = datetime.datetime.now(datetime.timezone.utc).isoformat()
    save()
    print("EVIDENCE " + str(EVIDENCE), flush=True)
    print("CLEANUP " + json.dumps(records["cleanup"]), flush=True)
if records.get("outcome") != "pass" or any(key.endswith("error") for key in records["cleanup"]):
    raise SystemExit(1)
