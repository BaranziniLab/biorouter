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
import re
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
    def __init__(self, hold_stream: bool = False) -> None:
        self.lock = threading.Lock()
        self.bodies: list[bytes] = []
        self.hold_stream = hold_stream
        self.stream_started = threading.Event()
        self.release_stream = threading.Event()


def make_sink_handler(state: SinkState):
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self) -> None:  # noqa: N802 - stdlib callback
            length = int(self.headers.get("Content-Length", "0"))
            body = self.rfile.read(length)
            with state.lock:
                state.bodies.append(body)
            if state.hold_stream:
                ready = (
                    'data: {"id":"luna-cancel","object":"chat.completion.chunk",'
                    '"created":1,"model":"synthetic-model","choices":[{"index":0,'
                    '"delta":{"role":"assistant","content":"CANCEL_STREAM_READY"},'
                    '"finish_reason":null}]}\n\n'
                ).encode()
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Cache-Control", "no-cache")
                self.send_header("Connection", "keep-alive")
                self.end_headers()
                self.wfile.write(ready)
                self.wfile.flush()
                state.stream_started.set()
                state.release_stream.wait(timeout=20)
                final = (
                    'data: {"id":"luna-cancel","object":"chat.completion.chunk",'
                    '"created":1,"model":"synthetic-model","choices":[{"index":0,'
                    '"delta":{"content":"CANCEL_STREAM_FINAL"},"finish_reason":"stop"}]}\n\n'
                    "data: [DONE]\n\n"
                ).encode()
                self.wfile.write(final)
                self.wfile.flush()
                return
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
              user_action: str | None = None, method: str | None = None,
              caller_provider: str | None = None) -> tuple[int, object]:
    data = None if payload is None else json.dumps(payload, separators=(",", ":")).encode()
    request = Request(f"{base}{path}", data=data, method=method or ("POST" if data is not None else "GET"))
    request.add_header("X-Secret-Key", secret)
    if user_action:
        request.add_header("X-User-Action", user_action)
    if caller_provider:
        request.add_header("X-Caller-Provider", caller_provider)
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


def sanitize_evidence(value: object) -> object:
    """Remove credential-shaped fields before retaining a session response."""
    if isinstance(value, dict):
        sanitized: dict[str, object] = {}
        for key, item in value.items():
            lowered = key.lower()
            if any(marker in lowered for marker in ("secret", "password", "token", "private_key", "identity_file")):
                sanitized[key] = "[REDACTED]"
            else:
                sanitized[key] = sanitize_evidence(item)
        return sanitized
    if isinstance(value, list):
        return [sanitize_evidence(item) for item in value]
    return value


