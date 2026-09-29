#!/usr/bin/env python3
"""Signed real-UID Crew workload soak for a disposable Linux workspace.

Run this only in a disposable Linux container as root. It creates synthetic
accounts, bootstraps a separate rootless broker, enrolls every participant,
accepts a team invitation, and runs signed message post/history traffic. The
three-user QA fixture is never touched.
"""

from __future__ import annotations

import argparse
import datetime
import hashlib
import json
import os
import pathlib
import pwd
import signal
import socket
import subprocess
import sys
import tempfile
import time

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives.serialization import Encoding, PublicFormat


BOOTSTRAP_SEED = bytes([7]) * 32


def key_for(uid: int) -> Ed25519PrivateKey:
    seed = hashlib.sha256(f"crew-soak-key:{uid}".encode()).digest()
    return Ed25519PrivateKey.from_private_bytes(seed)


def public_hex(key: Ed25519PrivateKey) -> str:
    public_key = key.public_key()
    try:
        return public_key.public_bytes_raw().hex()
    except AttributeError:
        return public_key.public_bytes(Encoding.Raw, PublicFormat.Raw).hex()


def device_id(public: str) -> str:
    return hashlib.sha256(bytes.fromhex(public)).hexdigest()


def canonical(value):
    if isinstance(value, dict):
        return {key: canonical(value[key]) for key in sorted(value)}
    if isinstance(value, list):
        return [canonical(item) for item in value]
    return value


def sign_payload(workspace: str, uid: int, nonce: str, method: str, params: dict) -> bytes:
    return json.dumps(
        [workspace, uid, nonce, method, canonical(params)],
        separators=(",", ":"),
        ensure_ascii=False,
    ).encode()


class Client:
    def __init__(self, socket_path: str, workspace: str, uid: int, key: Ed25519PrivateKey):
        self.workspace = workspace
        self.uid = uid
        self.key = key
        self.public = public_hex(key)
        self.stream = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.stream.settimeout(15)
        self.stream.connect(socket_path)
        self.file = self.stream.makefile("rwb")
        self.counter = 0

    def request(self, method: str, params: dict, auth=None) -> dict:
        self.counter += 1
        request = {"version": 1, "id": f"soak-{self.uid}-{self.counter}", "method": method, "params": params}
        if auth is not None:
            request["auth"] = auth
        self.file.write(json.dumps(request, separators=(",", ":")).encode() + b"\n")
        self.file.flush()
        line = self.file.readline()
        if not line:
            raise ConnectionError("broker closed framed response")
        response = json.loads(line)
        if response.get("error"):
            raise RuntimeError(response["error"])
        return response

    def signed(self, method: str, params: dict) -> dict:
        challenge = self.request("auth.challenge", {"device_id": device_id(self.public)})
        nonce = challenge["result"]["nonce"]
        signature = self.key.sign(sign_payload(self.workspace, self.uid, nonce, method, params)).hex()
        return self.request(
            method,
            params,
            {"device_id": device_id(self.public), "nonce": nonce, "signature": signature},
        )

    def close(self) -> None:
        self.file.close()
        self.stream.close()


def runuser(user: str, args: list[str], env: dict[str, str] | None = None) -> subprocess.CompletedProcess:
    merged = os.environ.copy()
    if env:
        merged.update(env)
    return subprocess.run(["runuser", "-u", user, "--", *args], env=merged, check=True, text=True, capture_output=True)


def accounts(owner: str, owner_uid: int, count: int) -> list[tuple[str, int]]:
    if os.geteuid() != 0:
        raise RuntimeError("the real-UID soak must run as disposable-container root")
    specs = [(owner, owner_uid)] + [(f"crewsoak{index:03d}", 12000 + index) for index in range(1, count + 1)]
    for name, uid in specs:
        try:
            pwd.getpwnam(name)
        except KeyError:
            subprocess.run(["groupadd", "--gid", str(uid), name], check=True)
            subprocess.run(["useradd", "--uid", str(uid), "--gid", str(uid), "--create-home", "--shell", "/bin/sh", name], check=True)
    return specs[1:]


