import json
import os
import subprocess
import sys
import time
from hashlib import sha256

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey

WORKSPACE = os.environ.get("WORKSPACE", "b104bab2-9463-49de-bbd7-3c0c22d0ed59")
SOCKET = os.environ.get(
    "SOCKET", "/tmp/crew-1101-2a94b28e88da4fd6ba66e69ae99dd9fd/broker.sock"
)
REMOTE_ROOT = os.environ.get("REMOTE_ROOT", "/home/alice/work/csv")
CONTAINER = os.environ.get("CONTAINER", "biorouter-crew-canary-luna")
UID = 1101
PRIVATE = bytes([7]) * 32
KEY = Ed25519PrivateKey.from_private_bytes(PRIVATE)
PUBLIC = KEY.public_key().public_bytes_raw().hex()
DEVICE = sha256(bytes.fromhex(PUBLIC)).hexdigest()


def canonical(value):
    if isinstance(value, dict):
        return {key: canonical(value[key]) for key in sorted(value)}
    if isinstance(value, list):
        return [canonical(item) for item in value]
    return value


def payload(nonce, method, params):
    return json.dumps(
        [WORKSPACE, UID, nonce, method, canonical(params)],
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode()


def request(proc, value):
    proc.stdin.write(json.dumps(value, separators=(",", ":")) + "\n")
    proc.stdin.flush()
    line = proc.stdout.readline()
    if not line:
        raise RuntimeError(f"bridge closed while sending {value['id']}")
    return json.loads(line)


def signed(proc, number, method, params):
    challenge = request(
        proc,
        {
            "version": 1,
            "id": f"challenge-{number}",
            "method": "auth.challenge",
            "params": {"device_id": DEVICE},
        },
    )
    nonce = challenge["result"]["nonce"]
    auth = {
        "device_id": DEVICE,
        "nonce": nonce,
        "signature": KEY.sign(payload(nonce, method, params)).hex(),
    }
    return request(
        proc,
        {
            "version": 1,
            "id": f"req-{number}",
            "method": method,
            "params": params,
            "auth": auth,
        },
    )


def remote(proc, number, method, params, credential):
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


def main():
    suffix = os.environ.get("SUFFIX", "-2" if os.environ.get("CONTINUE") else "")
    command = [
        "/usr/local/bin/docker",
        "exec",
        "-i",
        "-u",
        "alice",
        "-e",
        "HOME=/home/alice",
        CONTAINER,
        "/home/alice/.local/bin/biorouter-crew",
        "bridge",
        "--stdio",
        "--socket",
        SOCKET,
        "--owner-uid",
        str(UID),
        "--workspace-id",
        WORKSPACE,
    ]
    proc = subprocess.Popen(
        command,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    results = {}
    try:
        if os.environ.get("CONTINUE"):
            snapshot = signed(proc, 1, "workspace.snapshot", {})
            results["snapshot"] = snapshot
            team = snapshot["result"]["teams"][0]
            channel = team["general_channel_id"]
        else:
            bootstrap = signed(proc, 1, "auth.bootstrap", {"public_key": PUBLIC})
            results["bootstrap"] = bootstrap
            team = signed(
                proc,
                2,
                "team.create",
                {"name": "remote-canary", "idempotency_key": "team"},
            )
            results["team"] = team
            channel = team["result"]["channel"]["id"]
        run_params = {
            "channel_id": channel,
            "source_channels": [channel],
            "provider_policy_id": "private",
            "personal_mode": "private",
            "public_provider": False,
            "remote_root": REMOTE_ROOT,
            "remote_execution": True,
            "expires_in": 300,
            "idempotency_key": f"remote-canary-run{suffix}",
        }
        created = signed(proc, 3, "run.create", run_params)
        results["run"] = created
        credential = created["result"]["credential"]
        positive_script = (
            "import csv\n"
            "with open('input.csv', newline='') as h:\n"
            "    rows = list(csv.DictReader(h))\n"
            "total = sum(int(row['amount']) for row in rows)\n"
            "with open('summary.csv', 'w', newline='') as h:\n"
            "    writer = csv.writer(h)\n"
            "    writer.writerow(['rows', 'total'])\n"
            "    writer.writerow([len(rows), total])\n"
            "print(f'rows={len(rows)} total={total}')\n"
        )
        started = remote(
            proc,
            4,
            "remote.execute",
            {
                "argv": ["python3", "-c", positive_script],
                "timeout_seconds": 20,
                "idempotency_key": f"csv-positive{suffix}",
            },
            credential,
        )
        results["positive_started"] = started
        if "result" not in started:
            raise RuntimeError(json.dumps(started, sort_keys=True))
        job_id = started["result"]["job_id"]
        for index in range(100):
            status = remote(
                proc,
                100 + index,
                "remote.job_status",
                {"job_id": job_id},
                credential,
            )
            if status.get("result", {}).get("status") not in {"running", "starting"}:
                results["positive_status"] = status
                break
            time.sleep(0.1)
        results["positive_read"] = remote(
            proc,
            200,
            "remote.read",
            {"path": "summary.csv"},
            credential,
        )
        results["outside_absolute"] = remote(
            proc,
            201,
            "remote.read",
            {"path": "/etc/passwd"},
            credential,
        )
        results["outside_parent"] = remote(
            proc,
            202,
            "remote.read",
            {"path": "../.ssh"},
            credential,
        )
        results["invalid_credential"] = remote(
            proc,
            203,
            "remote.list",
            {"path": "."},
            "deadbeef",
        )
        results["network_job"] = remote(
            proc,
            204,
            "remote.execute",
            {
                "argv": [
                    "python3",
                    "-c",
                    "import socket; socket.create_connection(('127.0.0.1', 9), 1)",
                ],
                "timeout_seconds": 10,
                "idempotency_key": f"network-negative{suffix}",
            },
            credential,
        )
        network_id = results["network_job"]["result"]["job_id"]
        for index in range(100):
            status = remote(
                proc,
                300 + index,
                "remote.job_status",
                {"job_id": network_id},
                credential,
            )
            if status.get("result", {}).get("status") not in {"running", "starting"}:
                results["network_status"] = status
                break
            time.sleep(0.1)
        results["subprocess_job"] = remote(
            proc,
            205,
            "remote.execute",
            {
                "argv": [
                    "python3",
                    "-c",
                    "import subprocess; subprocess.run(['/bin/echo', 'forbidden'], check=True)",
                ],
                "timeout_seconds": 10,
                "idempotency_key": f"subprocess-negative{suffix}",
            },
            credential,
        )
        subprocess_id = results["subprocess_job"]["result"]["job_id"]
        for index in range(100):
            status = remote(
                proc,
                400 + index,
                "remote.job_status",
                {"job_id": subprocess_id},
                credential,
            )
            if status.get("result", {}).get("status") not in {"running", "starting"}:
                results["subprocess_status"] = status
                break
            time.sleep(0.1)
        subprocess.run(
            [
                "/usr/local/bin/docker",
                "exec",
                CONTAINER,
                "sh",
                "-lc",
                f"ln -s /etc/passwd {REMOTE_ROOT}/escape-link",
            ],
            check=True,
        )
        results["symlink_read"] = remote(
            proc,
            206,
            "remote.read",
            {"path": "escape-link"},
            credential,
        )
        results["symlink_execute"] = remote(
            proc,
            207,
            "remote.execute",
            {
                "argv": ["python3", "-c", "print('must not run')"],
                "timeout_seconds": 10,
                "idempotency_key": f"symlink-negative{suffix}",
            },
            credential,
        )
    finally:
        proc.stdin.close()
        proc.wait(timeout=10)
    print(json.dumps(results, sort_keys=True))


if __name__ == "__main__":
    main()