def extract_private_ollama_evidence(session: object) -> dict[str, object]:
    """Extract a typed Crew tool trace from persisted session JSON.

    Conversation serializes as a JSON array, and tool responses normally have
    the user role even though their matching request is assistant-owned. Keep
    this parser structural so prompt text containing remote.read or CSV-looking
    values cannot satisfy the acceptance check.
    """
    if not isinstance(session, dict):
        raise ValueError("session response is not an object")
    conversation = session.get("conversation")
    if not isinstance(conversation, list) or not conversation:
        raise ValueError("session conversation is not a non-empty message array")

    requests: dict[str, dict[str, object]] = {}
    responses: dict[str, dict[str, object]] = {}
    assistant_text: list[str] = []
    for message in conversation:
        if not isinstance(message, dict):
            continue
        role = message.get("role")
        contents = message.get("content")
        if not isinstance(contents, list):
            continue
        for content in contents:
            if not isinstance(content, dict):
                continue
            kind = content.get("type")
            if kind == "text" and role == "assistant" and isinstance(content.get("text"), str):
                assistant_text.append(content["text"])
            elif kind == "toolRequest" and role == "assistant":
                request_id = content.get("id")
                envelope = content.get("toolCall")
                value = envelope.get("value") if isinstance(envelope, dict) else None
                if (isinstance(request_id, str) and isinstance(envelope, dict)
                        and envelope.get("status") == "success" and isinstance(value, dict)):
                    requests[request_id] = {"id": request_id, "call": value}
            elif kind == "toolResponse":
                response_id = content.get("id")
                envelope = content.get("toolResult")
                value = envelope.get("value") if isinstance(envelope, dict) else None
                if (isinstance(response_id, str) and isinstance(envelope, dict)
                        and envelope.get("status") == "success" and isinstance(value, dict)):
                    responses[response_id] = {"id": response_id, "result": value}

    matched: list[tuple[dict[str, object], dict[str, object]]] = []
    for request_id, request in requests.items():
        call = request["call"]
        if not isinstance(call, dict):
            continue
        name = call.get("name")
        arguments = call.get("arguments")
        params = arguments.get("params") if isinstance(arguments, dict) else None
        if name not in {"request", "crew__request"} or not isinstance(arguments, dict):
            continue
        if arguments.get("method") != "remote.read" or not isinstance(params, dict):
            continue
        if params.get("path") != "input.csv":
            continue
        response = responses.get(request_id)
        if response is not None:
            matched.append((request, response))

    csv_results: list[dict[str, object]] = []
    for _, response in matched:
        result = response["result"]
        if not isinstance(result, dict) or result.get("isError") is True:
            continue
        for item in result.get("content", []):
            if not isinstance(item, dict) or item.get("type") != "text":
                continue
            text = item.get("text")
            if not isinstance(text, str):
                continue
            if text.startswith("<tool-output ") and text.endswith("</tool-output>"):
                _, _, text = text.partition("\n")
                text = text.removesuffix("</tool-output>").rstrip()
            try:
                decoded = json.loads(text)
            except json.JSONDecodeError:
                continue
            if (isinstance(decoded, dict) and decoded.get("path") == "input.csv"
                    and decoded.get("text_utf8") == "item,amount\nA,10\nB,20\nC,30\n"
                    and decoded.get("size") == len("item,amount\nA,10\nB,20\nC,30\n")):
                csv_results.append(decoded)

    final_text = "\n".join(assistant_text)
    if len(matched) != 1 or len(csv_results) != 1:
        raise ValueError(
            "typed Crew trace incomplete: "
            f"messages={len(conversation)} requests={len(requests)} "
            f"responses={len(responses)} matched_remote_reads={len(matched)} "
            f"csv_results={len(csv_results)}"
        )
    if (not re.search(r"(?<![0-9])rows\s*=\s*3(?![0-9])", final_text)
            or not re.search(r"(?<![0-9])total\s*=\s*60(?![0-9])", final_text)):
        raise ValueError(f"assistant completion omitted rows=3 total=60: {final_text!r}")
    request, _ = matched[0]
    return {
        "message_count": len(conversation),
        "request_id": request["id"],
        "tool_name": request["call"]["name"],
        "remote_read_request": request["call"],
        "remote_read_response": csv_results[0],
        "assistant_completion": final_text,
    }


