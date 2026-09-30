#!/usr/bin/env python3
"""Native release-broker protocol/confinement acceptance; no provider or model calls."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import pwd
import select
import sqlite3
import subprocess
import time
import uuid


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def run(argv, **options):
    return subprocess.run(argv, check=True, capture_output=True, **options)


class Person:
    def __init__(self, name, root):
        self.name = name
        self.account = pwd.getpwnam(name)
        self.home = Path(self.account.pw_dir)
        self.key = root / (name + ".pem")
        run(["openssl", "genpkey", "-algorithm", "Ed25519", "-out", str(self.key)])
        self.key.chmod(0o600)
        public = run(["openssl", "pkey", "-in", str(self.key), "-pubout", "-outform", "DER"]).stdout
        require(len(public) == 44, "Unexpected Ed25519 public encoding")
        self.public = public[-32:].hex()
        self.device = hashlib.sha256(public[-32:]).hexdigest()
        self.payload = root / (name + "-payload.json")
        self.bridge = None

    def argv(self, arguments):
        return ["runuser", "-u", self.name, "--", "env", "-i", "HOME=" + str(self.home),
                "PATH=/usr/bin:/bin", "TMPDIR=/tmp", *map(str, arguments)]

    def command(self, arguments):
        return run(self.argv(arguments))

    def connect(self, binary, runtime):
        self.bridge = subprocess.Popen(self.argv([binary, "bridge", "--stdio", "--socket",
                                                  runtime["socket"], "--owner-uid",
                                                  runtime["host_uid"], "--workspace-id",
                                                  runtime["workspace_id"]]),
                                       stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                       stderr=subprocess.DEVNULL, start_new_session=True)
        self.workspace = runtime["workspace_id"]
        hello = self.call("hello", {})
        require(hello["workspace_id"] == self.workspace, "Bridge workspace mismatch")
        require(hello["host_uid"] == runtime["host_uid"], "Bridge host mismatch")
        return hello

    def frame(self, method, params, auth=None, credential=None):
        identifier = str(uuid.uuid4())
        frame = dict(version=1, id=identifier, method=method, params=params,
                     auth=auth, credential=credential)
        self.bridge.stdin.write(encoded(frame) + b"\n")
        self.bridge.stdin.flush()
        require(select.select([self.bridge.stdout], [], [], 40)[0], "Bridge response timed out")
        response = json.loads(self.bridge.stdout.readline())
        require(response.get("id") == identifier, "Bridge response identity mismatch")
        return response

    def call(self, method, params, **options):
        response = self.frame(method, params, **options)
        require("error" not in response, "Protocol request refused: " + method)
        require("result" in response, "Protocol response missing result")
        return response["result"]

    def signed(self, method, params):
        params = {"idempotency_key": str(uuid.uuid4()), **params}
        nonce = self.call("auth.challenge", {"device_id": self.device})["nonce"]
        self.payload.write_bytes(encoded([self.workspace, self.account.pw_uid, nonce, method, params]))
        self.payload.chmod(0o600)
        signature = run(["openssl", "pkeyutl", "-sign", "-rawin", "-inkey", str(self.key),
                         "-in", str(self.payload)]).stdout.hex()
        return self.call(method, params, auth=dict(device_id=self.device, nonce=nonce,
                                                   signature=signature))

    def close(self):
        if self.bridge is not None:
            self.bridge.stdin.close()
            try:
                self.bridge.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.bridge.terminate()
                self.bridge.wait(timeout=5)


def create_person(name, uid, root):
    try:
        pwd.getpwnam(name)
    except KeyError:
        pass
    else:
        raise RuntimeError("Acceptance account already exists")
    try:
        pwd.getpwuid(uid)
    except KeyError:
        pass
    else:
        raise RuntimeError("Acceptance UID already exists")
    home = root / name
    run(["useradd", "--uid", str(uid), "--create-home", "--home-dir", str(home),
         "--shell", "/bin/bash", name])
    home.chmod(0o700)
    return Person(name, root)


def wait_runtime(state):
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        runtime = state / "runtime.json"
        if runtime.exists():
            return json.loads(runtime.read_bytes())
        time.sleep(0.1)
    raise RuntimeError("Broker runtime publication timed out")


def cleanup(alice, bob, binary, state, runtime):
    try:
        alice.close()
    finally:
        try:
            bob.close()
        finally:
            if runtime is not None:
                stopped = json.loads(alice.command([binary, "stop", "--state-dir", state]).stdout)
                require(stopped.get("stopped") is True and stopped.get("pid") == runtime["pid"],
                        "Exact broker cleanup failed")


def accept(binary, root, report, source_sha, run_id):
    require(os.geteuid() == 0 and os.uname().machine == "x86_64", "Native root-owned Linux CI required")
    require(not root.exists(), "Acceptance root must be fresh")
    root.mkdir(mode=0o755)
    require(binary.is_file() and not binary.is_symlink(), "Exact broker file required")
    require(run([str(binary), "--version"]).stdout.strip() == b"biorouter-crew 1.92.0", "Broker version mismatch")
    binary_hash = hashlib.sha256(binary.read_bytes()).hexdigest()
    alice = create_person("crew_ci_alice", 64001, root)
    bob = create_person("crew_ci_bob", 64002, root)
    state = alice.home / "authority"
    work = alice.home / "work"
    for path in (state, work):
        path.mkdir(mode=0o700)
        os.chown(path, alice.account.pw_uid, alice.account.pw_gid)
    database = work / "health.sqlite"
    with sqlite3.connect(database) as db:
        db.execute("CREATE TABLE release_health(id INTEGER PRIMARY KEY, value INTEGER NOT NULL)")
        db.execute("INSERT INTO release_health VALUES(1,1)")
    database.chmod(0o600)
    os.chown(database, alice.account.pw_uid, alice.account.pw_gid)
    database_hash = hashlib.sha256(database.read_bytes()).hexdigest()
    runtime = None
    evidence = None
    try:
        alice.command([binary, "start", "--name", "native-release-acceptance", "--state-dir", state,
                       "--bootstrap-key", alice.public])
        runtime = wait_runtime(state)
        require(runtime["host_uid"] == alice.account.pw_uid, "Broker owner mismatch")
        require(Path("/proc").joinpath(str(runtime["pid"]), "exe").resolve() == binary, "Broker executable mismatch")
        alice.connect(binary, runtime)
        alice.signed("auth.bootstrap", {"public_key": alice.public})
        alice.signed("policy.set", {"mode": "private", "institution_id": "ci-synthetic"})
        invitation = alice.signed("enrollment.invite", {"uid": bob.account.pw_uid, "public_key": bob.public})
        bob.connect(binary, runtime)
        admitted = bob.signed("auth.enroll", {"public_key": bob.public, "invitation": invitation["invitation"]})
        team = alice.signed("team.create", {"name": "release-db"})
        channel = team["channel"]["id"]
        alice.signed("team.add_member", {"team_id": team["team"]["id"],
                                          "principal_id": admitted["principal"]["id"],
                                          "expected_username": bob.name})
        hello = alice.call("hello", {})
        consent = dict(channel_id=channel, source_channels=[channel], provider_policy_id="native-protocol-no-model",
                       provider_affiliation={"kind": "local"}, personal_mode="private", public_provider=False,
                       workspace_institution_id=hello["institution_id"], connection_institution_id=hello["institution_id"],
                       expected_workspace_policy_epoch=hello["policy_epoch"], expected_protected_context=True,
                       remote_root=str(work), remote_execution=True, expires_in=180)
        denied = alice.frame("remote.execute", {"argv": ["/usr/bin/python3", "-V"]})
        require(denied.get("error", {}).get("code") == "remote_operation_denied" and
                "scoped worker credential" in denied["error"]["message"],
                "Remote execution did not prove missing-grant refusal")
        grant = alice.signed("run.create", consent)
        credential = grant["credential"]
        script = "import sqlite3,json,socket; d=sqlite3.connect('file:health.sqlite?mode=ro&immutable=1',uri=True); r={'health_check':d.execute('SELECT 1').fetchone()[0],'stored_value':d.execute('SELECT value FROM release_health WHERE id=1').fetchone()[0]};\ntry: socket.socket(); r['socket_creation_denied']=False\nexcept PermissionError: r['socket_creation_denied']=True\ntry: open('/etc/passwd').read(); r['outside_read_denied']=False\nexcept PermissionError: r['outside_read_denied']=True\nprint(json.dumps(r))"
        params = dict(argv=["/usr/bin/python3", "-I", "-S", "-c", script], timeout_seconds=30,
                      idempotency_key=str(uuid.uuid4()))
        job = alice.call("remote.execute", params, credential=credential)
        deadline = time.monotonic() + 35
        while job["status"] in ("running", "starting") and time.monotonic() < deadline:
            time.sleep(0.1)
            job = alice.call("remote.job_status", {"job_id": job["job_id"]}, credential=credential)
        require(job["status"] == "completed" and job["exit_code"] == 0, "Confined SQLite job did not complete")
        result = json.loads(job["stdout"])
        require(result == dict(health_check=1, stored_value=1, socket_creation_denied=True, outside_read_denied=True),
                "Actual confined SQL/denial result mismatch")
        require(job["stderr"] == "", "Confined SQLite job emitted unexpected stderr")
        body = encoded(result).decode()
        alice.call("run.project", dict(body=body, status="completed", idempotency_key=str(uuid.uuid4())),
                   credential=credential)
        history = bob.signed("messages.history", {"channel_id": channel, "latest": True, "limit": 10})
        require(any(message["body"] == body for message in history["messages"]), "Bob did not receive actual SQL result")
        alice.signed("run.revoke", {"run_id": grant["run"]["id"]})
        revoked = alice.frame("remote.execute", {**params, "idempotency_key": str(uuid.uuid4())}, credential=credential)
        require(revoked.get("error", {}).get("code") == "remote_operation_denied" and
                "grant_expired" in revoked["error"]["message"],
                "Remote execution did not prove revoked-grant refusal")
        require(hashlib.sha256(database.read_bytes()).hexdigest() == database_hash, "Read-only SQLite fixture changed")
        evidence = dict(scope="native-signed-protocol-and-confinement-no-model",
                                        sourceSha=source_sha, releaseRunId=run_id, brokerSha256=binary_hash,
                                        realUnixAccounts=True, explicitSignedUserGrant=True,
                                        actualRemoteJobReceipt={"job_id": job["job_id"], "status": job["status"],
                                                                "exit_code": job["exit_code"], "stdout": job["stdout"],
                                                                "stderr": job["stderr"]},
                                        sqlResult=result, bobHistoryMatched=True,
                        noGrantDenied=True, revokedGrantDenied=True, sqliteUnchanged=True)
    finally:
        cleanup(alice, bob, binary, state, runtime)
    require(evidence is not None, "Acceptance evidence missing")
    evidence["exactBrokerCleanup"] = True
    report.write_text(json.dumps(evidence, indent=2) + "\n")
    print("NATIVE_CREW_SQLITE_PROTOCOL_ACCEPTANCE_PASS")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--broker", type=Path, required=True)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    parser.add_argument("--source-sha", required=True)
    parser.add_argument("--release-run-id", required=True)
    args = parser.parse_args()
    accept(args.broker.absolute(), args.root, args.report, args.source_sha, args.release_run_id)