def owner_bootstrap(args: argparse.Namespace, manifest: pathlib.Path) -> dict:
    data = json.loads(manifest.read_text())
    client = Client(args.socket, args.workspace, args.owner_uid, Ed25519PrivateKey.from_private_bytes(BOOTSTRAP_SEED))
    try:
        client.signed("auth.bootstrap", {"public_key": public_hex(client.key)})
        team = client.signed("team.create", {"name": "real-uid-soak", "idempotency_key": "soak-team"})["result"]
        data["team_id"] = team["team"]["id"]
        data["channel_id"] = team["channel"]["id"]
        for profile in data["profiles"]:
            response = client.signed(
                "enrollment.invite",
                {"uid": profile["uid"], "public_key": profile["public"], "idempotency_key": f"enroll-{profile['uid']}"},
            )
            profile["enrollment"] = response["result"]["invitation"]
        manifest.write_text(json.dumps(data))
        print(json.dumps({"team_id": data["team_id"], "channel_id": data["channel_id"], "enrollments": len(data["profiles"])}))
        return data
    finally:
        client.close()


def participant_enroll(args: argparse.Namespace) -> int:
    key = Ed25519PrivateKey.from_private_bytes(bytes.fromhex(args.seed))
    client = Client(args.socket, args.workspace, args.uid, key)
    try:
        response = client.signed("auth.enroll", {"public_key": public_hex(key), "invitation": args.invitation})
        pathlib.Path(args.marker).write_text(json.dumps({"principal_id": response["result"]["principal"]["id"], "uid": args.uid}))
        return 0
    finally:
        client.close()


def owner_team_invites(args: argparse.Namespace, manifest: pathlib.Path) -> dict:
    data = json.loads(manifest.read_text())
    client = Client(args.socket, args.workspace, args.owner_uid, Ed25519PrivateKey.from_private_bytes(BOOTSTRAP_SEED))
    try:
        for profile in data["profiles"]:
            result = client.signed(
                "invitation.create",
                {"kind": "team", "target_id": data["team_id"], "principal_id": profile["principal_id"], "idempotency_key": f"team-invite-{profile['uid']}"},
            )
            profile["team_invitation"] = result["result"]["id"]
        manifest.write_text(json.dumps(data))
        return data
    finally:
        client.close()