def parser_self_test() -> None:
    """Exercise array shape, typed matching, and prompt-text rejection offline."""
    csv_text = "item,amount\nA,10\nB,20\nC,30\n"
    session = {
        "conversation": [
            {"role": "user", "content": [{"type": "text", "text": "remote.read item,amount A,10 B,20 C,30"}]},
            {"role": "assistant", "content": [{
                "type": "toolRequest", "id": "crew-read-1",
                "toolCall": {"status": "success", "value": {
                    "name": "request", "arguments": {"method": "remote.read", "params": {"path": "input.csv"}}
                }},
            }]},
            {"role": "user", "content": [{
                "type": "toolResponse", "id": "crew-read-1",
                "toolResult": {"status": "success", "value": {"isError": False, "content": [
                    {"type": "text", "text": "<tool-output untrusted=\"true\" tool=\"crew__request\">\n"
                        + json.dumps({"path": "input.csv", "text_utf8": csv_text, "size": len(csv_text)})
                        + "\n</tool-output>"}
                ]}},
            }]},
            {"role": "assistant", "content": [{"type": "text", "text": "rows=3 total=60"}]},
        ],
    }
    evidence = extract_private_ollama_evidence(session)
    if evidence["request_id"] != "crew-read-1":
        raise AssertionError(evidence)
    rejected = json.loads(json.dumps(session))
    rejected["conversation"][1]["content"][0]["id"] = "unmatched"
    try:
        extract_private_ollama_evidence(rejected)
    except ValueError:
        pass
    else:
        raise AssertionError("unmatched tool response was accepted")
    try:
        extract_private_ollama_evidence({"conversation": session["conversation"][:1]})
    except ValueError:
        pass
    else:
        raise AssertionError("prompt-only conversation was accepted")
    wrong_numbers = json.loads(json.dumps(session))
    wrong_numbers["conversation"][-1]["content"][0]["text"] = "rows=30 total=600"
    try:
        extract_private_ollama_evidence(wrong_numbers)
    except ValueError:
        pass
    else:
        raise AssertionError("rows=30 total=600 passed exact-result validation")
    for malformed in ({}, {"conversation": {"messages": session["conversation"]}}, {"conversation": []}):
        try:
            extract_private_ollama_evidence(malformed)
        except ValueError:
            continue
        raise AssertionError(f"malformed session was accepted: {malformed!r}")


