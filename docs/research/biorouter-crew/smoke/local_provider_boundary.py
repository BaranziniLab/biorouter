#!/usr/bin/env python3
"""Bounded local API evidence for the real Crew/provider privacy boundary.

This is separate from graphical G05 acceptance. It uses a fresh local broker
state, a strict isolated SSH profile, a synthetic public provider, and a
loopback HTTP sink. No external network or desktop credentials are used.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import pathlib
import shutil
import socket
import subprocess
import tempfile
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

PUBLIC_MARKER = "LUNA_PUBLIC_SAFE_FIXTURE"
PRIVATE_MARKER = "LUNA_PRIVATE_CREW_SECRET"


class SinkState:
    def __init__(self) -> None:
        self.lock = threading.Lock()
        self.bodies: list[bytes] = []


def make_sink_handler(state: SinkState):
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self) -> None:  # noqa: N802 - stdlib callback
            length = int(self.headers.get("Content-Length", "0"))
            body = self.rfile.read(length)
            with state.lock:
                state.bodies.append(body)
            response = json.dumps({
                "id": "luna-boundary", "object": "chat.completion", "created": 1,
                "model": "synthetic-model", "choices": [{"index": 0,
                "message": {"role": "assistant", "content": "PUBLIC_PROVIDER_OK"},
                "finish_reason": "stop"}],
            }, separators=(",", ":")).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(response)))
            self.end_headers()
            self.wfile.write(response)

        def log_message(self, *_args: object) -> None:
            return

    return Handler


def free_port() -> int:
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def http_json(base: str, secret: str, path: str, payload: dict | None = None,
              user_action: str | None = None, method: str | None = None) -> tuple[int, object]:
    data = None if payload is None else json.dumps(payload, separators=(",", ":")).encode()
    request = Request(f"{base}{path}", data=data, method=method or ("POST" if data is not None else "GET"))
    request.add_header("X-Secret-Key", secret)
    if user_action:
        request.add_header("X-User-Action", user_action)
    if data is not None:
        request.add_header("Content-Type", "application/json")
    try:
        with urlopen(request, timeout=20) as response:
            return response.status, json.loads(response.read())
    except HTTPError as error:
        raw = error.read()
        try:
            return error.code, json.loads(raw)
        except json.JSONDecodeError:
            return error.code, raw.decode(errors="replace")


def docker_exec(docker: str, container: str, command: str) -> str:
    return subprocess.check_output(
        [docker, "exec", container, "sh", "-lc", command], text=True, stderr=subprocess.STDOUT
    )


def docker_copy(docker: str, container: str, source: str, destination: str) -> None:
    subprocess.run(
        [docker, "cp", source, f"{container}:{destination}"],
        check=True,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.STDOUT,
    )


def fail(message: str, *details: object) -> None:
    raise RuntimeError(f"{message}: {' '.join(map(str, details))}")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--daemon", default="/private/tmp/biorouter-crew-target/debug/biorouterd")
    parser.add_argument("--docker", default="/usr/local/bin/docker")
    parser.add_argument("--container", default="biorouter-crew-ssh-luna")
    parser.add_argument("--broker-binary", default="/private/tmp/crew-linux-target/debug/biorouter-crew")
    parser.add_argument("--identity-file", default="/private/tmp/biorouter-crew-ssh-fixture/keys/alice")
    parser.add_argument("--known-hosts", default="/private/tmp/biorouter-crew-ssh-fixture/known_hosts")
    parser.add_argument("--ssh-port", type=int, default=56928)
    parser.add_argument("--scenario", choices=("core", "personal-private", "alias", "restricted-history"), default="core")
    args = parser.parse_args()
    fixture_binary = f"/home/alice/.local/bin/biorouter-crew-provider-boundary-{os.getpid()}"
    broker_sha256 = hashlib.sha256(pathlib.Path(args.broker_binary).read_bytes()).hexdigest()

    sink_state = SinkState()
    sink = ThreadingHTTPServer(("127.0.0.1", 0), make_sink_handler(sink_state))
    sink_thread = threading.Thread(target=sink.serve_forever, daemon=True)
    sink_thread.start()
    temp_root = pathlib.Path(tempfile.mkdtemp(prefix="crew-provider-", dir="/tmp"))
    short_tmp = temp_root / "tmp"
    short_tmp.mkdir(mode=0o700)
    profile_ssh = temp_root / "ssh-profile" / "home" / ".ssh"
    profile_ssh.mkdir(mode=0o700, parents=True)
    shutil.copyfile(args.known_hosts, profile_ssh / "known_hosts")
    (profile_ssh / "config").write_text(
        "Host *\nStrictHostKeyChecking yes\n"
        f"UserKnownHostsFile {profile_ssh / 'known_hosts'}\n"
        "IdentityAgent none\nIdentitiesOnly yes\n"
    )
    profile_root = temp_root / "biorouter"
    (profile_root / "work").mkdir(mode=0o700, parents=True)
    providers = profile_root / "config" / "custom_providers"
    providers.mkdir(mode=0o700, parents=True)
    sink_port = sink.server_address[1]
    (providers / "custom_luna_sink.json").write_text(json.dumps({
        "name": "custom_luna_sink", "engine": "openai", "display_name": "Luna sink",
        "description": "Synthetic local public provider", "api_key_env": "CUSTOM_LUNA_SINK_API_KEY",
        "base_url": f"http://127.0.0.1:{sink_port}/v1/chat/completions",
        "models": [{"name": "synthetic-model", "context_limit": 128000}],
        "supports_streaming": False,
    }))
    daemon_port = free_port()
    secret = "luna-daemon-secret"
    user_action = "synthetic-human-key"
    environment = os.environ.copy()
    environment.update({
        "TMPDIR": str(short_tmp), "BIOROUTER_PATH_ROOT": str(profile_root),
        "BIOROUTER_DISABLE_KEYRING": "true",
        "BIOROUTER_DEV_PROFILE_ROOT": str(temp_root / "ssh-profile"),
        "BIOROUTER_SERVER__SECRET_KEY": secret, "BIOROUTER_HOST": "127.0.0.1",
        "BIOROUTER_PORT": str(daemon_port), "BIOROUTER_PROVIDER": "custom_luna_sink",
        "BIOROUTER_MODEL": "synthetic-model", "CUSTOM_LUNA_SINK_API_KEY": "synthetic-public-key",
        "BIOROUTER_USER_ACTION_EXPECTED": "1",
    })
    daemon = subprocess.Popen(
        [args.daemon, "agent"], stdin=subprocess.PIPE, stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE, text=True, env=environment,
    )
    remote_state = f"/home/alice/.local/share/biorouter-crew/provider-boundary-{os.getpid()}"
    base = f"http://127.0.0.1:{daemon_port}"
    try:
        digest = hashlib.sha256(user_action.encode()).hexdigest()
        daemon.stdin.write(digest + "\n")
        daemon.stdin.close()
        ready = False
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if daemon.poll() is not None:
                diagnostic = daemon.stderr.read()[-2000:]
                fail("daemon exited during startup", diagnostic)
            try:
                status, _ = http_json(base, secret, "/config/providers")
                ready = status == 200
            except URLError:
                ready = False
            if ready:
                break
            time.sleep(0.25)
        if not ready:
            fail("daemon readiness timeout")

        status, prepared = http_json(base, secret, "/crew/devices/prepare", {}, user_action)
        if status != 200 or not isinstance(prepared, dict):
            fail("device preparation failed", status, prepared)
        key = prepared["public_key"]
        docker_copy(args.docker, args.container, args.broker_binary, fixture_binary)
        docker_exec(args.docker, args.container, f"chown alice:alice {fixture_binary}")
        docker_exec(args.docker, args.container, f"chmod 755 {fixture_binary}")
        observed_broker_sha256 = docker_exec(args.docker, args.container, f"sha256sum {fixture_binary}").split()[0]
        if observed_broker_sha256 != broker_sha256:
            fail("broker artifact hash changed during fixture install", broker_sha256, observed_broker_sha256)
        docker_exec(args.docker, args.container,
                    f"runuser -u alice -- sh -lc 'mkdir -p {remote_state}; chmod 700 {remote_state}; "
                    f"{fixture_binary} start --state-dir {remote_state} --bootstrap-key {key}'")
        runtime = None
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            try:
                runtime = json.loads(docker_exec(args.docker, args.container, f"cat {remote_state}/runtime.json"))
                break
            except (OSError, subprocess.CalledProcessError, json.JSONDecodeError):
                time.sleep(0.25)
        if not isinstance(runtime, dict):
            fail("remote broker runtime descriptor timeout")

        descriptor = {
            "preparation_id": prepared["preparation_id"], "name": "Luna provider boundary",
            "ssh_target": "alice@127.0.0.1", "port": args.ssh_port,
            "identity_file": args.identity_file, "proxy_jump": None,
            "socket_path": runtime["socket"], "owner_uid": runtime["host_uid"],
            "workspace_id": runtime["workspace_id"],
            "workspace_public_key": runtime["workspace_public_key"], "remote_root": None,
            "remote_execution": False, "cluster_connection_id": None,
            "mode": "public",
        }
        status, saved = http_json(base, secret, "/crew/connections", descriptor, user_action)
        if status != 200 or not isinstance(saved, dict):
            fail("connection save failed", status, saved)
        connection_id = saved["id"]
        status, connected = http_json(base, secret, f"/crew/connections/{connection_id}/connect", {}, user_action)
        if status != 200:
            fail("connection handshake failed", status, connected)

        crew_request_number = 0

        def crew(method: str, params: dict) -> tuple[int, object]:
            nonlocal crew_request_number
            crew_request_number += 1
            return http_json(base, secret, f"/crew/connections/{connection_id}/request",
                             {"method": method, "params": params, "request_id": f"luna-{method}-{crew_request_number}"}, user_action)

        for method, params in (
            ("auth.bootstrap", {"public_key": prepared["public_key"]}),
            ("policy.set", {"mode": "public", "idempotency_key": "luna-public"}),
        ):
            status, response = crew(method, params)
            if status != 200:
                fail(method, "failed", status, response)
        status, team = crew("team.create", {"name": "Luna provider boundary"})
        if status != 200 or not isinstance(team, dict):
            fail("team.create failed", status, team)
        channel_id = team["channel"]["id"]
        status, posted = crew("message.post", {
            "channel_id": channel_id, "body": PUBLIC_MARKER, "personal_mode": "public",
        })
        if status != 200 or not isinstance(posted, dict) or posted.get("restricted"):
            fail("public-safe post failed", status, posted)

        status, started = http_json(base, secret, f"/crew/connections/{connection_id}/runs", {
            "request_id": "luna-public-run", "channel_id": channel_id,
            "prompt": "Return PUBLIC_PROVIDER_OK for the public-safe fixture.",
            "provider": "custom_luna_sink", "model": "synthetic-model",
            "context_channels": [], "posting_grant": True,
        }, user_action)
        if status != 200 or not isinstance(started, dict):
            fail("public run admission failed", status, started)
        run_id = started["run_id"]
        final_run = None
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            status, runs = http_json(base, secret, f"/crew/connections/{connection_id}/runs", None, user_action)
            if status == 200 and isinstance(runs, dict):
                final_run = next((item for item in runs.get("runs", []) if item.get("run_id") == run_id), None)
                if final_run and final_run.get("status") in {"completed", "failed", "cancelled"}:
                    break
            time.sleep(0.25)
        with sink_state.lock:
            public_bodies = list(sink_state.bodies)
        if not final_run or final_run.get("status") != "completed":
            fail("public run did not complete", final_run)
        if len(public_bodies) != 1 or PUBLIC_MARKER.encode() not in public_bodies[0]:
            fail("public sink evidence invalid", len(public_bodies))
        if PRIVATE_MARKER.encode() in public_bodies[0]:
            fail("private marker leaked into public sink")

        if args.scenario == "personal-private":
            private_descriptor = dict(descriptor)
            private_descriptor.pop("preparation_id", None)
            private_descriptor["mode"] = "private"
            status, updated = http_json(base, secret, f"/crew/connections/{connection_id}", private_descriptor, user_action, method="PATCH")
            if status != 200 or not isinstance(updated, dict) or updated.get("mode") != "private":
                fail("personal mode update failed", status, updated)
            status, reconnected = http_json(base, secret, f"/crew/connections/{connection_id}/connect", {}, user_action)
            if status != 200:
                fail("private-mode reconnect failed", status, reconnected)
            status, refusal = http_json(base, secret, f"/crew/connections/{connection_id}/runs", {
                "request_id": "luna-personal-private", "channel_id": channel_id,
                "prompt": PRIVATE_MARKER, "provider": "custom_luna_sink", "model": "synthetic-model",
                "context_channels": [], "posting_grant": True,
            }, user_action)
            with sink_state.lock:
                bodies = list(sink_state.bodies)
            if status != 400 or not any(marker in json.dumps(refusal) for marker in ("privacy_denied", "Private cluster blocks public models")) or len(bodies) != 1 or any(PRIVATE_MARKER.encode() in body for body in bodies):
                fail("personal-private public run boundary invalid", status, refusal, len(bodies))
            print(json.dumps({"api_harness": "pass", "scenario": args.scenario,
                              "control_sink_count": len(public_bodies), "refusal": refusal, "sink_count": len(bodies),
                              "workspace_id": runtime["workspace_id"],
                              "daemon_sha256": hashlib.sha256(pathlib.Path(args.daemon).read_bytes()).hexdigest(),
                              "broker_sha256": broker_sha256}, sort_keys=True))
            return 0

        if args.scenario == "alias":
            alias_descriptor = dict(descriptor)
            alias_descriptor.pop("preparation_id", None)
            alias_descriptor["name"] = "Luna private alias"
            alias_descriptor["mode"] = "private"
            status, alias_saved = http_json(base, secret, "/crew/connections", alias_descriptor, user_action)
            if status != 200 or not isinstance(alias_saved, dict):
                fail("alias save failed", status, alias_saved)
            alias_id = alias_saved["id"]
            status, alias_connected = http_json(base, secret, f"/crew/connections/{alias_id}/connect", {}, user_action)
            if status != 200:
                fail("alias handshake failed", status, alias_connected)
            status, alias_refusal = http_json(base, secret, f"/crew/connections/{connection_id}/runs", {
                "request_id": "luna-alias-private", "channel_id": channel_id,
                "prompt": PRIVATE_MARKER, "provider": "custom_luna_sink", "model": "synthetic-model",
                "context_channels": [], "posting_grant": True,
            }, user_action)
            with sink_state.lock:
                alias_bodies = list(sink_state.bodies)
            if status != 400 or not any(marker in json.dumps(alias_refusal) for marker in ("privacy_denied", "Private cluster blocks public models")) or len(alias_bodies) != 1:
                fail("private alias boundary invalid", status, alias_refusal, len(alias_bodies))
            print(json.dumps({"api_harness": "pass", "scenario": args.scenario,
                              "alias_refusal": alias_refusal, "sink_count": len(alias_bodies),
                              "workspace_id": runtime["workspace_id"],
                              "daemon_sha256": hashlib.sha256(pathlib.Path(args.daemon).read_bytes()).hexdigest(),
                              "broker_sha256": broker_sha256}, sort_keys=True))
            return 0

        if args.scenario == "restricted-history":
            private_descriptor = dict(descriptor)
            private_descriptor.pop("preparation_id", None)
            private_descriptor["mode"] = "private"
            status, updated = http_json(base, secret, f"/crew/connections/{connection_id}", private_descriptor, user_action, method="PATCH")
            if status != 200 or not isinstance(updated, dict) or updated.get("mode") != "private":
                fail("restricted-history private mode update failed", status, updated)
            status, reconnected = http_json(base, secret, f"/crew/connections/{connection_id}/connect", {}, user_action)
            if status != 200:
                fail("restricted-history private-mode reconnect failed", status, reconnected)
            status, policy = crew("policy.set", {"mode": "private", "idempotency_key": "luna-restricted-private"})
            if status != 200:
                fail("restricted-history private policy transition failed", status, policy)
            status, restricted_post = crew("message.post", {
                "channel_id": channel_id, "body": PRIVATE_MARKER,
            })
            if status != 200 or not isinstance(restricted_post, dict) or not restricted_post.get("restricted"):
                fail("same-channel restricted history post failed", status, restricted_post)
            public_descriptor = dict(descriptor)
            public_descriptor.pop("preparation_id", None)
            status, updated = http_json(base, secret, f"/crew/connections/{connection_id}", public_descriptor, user_action, method="PATCH")
            if status != 200 or not isinstance(updated, dict) or updated.get("mode") != "public":
                fail("restricted-history public mode update failed", status, updated)
            status, reconnected = http_json(base, secret, f"/crew/connections/{connection_id}/connect", {}, user_action)
            if status != 200:
                fail("restricted-history public-mode reconnect failed", status, reconnected)
            status, policy = crew("policy.set", {"mode": "public", "idempotency_key": "luna-restricted-public"})
            if status != 200:
                fail("restricted-history public policy transition failed", status, policy)
            status, snapshot = crew("workspace.snapshot", {})
            channels = snapshot.get("channels", []) if isinstance(snapshot, dict) else []
            channel_view = next((item for item in channels if item.get("id") == channel_id), None)
            if status != 200 or not isinstance(channel_view, dict) or channel_view.get("classification") != "public_safe":
                fail("same-channel classification changed", status, channel_view)
            status, history_refusal = http_json(base, secret, f"/crew/connections/{connection_id}/runs", {
                "request_id": "luna-restricted-history", "channel_id": channel_id,
                "prompt": PRIVATE_MARKER, "provider": "custom_luna_sink", "model": "synthetic-model",
                "context_channels": [], "posting_grant": True,
            }, user_action)
            with sink_state.lock:
                history_bodies = list(sink_state.bodies)
            if status != 400 or "privacy_denied" not in json.dumps(history_refusal) or len(history_bodies) != 1 or any(PRIVATE_MARKER.encode() in body for body in history_bodies):
                fail("restricted-history boundary invalid", status, history_refusal, len(history_bodies))
            print(json.dumps({"api_harness": "pass", "scenario": args.scenario,
                              "history_refusal": history_refusal, "sink_count": len(history_bodies),
                              "workspace_id": runtime["workspace_id"],
                              "daemon_sha256": hashlib.sha256(pathlib.Path(args.daemon).read_bytes()).hexdigest(),
                              "broker_sha256": broker_sha256}, sort_keys=True))
            return 0

        status, policy = crew("policy.set", {"mode": "private", "idempotency_key": "luna-private"})
        if status != 200:
            fail("private policy transition failed", status, policy)
        status, refusal = http_json(base, secret, f"/crew/connections/{connection_id}/runs", {
            "request_id": "luna-private-run", "channel_id": channel_id, "prompt": PRIVATE_MARKER,
            "provider": "custom_luna_sink", "model": "synthetic-model",
            "context_channels": [], "posting_grant": True,
        }, user_action)
        with sink_state.lock:
            final_bodies = list(sink_state.bodies)
        if status != 400 or "privacy_denied" not in json.dumps(refusal) or len(final_bodies) != len(public_bodies):
            fail("private refusal boundary invalid", status, refusal, len(final_bodies))
        evidence = {
            "api_harness": "pass", "graphical_g05": "not_run", "public_run": final_run,
            "private_refusal": refusal, "sink_count": len(final_bodies),
            "sink_sha256": hashlib.sha256(final_bodies[0]).hexdigest(),
            "sink_bytes": len(final_bodies[0]), "workspace_id": runtime["workspace_id"],
            "daemon_sha256": hashlib.sha256(pathlib.Path(args.daemon).read_bytes()).hexdigest(),
            "broker_sha256": broker_sha256,
        }
        print(json.dumps(evidence, sort_keys=True))
        return 0
    finally:
        try:
            docker_exec(args.docker, args.container,
                        f"runuser -u alice -- {fixture_binary} stop --state-dir {remote_state}")
        except (OSError, subprocess.CalledProcessError):
            pass
        try:
            docker_exec(args.docker, args.container, f"rm -f {fixture_binary}")
        except (OSError, subprocess.CalledProcessError):
            pass
        daemon.terminate()
        try:
            daemon.wait(timeout=10)
        except subprocess.TimeoutExpired:
            daemon.kill()
            daemon.wait(timeout=5)
        if daemon.stderr:
            daemon.stderr.close()
        sink.shutdown()
        sink.server_close()
        shutil.rmtree(temp_root, ignore_errors=True)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, URLError, subprocess.CalledProcessError) as error:
        print(f"provider boundary failed: {error}")
        raise SystemExit(1)