def participant_workload(args: argparse.Namespace) -> int:
    key = Ed25519PrivateKey.from_private_bytes(bytes.fromhex(args.seed))
    client = Client(args.socket, args.workspace, args.uid, key)
    latencies: list[float] = []
    sent = acked = disconnects = readback = 0
    posted_ids: list[str] = []
    readback_ids: list[str] = []
    posted_body_hashes: dict[str, str] = {}
    readback_body_hashes: dict[str, str] = {}
    errors: list[str] = []
    cursor: str | None = None
    started = time.monotonic()
    started_at = datetime.datetime.now(datetime.timezone.utc).isoformat()
    metrics = pathlib.Path(args.metrics)
    try:
        client.signed("invitation.accept", {"invitation_id": args.invitation, "idempotency_key": f"accept-{args.uid}"})
        while time.monotonic() - started < args.duration:
            body = f"soak uid={args.uid} message={sent + 1}"
            begin = time.monotonic()
            response = client.signed("message.post", {"channel_id": args.channel, "body": body, "idempotency_key": f"msg-{args.uid}-{sent + 1}"})
            latencies.append((time.monotonic() - begin) * 1000)
            sent += 1
            message = response.get("result") or {}
            if message.get("id") and message.get("body") == body:
                acked += 1
                posted_ids.append(message["id"])
                posted_body_hashes[message["id"]] = hashlib.sha256(body.encode()).hexdigest()
            else:
                errors.append(f"message.post ack missing id/body for message {sent}")
            history_params = {"channel_id": args.channel, "latest": True, "limit": 50}
            if cursor is not None:
                history_params["after"] = cursor
            history = client.signed("messages.history", history_params)
            next_cursor = history["result"].get("cursor")
            if next_cursor is not None and not isinstance(next_cursor, str):
                errors.append(f"messages.history returned a non-string cursor: {next_cursor!r}")
            else:
                cursor = next_cursor
            visible = history["result"].get("messages", [])
            visible_by_body = {item.get("body"): item.get("id") for item in visible}
            if body in visible_by_body:
                readback += 1
                if visible_by_body[body]:
                    readback_ids.append(visible_by_body[body])
                    readback_body_hashes[visible_by_body[body]] = hashlib.sha256(body.encode()).hexdigest()
            else:
                errors.append(f"messages.history did not replay posted body {body!r}")
            if sent % 4 == 0:
                client.signed("workspace.snapshot", {})
            with metrics.open("a") as output:
                output.write(json.dumps({"uid": args.uid, "sent": sent, "acked": acked, "latency_ms": latencies[-1], "rss_kib": int(pathlib.Path("/proc/self/status").read_text().split("VmRSS:")[1].split()[0])}) + "\n")
            time.sleep(args.interval)
    except (ConnectionError, OSError) as exc:
        disconnects += 1
        errors.append(f"disconnect: {exc}")
    except Exception as exc:
        errors.append(str(exc))
        with metrics.open("a") as output:
            output.write(json.dumps({"uid": args.uid, "error": str(exc), "sent": sent, "acked": acked}) + "\n")
    finally:
        client.close()
    with metrics.open("a") as output:
        output.write(json.dumps({"uid": args.uid, "summary": True, "started_at": started_at, "ended_at": datetime.datetime.now(datetime.timezone.utc).isoformat(), "sent": sent, "acked": acked, "readback": readback, "posted_ids": posted_ids, "readback_ids": readback_ids, "posted_body_hashes": posted_body_hashes, "readback_body_hashes": readback_body_hashes, "disconnects": disconnects, "errors": errors, "latencies": latencies}) + "\n")
    return 1 if errors or disconnects or acked != sent or readback < sent else 0


def restart_replay(args: argparse.Namespace) -> int:
    expected = json.loads(pathlib.Path(args.expected).read_text())
    if args.uid is None or not args.seed:
        raise RuntimeError("restart replay requires an enrolled participant --uid and --seed")
    client = Client(args.socket, args.workspace, args.uid, Ed25519PrivateKey.from_private_bytes(bytes.fromhex(args.seed)))
    seen: dict[str, str] = {}
    cursor: str | None = None
    try:
        for _ in range(100):
            history_params = {"channel_id": args.channel, "latest": False, "limit": 100}
            if cursor is not None:
                history_params["after"] = cursor
            history = client.signed("messages.history", history_params)
            result = history["result"]
            messages = result.get("messages", [])
            next_cursor = result.get("cursor")
            if next_cursor is not None and not isinstance(next_cursor, str):
                raise RuntimeError(f"messages.history returned a non-string cursor: {next_cursor!r}")
            for message in messages:
                message_id = message.get("id")
                body = message.get("body")
                if message_id and isinstance(body, str):
                    seen[message_id] = hashlib.sha256(body.encode()).hexdigest()
            if not messages or next_cursor == cursor:
                break
            cursor = next_cursor
    finally:
        client.close()
    expected_hashes = expected["posted_body_hashes"]
    missing = sorted(set(expected_hashes) - set(seen))
    mismatched = sorted(message_id for message_id, digest in expected_hashes.items() if seen.get(message_id) != digest)
    result = {"expected": len(expected_hashes), "replayed": len(seen), "missing": missing, "mismatched": mismatched}
    print(json.dumps(result, sort_keys=True))
    return 1 if missing or mismatched else 0