def validate_broker_binary(raw_path: str | None) -> pathlib.Path:
    """Reject missing or host-native broker artifacts before fixture setup."""
    if not raw_path:
        raise RuntimeError(
            "--broker-binary is required; provide the path to the Linux ELF64 "
            "biorouter-crew artifact (for example, /path/to/biorouter-crew)."
        )
    path = pathlib.Path(raw_path).expanduser()
    if not path.is_file():
        raise RuntimeError(
            f"broker binary does not exist: {path}; pass --broker-binary with "
            "the Linux ELF64 artifact, not the macOS build."
        )
    try:
        header = path.read_bytes()[:20]
    except OSError as error:
        raise RuntimeError(f"cannot read broker binary {path}: {error}") from error
    if len(header) < 20 or header[:4] != b"\x7fELF":
        raise RuntimeError(
            f"broker binary is not an ELF file: {path}; pass --broker-binary "
            "with the Linux ELF64 artifact, not the macOS build."
        )
    elf_class, data_encoding = header[4], header[5]
    machine = int.from_bytes(header[18:20], byteorder="little")
    if elf_class != 2 or data_encoding != 1 or machine not in {0x3E, 0xB7}:
        raise RuntimeError(
            f"broker binary is not a supported little-endian ELF64 (x86_64 or aarch64): {path} "
            f"(class={elf_class}, data={data_encoding}, machine=0x{machine:x}); "
            "pass --broker-binary with the Linux ELF64 artifact for the fixture architecture."
        )
    return path


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--daemon", default="/private/tmp/biorouter-crew-target/debug/biorouterd")
    parser.add_argument("--docker", default="/usr/local/bin/docker")
    parser.add_argument("--container", default="biorouter-crew-ssh-luna")
    parser.add_argument("--broker-binary", default=None,
                        help="required Linux ELF64 biorouter-crew artifact (x86_64 or aarch64)")
    parser.add_argument("--identity-file", default="/private/tmp/biorouter-crew-ssh-fixture/keys/alice")
    parser.add_argument("--known-hosts", default="/private/tmp/biorouter-crew-ssh-fixture/known_hosts")
    parser.add_argument("--ssh-port", type=int, default=56928)
    parser.add_argument("--parser-self-test", action="store_true")
    parser.add_argument("--evidence-dir", type=pathlib.Path)
    parser.add_argument("--scenario", choices=("core", "personal-private", "alias", "restricted-history", "private-ollama", "cancellation-http"), default="core")
    args = parser.parse_args()
    if args.parser_self_test:
        parser_self_test()
        print(json.dumps({"parser_self_test": "pass"}, sort_keys=True))
        return 0
    broker_binary = validate_broker_binary(args.broker_binary)
    fixture_binary = f"/home/alice/.local/bin/biorouter-crew-provider-boundary-{os.getpid()}"
    broker_sha256 = hashlib.sha256(broker_binary.read_bytes()).hexdigest()

    sink_state = SinkState(hold_stream=args.scenario == "cancellation-http")
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
        "supports_streaming": args.scenario == "cancellation-http",
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
    if args.scenario == "private-ollama":
        environment.update({
            "BIOROUTER_PROVIDER": "ollama",
            "BIOROUTER_MODEL": "qwen3:8b",
            "OLLAMA_HOST": "http://127.0.0.1:11434",
            "OLLAMA_TIMEOUT": "120",
        })
    daemon = subprocess.Popen(
        [args.daemon, "agent"], stdin=subprocess.PIPE, stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE, text=True, env=environment,
    )
    remote_state = f"/home/alice/.local/share/biorouter-crew/provider-boundary-{os.getpid()}"
    fixture_remote_root: str | None = None
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
        docker_copy(args.docker, args.container, str(broker_binary), fixture_binary)
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

        private_ollama = args.scenario == "private-ollama"
        if private_ollama:
            fixture_remote_root = f"/home/alice/work/provider-boundary-{os.getpid()}"
            docker_exec(args.docker, args.container, f"runuser -u alice -- mkdir -p {fixture_remote_root}")
            docker_exec(
                args.docker,
                args.container,
                f"runuser -u alice -- sh -lc 'printf \"item,amount\\nA,10\\nB,20\\nC,30\\n\" > {fixture_remote_root}/input.csv'",
            )
        descriptor = {
            "preparation_id": prepared["preparation_id"], "name": "Luna provider boundary",
            "ssh_target": "alice@127.0.0.1", "port": args.ssh_port,
            "identity_file": args.identity_file, "proxy_jump": None,
            "socket_path": runtime["socket"], "owner_uid": runtime["host_uid"],
            "workspace_id": runtime["workspace_id"],
            "workspace_public_key": runtime["workspace_public_key"],
            "remote_root": fixture_remote_root,
            "remote_execution": private_ollama,
            "cluster_connection_id": None,
            "mode": "private" if private_ollama else "public",
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
            ("policy.set", {"mode": "private" if private_ollama else "public", "idempotency_key": "luna-private" if private_ollama else "luna-public"}),
        ):
            status, response = crew(method, params)
            if status != 200:
                fail(method, "failed", status, response)
        status, team = crew("team.create", {"name": "Luna provider boundary"})
        if status != 200 or not isinstance(team, dict):
            fail("team.create failed", status, team)
        channel_id = team["channel"]["id"]
        if args.scenario == "cancellation-http":
            def collect_finish_events(session_id: str, events: list[dict[str, object]], errors: list[str]) -> None:
                request = Request(f"{base}/sessions/{session_id}/events")
                request.add_header("X-Secret-Key", secret)
                request.add_header("X-User-Action", user_action)
                try:
                    with urlopen(request, timeout=25) as response:
                        deadline = time.monotonic() + 20
                        while time.monotonic() < deadline:
                            line = response.readline()
                            if not line:
                                break
                            if not line.startswith(b"data: "):
                                continue
                            try:
                                event = json.loads(line[6:])
                            except json.JSONDecodeError:
                                continue
                            if isinstance(event, dict):
                                events.append(event)
                                if event.get("type") == "Finish":
                                    return
                except (OSError, URLError) as error:
                    errors.append(str(error))

            def find_run(run_id: str, connection: str = connection_id) -> dict[str, object] | None:
                status, runs = http_json(base, secret, f"/crew/connections/{connection}/runs", None, user_action)
                if status != 200 or not isinstance(runs, dict):
                    return None
                return next((item for item in runs.get("runs", [])
                             if isinstance(item, dict) and item.get("run_id") == run_id), None)

            def wait_for_run(run_id: str, terminal: set[str] | None = None) -> dict[str, object]:
                deadline = time.monotonic() + 25
                latest: dict[str, object] | None = None
                while time.monotonic() < deadline:
                    latest = find_run(run_id)
                    if latest and (terminal is None or latest.get("status") in terminal):
                        return latest
                    time.sleep(0.2)
                fail("run did not reach expected state", run_id, latest)

            status, started = http_json(base, secret, f"/crew/connections/{connection_id}/runs", {
                "request_id": "luna-cancel-held", "channel_id": channel_id,
                "prompt": "Hold the deterministic local provider stream until cancellation, then return its final marker.",
                "provider": "custom_luna_sink", "model": "synthetic-model",
                "context_channels": [], "posting_grant": True,
            }, user_action)
            if status != 200 or not isinstance(started, dict):
                fail("held cancellation run admission failed", status, started)
            held_run_id = started["run_id"]
            held_run = wait_for_run(held_run_id)
            session_id = held_run.get("session_id")
            if held_run.get("status") not in {"starting", "running"} or not isinstance(session_id, str):
                fail("held run did not become active", held_run)
            if not sink_state.stream_started.wait(timeout=20):
                fail("synthetic stream never reached its held point")

            events: list[dict[str, object]] = []
            event_errors: list[str] = []
            event_thread = threading.Thread(
                target=collect_finish_events, args=(session_id, events, event_errors), daemon=True,
            )
            event_thread.start()
            time.sleep(0.3)
            wrong_status, wrong_body = http_json(
                base, secret, f"/crew/connections/wrong-luna-owner/runs/{held_run_id}/cancel",
                {}, user_action,
            )
            if wrong_status < 400 or "not owned by this device and connection" not in json.dumps(wrong_body):
                fail("owner-mismatch cancellation was accepted or lacked its refusal", wrong_status, wrong_body)
            cancel_status, cancel_body = http_json(
                base, secret, f"/crew/connections/{connection_id}/runs/{held_run_id}/cancel",
                {}, user_action,
            )
            if (cancel_status != 200 or not isinstance(cancel_body, dict)
                    or cancel_body.get("status") != "cancelled"
                    or cancel_body.get("cancelled") is not True
                    or cancel_body.get("remote_revocation_confirmed") is not True):
                fail("active cancellation was not confirmed", cancel_status, cancel_body)
            sink_state.release_stream.set()
            cancelled_run = wait_for_run(held_run_id, {"cancelled"})
            event_thread.join(timeout=20)
            if event_thread.is_alive() or event_errors:
                fail("session observer did not finish cleanly", events, event_errors)
            finish_events = [event for event in events if event.get("type") == "Finish"]
            if len(finish_events) != 1 or finish_events[0].get("reason") != "cancelled":
                fail("session observer did not match the cancelled terminal event", events)
            time.sleep(0.5)
            late_run = find_run(held_run_id)
            if not late_run or late_run.get("status") != "cancelled":
                fail("late provider completion overwrote cancellation", late_run)

            status, completed = http_json(base, secret, f"/crew/connections/{connection_id}/runs", {
                "request_id": "luna-cancel-complete-first", "channel_id": channel_id,
                "prompt": "Return the deterministic local provider response immediately.",
                "provider": "custom_luna_sink", "model": "synthetic-model",
                "context_channels": [], "posting_grant": True,
            }, user_action)
            if status != 200 or not isinstance(completed, dict):
                fail("completion-first run admission failed", status, completed)
            completed_run_id = completed["run_id"]
            completed_view = wait_for_run(completed_run_id, {"completed"})
            finished_cancel_status, finished_cancel_body = http_json(
                base, secret, f"/crew/connections/{connection_id}/runs/{completed_run_id}/cancel",
                {}, user_action,
            )
            if (finished_cancel_status != 200 or not isinstance(finished_cancel_body, dict)
                    or finished_cancel_body.get("already_finished") is not True
                    or finished_cancel_body.get("status") != "completed"
                    or finished_cancel_body.get("cancelled") is not False):
                fail("completion-before-cancellation was not reported honestly", finished_cancel_status, finished_cancel_body)
            print(json.dumps({
                "api_harness": "pass", "scenario": args.scenario,
                "daemon_sha256": hashlib.sha256(pathlib.Path(args.daemon).read_bytes()).hexdigest(),
                "broker_sha256": broker_sha256,
                "owner_mismatch": {"status": wrong_status, "body": wrong_body},
                "active_cancel": {"status": cancel_status, "body": cancel_body,
                                  "run": cancelled_run, "finish_event": finish_events[0]},
                "completion_first": {"run": completed_view, "cancel": finished_cancel_body},
                "sink_requests": len(sink_state.bodies),
                "workspace_id": runtime["workspace_id"],
            }, sort_keys=True))
            return 0
        if private_ollama:
            status, started = http_json(base, secret, f"/crew/connections/{connection_id}/runs", {
                "request_id": "luna-private-ollama", "channel_id": channel_id,
                "prompt": "/no_think\nUse the granted Crew tool exactly once with method remote.read and params {path: input.csv}. Do not use remote.execute. Parse only the returned CSV data, count its data rows, sum its amount column, and report exactly in the format rows=N total=M. If the tool fails, report the failure and do not invent file contents.",
                "provider": "ollama", "model": "qwen3:8b",
                "context_channels": [], "posting_grant": True,
            }, user_action)
            if status != 200 or not isinstance(started, dict):
                fail("private Ollama run admission failed", status, started)
            run_id = started["run_id"]
            final_run = None
            deadline = time.monotonic() + 240
            while time.monotonic() < deadline:
                status, runs = http_json(base, secret, f"/crew/connections/{connection_id}/runs", None, user_action)
                if status == 200 and isinstance(runs, dict):
                    final_run = next((item for item in runs.get("runs", []) if item.get("run_id") == run_id), None)
                    if final_run and final_run.get("status") in {"completed", "failed", "cancelled"}:
                        break
                time.sleep(1)
            if not final_run:
                fail("private Ollama run did not reach a terminal record", final_run)
            session_id = final_run.get("session_id")
            if not isinstance(session_id, str):
                fail("private Ollama run omitted session id", final_run)
            status, session = http_json(
                base,
                secret,
                f"/sessions/{session_id}",
                None,
                user_action,
                caller_provider="ollama",
            )
            if status != 200 or not isinstance(session, dict):
                fail("private Ollama session retrieval failed", status, session)
            if args.evidence_dir:
                args.evidence_dir.mkdir(mode=0o700, parents=True, exist_ok=True)
                (args.evidence_dir / "private-ollama-run.json").write_text(
                    json.dumps(final_run, sort_keys=True, indent=2) + "\n"
                )
                (args.evidence_dir / "private-ollama-session.json").write_text(
                    json.dumps(sanitize_evidence(session), sort_keys=True, indent=2) + "\n"
                )
            if final_run.get("status") != "completed":
                fail("private Ollama run did not complete", final_run)
            try:
                tool_evidence = extract_private_ollama_evidence(session)
            except ValueError as error:
                fail("private Ollama typed tool evidence incomplete", str(error), final_run)
            print(json.dumps({
                "api_harness": "pass", "scenario": args.scenario,
                "broker_sha256": broker_sha256,
                "daemon_sha256": hashlib.sha256(pathlib.Path(args.daemon).read_bytes()).hexdigest(),
                "run": final_run,
                **tool_evidence,
                "workspace_id": runtime["workspace_id"],
            }, sort_keys=True))
            return 0
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
        if fixture_remote_root:
            try:
                docker_exec(args.docker, args.container, f"rm -rf {fixture_remote_root}")
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