def percentile(values: list[float], fraction: float) -> float:
    if not values:
        return 0.0
    values = sorted(values)
    return values[min(len(values) - 1, int(len(values) * fraction))]


def soak(args: argparse.Namespace) -> int:
    if sys.platform != "linux" or os.geteuid() != 0:
        raise RuntimeError("run this workload only as root in a disposable Linux container")
    if not 1 <= args.profiles <= 50:
        raise RuntimeError("profiles must be between 1 and 50")
    if args.duration < 60:
        raise RuntimeError("duration must be at least 60 seconds")
    binary = pathlib.Path(args.binary).resolve()
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise RuntimeError(f"missing executable: {binary}")
    owner, owner_uid = "crewsoakowner", 11900
    profiles = accounts(owner, owner_uid, args.profiles)
    state = pathlib.Path(args.state_dir).resolve()
    state.mkdir(mode=0o700, parents=True, exist_ok=False)
    os.chown(state, owner_uid, owner_uid)
    owner_home = pathlib.Path("/home") / owner
    process = subprocess.Popen(["runuser", "-u", owner, "--", "env", f"HOME={owner_home}", str(binary), "start", "--state-dir", str(state), "--bootstrap-key", public_hex(Ed25519PrivateKey.from_private_bytes(BOOTSTRAP_SEED))], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    if process.wait(timeout=15) != 0:
        raise RuntimeError(process.stderr.read())
    runtime_path = state / "runtime.json"
    deadline = time.monotonic() + 15
    while not runtime_path.exists() and time.monotonic() < deadline:
        time.sleep(0.25)
    runtime = json.loads(runtime_path.read_text())
    marker_dir = pathlib.Path(tempfile.mkdtemp(prefix="crew-real-uid-markers-"))
    os.chmod(marker_dir, 0o777)
    manifest = marker_dir / "manifest.json"
    manifest.write_text(json.dumps({"profiles": [{"name": name, "uid": uid, "public": public_hex(key_for(uid))} for name, uid in profiles]}))
    os.chmod(manifest, 0o666)
    owner_args = [sys.executable, __file__, "--owner-bootstrap", "--socket", runtime["socket"], "--workspace", runtime["workspace_id"], "--owner-uid", str(owner_uid), "--manifest", str(manifest)]
    runuser(owner, owner_args)
    data = json.loads(manifest.read_text())
    for profile in data["profiles"]:
        marker = marker_dir / f"enroll-{profile['uid']}.json"
        runuser(profile["name"], [sys.executable, __file__, "--participant-enroll", "--socket", runtime["socket"], "--workspace", runtime["workspace_id"], "--uid", str(profile["uid"]), "--seed", hashlib.sha256(f"crew-soak-key:{profile['uid']}".encode()).hexdigest(), "--invitation", profile["enrollment"], "--marker", str(marker)])
        profile["principal_id"] = json.loads(marker.read_text())["principal_id"]
    manifest.write_text(json.dumps(data))
    runuser(
        owner,
        [
            sys.executable,
            __file__,
            "--owner-team-invites",
            "--socket",
            runtime["socket"],
            "--workspace",
            runtime["workspace_id"],
            "--owner-uid",
            str(owner_uid),
            "--manifest",
            str(manifest),
        ],
    )
    data = json.loads(manifest.read_text())
    children = []
    metrics = marker_dir / "metrics.jsonl"
    metrics.touch()
    os.chmod(metrics, 0o666)
    start = time.monotonic()
    started_at = datetime.datetime.now(datetime.timezone.utc).isoformat()
    child_errors: list[str] = []
    try:
        for profile in data["profiles"]:
            children.append(subprocess.Popen(["runuser", "-u", profile["name"], "--", sys.executable, __file__, "--participant-workload", "--socket", runtime["socket"], "--workspace", runtime["workspace_id"], "--uid", str(profile["uid"]), "--seed", hashlib.sha256(f"crew-soak-key:{profile['uid']}".encode()).hexdigest(), "--invitation", profile["team_invitation"], "--channel", data["channel_id"], "--metrics", str(metrics), "--duration", str(args.duration), "--interval", str(args.interval)], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL))
        rss_samples = []
        journal_samples = []
        cpu_samples = []
        fd_samples = []
        sample_interval = min(10, max(1, args.interval))
        collection_deadline = start + args.duration + max(30, min(120, args.interval + 30))
        while time.monotonic() < collection_deadline and (time.monotonic() - start < args.duration or any(child.poll() is None for child in children)):
            if pathlib.Path(f"/proc/{runtime['pid']}/status").exists():
                status = pathlib.Path(f"/proc/{runtime['pid']}/status").read_text()
                rss_samples.append(int(status.split("VmRSS:")[1].split()[0]))
                stat = pathlib.Path(f"/proc/{runtime['pid']}/stat").read_text()
                stat = stat.rsplit(") ", 1)[1].split()
                cpu_samples.append(int(stat[11]) + int(stat[12]))
                fd_samples.append(len(list(pathlib.Path(f"/proc/{runtime['pid']}/fd").iterdir())))
            journal_samples.append((state / "journal.jsonl").stat().st_size)
            if time.monotonic() - start >= args.duration and not any(child.poll() is None for child in children):
                break
            time.sleep(sample_interval)
        if any(child.poll() is None for child in children):
            child_errors.append("participant processes exceeded the bounded collection grace")
        for child, profile in zip(children, data["profiles"]):
            try:
                return_code = child.wait(timeout=15)
            except subprocess.TimeoutExpired:
                child.send_signal(signal.SIGTERM)
                return_code = child.wait(timeout=10)
            if return_code != 0:
                child_errors.append(f"uid {profile['uid']} workload exited {return_code}")
        records = [json.loads(line) for line in metrics.read_text().splitlines()] if metrics.exists() else []
        latencies = [record["latency_ms"] for record in records if "latency_ms" in record]
        summaries = [record for record in records if record.get("summary")]
        summaries_by_uid = {record.get("uid"): record for record in summaries if record.get("uid") is not None}
        expected_uids = {profile["uid"] for profile in data["profiles"]}
        missing_uids = sorted(expected_uids - summaries_by_uid.keys())
        if missing_uids:
            child_errors.append(f"missing terminal metrics for UIDs {missing_uids}")
        if len(summaries_by_uid) != len(expected_uids):
            child_errors.append("terminal metrics did not cover each distinct participant UID")
        expected_min = max(1, args.duration // (args.interval + 15))
        for uid in sorted(expected_uids):
            summary = summaries_by_uid.get(uid)
            if summary is None:
                continue
            if summary.get("sent", 0) < expected_min:
                child_errors.append(f"uid {uid} sent {summary.get('sent', 0)}, expected at least {expected_min}")
            if summary.get("acked", 0) != summary.get("sent", 0):
                child_errors.append(f"uid {uid} ack count does not equal sent count")
            if summary.get("readback", 0) < summary.get("sent", 0):
                child_errors.append(f"uid {uid} history replay count is below sent count")
            if len(set(summary.get("posted_ids", []))) != summary.get("sent", 0):
                child_errors.append(f"uid {uid} posted message IDs are not distinct")
            if not set(summary.get("posted_ids", [])).issubset(set(summary.get("readback_ids", []))):
                child_errors.append(f"uid {uid} history replay omitted a posted message ID")
            child_errors.extend(f"uid {uid}: {error}" for error in summary.get("errors", []))
        sent = sum(record.get("sent", 0) for record in summaries_by_uid.values())
        acked = sum(record.get("acked", 0) for record in summaries_by_uid.values())
        disconnects = sum(record.get("disconnects", 0) for record in summaries_by_uid.values())
        posted_ids = [message_id for record in summaries_by_uid.values() for message_id in record.get("posted_ids", [])]
        readback_ids = [message_id for record in summaries_by_uid.values() for message_id in record.get("readback_ids", [])]
        unique_posted = len(set(posted_ids))
        unique_readback = len(set(readback_ids))
        if unique_posted != len(posted_ids):
            child_errors.append("posted message IDs were duplicated across participants")
        if unique_readback != len(readback_ids):
            child_errors.append("readback message IDs were duplicated across participants")
        result = {"profiles": args.profiles, "duration_seconds": args.duration, "started_at": started_at, "ended_at": datetime.datetime.now(datetime.timezone.utc).isoformat(), "sent": sent, "acked": acked, "disconnects": disconnects, "unique_posted_ids": unique_posted, "unique_readback_ids": unique_readback, "p50_ms": percentile(latencies, .50), "p95_ms": percentile(latencies, .95), "p99_ms": percentile(latencies, .99), "broker_rss_kib_max": max(rss_samples, default=0), "broker_cpu_ticks_max": max(cpu_samples, default=0), "broker_open_fds_max": max(fd_samples, default=0), "journal_bytes_max": max(journal_samples, default=0), "expected_min_messages_per_uid": expected_min, "errors": child_errors}
        replay_expected = marker_dir / "restart-expected.json"
        replay_expected.write_text(json.dumps({"posted_body_hashes": {message_id: digest for summary in summaries for message_id, digest in summary.get("posted_body_hashes", {}).items()}}, sort_keys=True))
        result["marker_dir"] = str(marker_dir)
        result["replay_expected"] = str(replay_expected)
        result["state_dir"] = str(state)
        result["socket"] = runtime["socket"]
        result["workspace"] = runtime["workspace_id"]
        print(json.dumps(result, sort_keys=True))
        if child_errors or sent == 0 or acked != sent or disconnects:
            return 1
        return 0
    finally:
        for child in children:
            if child.poll() is None:
                child.send_signal(signal.SIGTERM)
        for child in children:
            if child.poll() is None:
                child.wait(timeout=15)
        if not args.retain_state:
            runuser(owner, [str(binary), "stop", "--state-dir", str(state)], env={"HOME": str(owner_home)})


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", default="/usr/local/bin/biorouter-crew")
    parser.add_argument("--state-dir", default="/tmp/crew-real-uid-soak")
    parser.add_argument("--profiles", type=int, default=50)
    parser.add_argument("--duration", type=int, default=1800)
    parser.add_argument("--interval", type=int, default=60)
    parser.add_argument("--owner-bootstrap", action="store_true")
    parser.add_argument("--owner-team-invites", action="store_true")
    parser.add_argument("--participant-enroll", action="store_true")
    parser.add_argument("--participant-workload", action="store_true")
    parser.add_argument("--restart-replay", action="store_true")
    parser.add_argument("--expected")
    parser.add_argument("--retain-state", action="store_true")
    parser.add_argument("--socket")
    parser.add_argument("--workspace")
    parser.add_argument("--owner-uid", type=int)
    parser.add_argument("--uid", type=int)
    parser.add_argument("--seed")
    parser.add_argument("--invitation")
    parser.add_argument("--marker")
    parser.add_argument("--manifest")
    parser.add_argument("--channel")
    parser.add_argument("--metrics")
    args = parser.parse_args()
    try:
        if args.owner_bootstrap:
            owner_bootstrap(args, pathlib.Path(args.manifest))
            return 0
        if args.owner_team_invites:
            owner_team_invites(args, pathlib.Path(args.manifest))
            return 0
        if args.participant_enroll:
            return participant_enroll(args)
        if args.participant_workload:
            return participant_workload(args)
        if args.restart_replay:
            return restart_replay(args)
        return soak(args)
    except Exception as exc:
        print(f"real UID soak failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
