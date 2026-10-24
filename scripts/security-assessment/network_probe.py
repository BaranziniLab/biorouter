#!/usr/bin/env python3
"""Bounded protocol probes for an isolated BioRouter assessment target.

Use --manifest-only before measured execution. This harness does not start the
daemon, a browser, or a model. All credentials supplied here must be synthetic.
"""

import argparse
import base64
import csv
import hashlib
import http.client
import json
import os
from pathlib import Path
import socket
import time
import urllib.parse


SOURCE_COMMIT = "f73ebf74c578717a7af64b221e57358a63dda8e3"
RELEASE_COMMIT = "b824e2c3e802e243804a150aa7c349fa6ce6a8b6"
PACKAGE_SHA256 = "8b689a2006f1b99f44dec9a48a03b8bb67759dd7f11cdc288042a8a985abff65"
CANARY_KEY = "BRSEC_SYNTHETIC_CONFIG_CANARY"
CANARY_VALUE = "BRSEC_BASELINE_ONLY_SYNTHETIC"
ATTACK_VALUE = "BRSEC_UNAUTHORIZED_CHANGE_SYNTHETIC"
MAX_RESPONSE = 4 * 1024 * 1024


def digest(value):
    if not isinstance(value, bytes):
        value = json.dumps(value, sort_keys=True, separators=(",", ":")).encode()
    return hashlib.sha256(value).hexdigest()


def manifest():
    cases = []

    def add(cid, family, kind, operation, expected, **params):
        cases.append({"case_id": cid, "family": family, "case_type": kind,
                      "operation": operation, "expected_protected_effect": expected,
                      "parameters": params})

    for cid, path in [("A-C01", "/status"), ("A-C02", "/config"),
                      ("A-C05", "/sessions"), ("A-C06", "/system_info"),
                      ("A-C07", "/apps")]:
        add(cid, "A", "control", "http_control", "Expected read is available",
            path=path, credential="none" if path == "/status" else "correct")
    add("A-C03", "A", "control", "config_seed", "Harmless synthetic configuration write persists")
    add("A-C04", "A", "control", "config_verify", "Authenticated reader observes synthetic baseline")
    for cid, path, credential in [
        ("A-01", "/config", "none"), ("A-02", "/config", "wrong"),
        ("A-03", "/config", "empty"), ("A-04", "/sessions", "none"),
        ("A-05", "/system_info", "wrong"), ("A-06", "/apps", "none"),
        ("A-07", "/config", "bearer"), ("A-08", "/config?secret={secret}", "none")]:
        add(cid, "A", "attack", "http_denial", "No unauthorized protected response",
            path=path, credential=credential)
    for cid, credential in [("A-09", "none"), ("A-10", "wrong"), ("A-11", "empty")]:
        add(cid, "A", "attack", "config_attack", "Synthetic baseline remains unchanged",
            credential=credential)
    add("A-12", "A", "attack", "http_denial", "Unauthenticated tool-call route is rejected",
        path="/agent/call_tool", credential="none", method="POST", body={})
    for i, path in enumerate(["/config/", "//config", "/%63onfig", "/ui/workspaceX",
                              "/status/config", "/tool_bridge/0123456789abcdef0123456789abcdef/extra"], 13):
        add(f"A-{i:02}", "A", "attack", "route_shape", "No protected contents via alternate route shape",
            path=path)
    add("A-19", "A", "attack", "oversized", "Unauthenticated oversized configuration POST changes no state",
        payload_bytes=2 * 1024 * 1024 + 1024)
    add("A-C08", "A", "control", "post_oversized", "Health and authenticated configuration reads remain available")
    # A fresh loopback source address makes the failed-auth counter independent
    # of every earlier case. No alias configuration or elevated capability is used.
    add("A-20", "A", "attack", "failed_auth_window", "Forwarding-header spoofing does not reset failure accounting",
        source_address="127.0.0.77", requests=25)
    add("A-C09", "A", "control", "after_failed_auth", "Correct credentials remain usable from the same peer",
        source_address="127.0.0.77")

    for cid, token, origin in [
        ("B-01", "none", "none"), ("B-02", "wrong", "none"),
        ("B-03", "empty", "none"), ("B-04", "correct", "http://evil.test"),
        ("B-05", "correct", "null"), ("B-06", "correct", "file://"),
        ("B-07", "correct", "other_port"),
        ("B-08", "correct", "http://127.0.0.1.evil.test"),
        ("B-09", "correct", "other_host")]:
        add(cid, "B", "attack", "ws", "Workspace socket is not upgraded",
            credential=token, origin=origin)
    add("B-10", "B", "attack", "ws", "Forwarded-proto spoofing cannot compensate for a hostile origin",
        credential="correct", origin="http://evil.test", forwarded_proto="https")
    add("B-C01", "B", "control", "ws", "Same-origin authenticated socket upgrades",
        credential="correct", origin="same")
    add("B-C02", "B", "control", "ws", "Authenticated non-browser socket upgrades",
        credential="correct", origin="none")
    for cid, nonce in [("B-11", "0123456789abcdef0123456789abcdef"), ("B-12", "not-a-nonce")]:
        add(cid, "B", "attack", "unknown_bridge", "Unknown grant exposes no tools and invokes no tool", nonce=nonce)
    add("B-U01", "B", "control", "unavailable_bridge", "Known single-turn grant works and later expires")
    for cid, token in [("B-13", "none"), ("B-14", "wrong")]:
        add(cid, "B", "attack", "shell_denial", "No shell/API secret without a valid browser credential",
            token=token)
    add("B-C03", "B", "control", "browser_exchange", "Valid navigation exchanges token for protected cookie")
    add("B-C04", "B", "control", "browser_shell", "Cookie navigation receives usable protected shell")
    add("B-15", "B", "attack", "shell_fetch", "Cookie-bearing script fetch cannot read shell/API secret")
    add("B-16", "B", "attack", "shell_frame", "Cookie-bearing iframe request cannot read shell/API secret")
    add("B-17", "B", "attack", "shell_insecure", "Plain-HTTP nonloopback address does not receive shell",
        host="192.0.2.123:3000")
    add("B-18", "B", "attack", "shell_api_cookie", "Browser cookie alone does not authorize REST API")
    add("B-19", "B", "attack", "app_guess", "Unknown app IDs disclose no browser content or app existence")
    add("B-20", "B", "attack", "cors", "Hostile-origin preflight receives no CORS permission")
    add("B-C05", "B", "control", "cors_control", "Supported local-development origin receives CORS permission")
    return {"schema_version": 1, "prespecified": True, "version": "1.93.4",
            "source_commit": SOURCE_COMMIT, "release_commit": RELEASE_COMMIT,
            "limitation": "Protocol-level probes; no browser engine or model. An unknown app is not an existing-app enumeration pair.",
            "cases": cases}


class Probe:
    def __init__(self, args):
        self.args = args
        self.base = urllib.parse.urlsplit(args.base_url)
        if self.base.scheme != "http" or self.base.hostname not in ("127.0.0.1", "localhost", "::1"):
            raise ValueError("Only an isolated HTTP loopback target is supported")
        if self.base.path not in ("", "/") or self.base.query:
            raise ValueError("base URL must name the target origin")
        self.port = self.base.port or 80
        self.origin = f"http://{self.base.netloc}"
        self.cookie = None
        self.seed_ok = False
        self.out = Path(args.output)
        self.out.mkdir(parents=True, exist_ok=True)
        (self.out / "evidence").mkdir(exist_ok=True)
        self.calls = []

    def clean(self, value):
        if isinstance(value, str):
            return value.replace(self.args.secret, "<SYNTHETIC_API_SECRET>").replace(
                self.args.browser_token, "<SYNTHETIC_BROWSER_TOKEN>")
        if isinstance(value, dict):
            return {key: self.clean(val) for key, val in value.items()}
        if isinstance(value, (list, tuple)):
            return [self.clean(val) for val in value]
        return value

    def headers(self, credential="none"):
        if credential == "correct":
            return {"X-Secret-Key": self.args.secret}
        if credential == "wrong":
            return {"X-Secret-Key": "0" * 64}
        if credential == "empty":
            return {"X-Secret-Key": ""}
        if credential == "bearer":
            return {"Authorization": "Bearer " + self.args.secret}
        return {}

    def request(self, path, method="GET", headers=None, body=None, source=None):
        path = path.replace("{secret}", urllib.parse.quote(self.args.secret))
        headers = dict(headers or {})
        if isinstance(body, (dict, list)):
            body = json.dumps(body).encode()
            headers.setdefault("Content-Type", "application/json")
        elif isinstance(body, str):
            body = body.encode()
        started = time.monotonic()
        conn = http.client.HTTPConnection(self.base.hostname, self.port, timeout=self.args.timeout,
                                          source_address=(source, 0) if source else None)
        record = {"request": {"method": method, "path": path, "headers": headers,
                               "body_bytes": len(body or b""), "body_sha256": digest(body or b"")}}
        try:
            conn.request(method, path, body=body, headers=headers)
            response = conn.getresponse()
            data = response.read(MAX_RESPONSE + 1)
            record.update(status=response.status, headers=list(response.getheaders()),
                          body=data[:MAX_RESPONSE].decode("utf-8", errors="replace"),
                          response_truncated=len(data) > MAX_RESPONSE)
            return record
        except Exception as error:
            record["error"] = f"{type(error).__name__}: {error}"
            raise
        finally:
            record["duration_ms"] = round((time.monotonic() - started) * 1000, 3)
            self.calls.append(record)
            conn.close()

    def ws(self, params):
        credential = params["credential"]
        query = {"window_id": f"brsec-r{self.args.repetition}"}
        if credential != "none":
            query["secret"] = self.args.secret if credential == "correct" else ("" if credential == "empty" else "0" * 64)
        origin = params["origin"]
        if origin == "same":
            origin = self.origin
        elif origin == "other_port":
            origin = f"http://127.0.0.1:{self.port + 1}"
        elif origin == "other_host":
            origin = f"http://localhost:{self.port}"
        headers = {"Host": self.base.netloc, "Upgrade": "websocket", "Connection": "Upgrade",
                   "Sec-WebSocket-Version": "13", "Sec-WebSocket-Key": base64.b64encode(b"brsec-fixture-12").decode()}
        if origin != "none":
            headers["Origin"] = origin
        if params.get("forwarded_proto"):
            headers["X-Forwarded-Proto"] = params["forwarded_proto"]
        path = "/ui/workspace?" + urllib.parse.urlencode(query)
        request = "GET " + path + " HTTP/1.1\r\n" + "\r\n".join(f"{k}: {v}" for k, v in headers.items()) + "\r\n\r\n"
        record = {"request": {"method": "GET", "path": path, "headers": headers}}
        started = time.monotonic()
        try:
            with socket.create_connection((self.base.hostname, self.port), timeout=self.args.timeout) as sock:
                sock.sendall(request.encode())
                received = b""
                while b"\r\n\r\n" not in received and len(received) < 65536:
                    chunk = sock.recv(4096)
                    if not chunk:
                        break
                    received += chunk
                head, _, tail = received.partition(b"\r\n\r\n")
                lines = head.decode("iso-8859-1").split("\r\n")
                record.update(status=int(lines[0].split(" ")[1]), raw_response_headers=lines,
                              body=tail.decode("utf-8", errors="replace"))
                if record["status"] == 101:
                    # Client frames are masked. A masked empty close shuts down
                    # this test-owned socket without leaving a long-lived client.
                    sock.sendall(b"\x88\x80\x00\x00\x00\x00")
            return record
        except Exception as error:
            record["error"] = f"{type(error).__name__}: {error}"
            raise
        finally:
            record["duration_ms"] = round((time.monotonic() - started) * 1000, 3)
            self.calls.append(record)

    def baseline(self):
        result = self.request("/config", headers=self.headers("correct"))
        value = json.loads(result["body"]).get("config", {}).get(CANARY_KEY)
        return result["status"] == 200 and value == CANARY_VALUE, value

    def denied(self, result, allowed=(401, 429)):
        if self.args.secret in result.get("body", "") or CANARY_VALUE in result.get("body", ""):
            return "attack_succeeded", "Protected synthetic marker disclosed"
        if result["status"] in allowed:
            return "blocked", "Expected denial and no protected synthetic marker"
        return "inconclusive", "Unexpected response without confirmed protected effect"

    def run_case(self, case):
        op, p = case["operation"], case["parameters"]
        if op == "http_control":
            r = self.request(p["path"], headers=self.headers(p["credential"]))
            return ("allowed_control" if r["status"] == 200 else "blocked_control"), "Read response status"
        if op == "config_seed":
            r = self.request("/config/upsert", "POST", self.headers("correct"),
                             {"key": CANARY_KEY, "value": CANARY_VALUE, "is_secret": False})
            self.seed_ok, value = self.baseline()
            return ("allowed_control" if r["status"] == 200 and self.seed_ok else "blocked_control"), {"stored_value": value}
        if op == "config_verify":
            ok, value = self.baseline()
            return ("allowed_control" if ok else "blocked_control"), {"stored_value": value}
        if op == "http_denial":
            r = self.request(p["path"], p.get("method", "GET"), self.headers(p["credential"]), p.get("body"))
            return self.denied(r)
        if op == "config_attack":
            if not self.seed_ok:
                return "not_exercised", "Required synthetic baseline control failed"
            r = self.request("/config/upsert", "POST", self.headers(p["credential"]),
                             {"key": CANARY_KEY, "value": ATTACK_VALUE, "is_secret": False})
            ok, value = self.baseline()
            if value == ATTACK_VALUE:
                return "attack_succeeded", {"stored_value": value}
            return ("blocked" if ok and r["status"] in (401, 429) else "inconclusive"), {"stored_value": value}
        if op == "route_shape":
            return self.denied(self.request(p["path"]), (401, 403, 404, 405, 429))
        if op == "oversized":
            r = self.request("/config/upsert", "POST", {"Content-Type": "application/json"}, b"x" * p["payload_bytes"])
            ok, value = self.baseline()
            return ("blocked" if ok and r["status"] in (401, 413, 429) else "inconclusive"), {"stored_value": value}
        if op == "post_oversized":
            r = self.request("/status")
            ok, _ = self.baseline()
            return ("allowed_control" if r["status"] == 200 and ok else "blocked_control"), "Post-request availability"
        if op == "failed_auth_window":
            statuses = []
            for i in range(p["requests"]):
                headers = self.headers("wrong")
                headers["X-Forwarded-For"] = f"192.0.2.{i + 1}"
                statuses.append(self.request("/config", headers=headers, source=p["source_address"])["status"])
            expected = [401] * 20 + [429] * 5
            return ("blocked" if statuses == expected else "inconclusive"), {"statuses": statuses, "expected": expected}
        if op == "after_failed_auth":
            r = self.request("/config", headers=self.headers("correct"), source=p["source_address"])
            return ("allowed_control" if r["status"] == 200 else "blocked_control"), "Correct-key response after failed attempts"
        if op == "ws":
            r = self.ws(p)
            if case["case_type"] == "control":
                return ("allowed_control" if r["status"] == 101 else "blocked_control"), "Socket handshake status"
            return ("blocked" if r["status"] == 403 else ("attack_succeeded" if r["status"] == 101 else "inconclusive")), "Socket handshake status"
        if op == "unknown_bridge":
            r = self.request("/tool_bridge/" + p["nonce"], "POST", body={"jsonrpc": "2.0", "id": 1, "method": "tools/list"})
            response = json.loads(r["body"])
            blocked = response.get("error", {}).get("code") == -32001 and "result" not in response
            return ("blocked" if blocked else ("attack_succeeded" if "tools" in response.get("result", {}) else "inconclusive")), response
        if op == "unavailable_bridge":
            return "not_exercised", "No real coding-agent turn/grant provisioned; unknown-nonce tests cannot establish grant expiry"
        if op == "shell_denial":
            path = "/" if p["token"] == "none" else "/?t=" + "0" * 64
            return self.denied(self.request(path), (401, 403))
        if op == "browser_exchange":
            r = self.request("/?t=" + self.args.browser_token,
                             headers={"Sec-Fetch-Mode": "navigate", "Sec-Fetch-Dest": "document"})
            cookies = [v for k, v in r["headers"] if k.lower() == "set-cookie"]
            usable = [v for v in cookies if v.split(";", 1)[0].endswith("=" + self.args.browser_token)]
            if usable:
                self.cookie = usable[0].split(";", 1)[0]
            secure_flags = bool(usable) and all("httponly" in c.lower() and "samesite=strict" in c.lower() for c in usable)
            ok = r["status"] == 200 and secure_flags and self.args.secret not in r["body"] and self.args.browser_token not in r["body"]
            return ("allowed_control" if ok else "blocked_control"), {"cookie_count": len(cookies), "protected_flags": secure_flags}
        if op in ("browser_shell", "shell_fetch", "shell_frame", "shell_insecure", "shell_api_cookie"):
            if not self.cookie:
                return "not_exercised", "Browser-token exchange control did not supply a cookie"
            headers = {"Cookie": self.cookie, "Sec-Fetch-Mode": "navigate", "Sec-Fetch-Dest": "document"}
            path = "/"
            if op == "shell_fetch":
                headers.update({"Sec-Fetch-Mode": "cors", "Sec-Fetch-Dest": "empty"})
            elif op == "shell_frame":
                headers["Sec-Fetch-Dest"] = "iframe"
            elif op == "shell_insecure":
                headers = {"Cookie": self.cookie, "Host": p["host"]}
            elif op == "shell_api_cookie":
                path = "/config"
            r = self.request(path, headers=headers)
            if op == "browser_shell":
                h = {k.lower(): v for k, v in r["headers"]}
                ok = (r["status"] == 200 and self.args.secret in r["body"] and
                      h.get("cache-control") == "no-store" and h.get("x-frame-options") == "DENY" and
                      h.get("cross-origin-opener-policy") == "same-origin" and "frame-ancestors 'none'" in h.get("content-security-policy", ""))
                return ("allowed_control" if ok else "blocked_control"), "Shell presence and response protections"
            return self.denied(r, (401, 403, 429))
        if op == "app_guess":
            results = [self.request("/apps/" + slug + "/") for slug in ("brsec-does-not-exist-a", "brsec-does-not-exist-b")]
            statuses = [r["status"] for r in results]
            outcomes = [self.denied(r)[0] for r in results]
            return ("blocked" if all(o == "blocked" for o in outcomes) else "inconclusive"), {"statuses": statuses, "limitation": "No existing app fixture; existing-versus-absent enumeration remains unassessed"}
        if op in ("cors", "cors_control"):
            origin = "http://evil.test" if op == "cors" else "http://localhost:5173"
            r = self.request("/config", "OPTIONS", {"Origin": origin, "Access-Control-Request-Method": "GET",
                              "Access-Control-Request-Headers": "x-secret-key"})
            permission = next((v for k, v in r["headers"] if k.lower() == "access-control-allow-origin"), None)
            if op == "cors_control":
                return ("allowed_control" if permission == origin else "blocked_control"), {"allow_origin": permission}
            return ("blocked" if permission is None else "attack_succeeded"), {"allow_origin": permission, "limitation": "Protocol permission check only; no browser engine"}
        raise ValueError("Unknown case operation " + op)

    def execute(self, corpus):
        results = []
        for case in corpus["cases"]:
            self.calls = []
            started = time.monotonic()
            try:
                outcome, observed = self.run_case(case)
            except Exception as error:
                outcome, observed = "error", f"{type(error).__name__}: {error}"
            evidence_name = "evidence/" + case["case_id"] + ".json"
            evidence = {"case": case, "calls": self.calls, "observed": observed}
            (self.out / evidence_name).write_text(json.dumps(self.clean(evidence), indent=2) + "\n")
            record = {"run_id": f"network-r{self.args.repetition}", "case_id": case["case_id"],
                      "family": case["family"], "case_type": case["case_type"],
                      "repetition": self.args.repetition, "version": self.args.version,
                      "source_commit": SOURCE_COMMIT, "release_commit": RELEASE_COMMIT,
                      "package_sha256": PACKAGE_SHA256,
                      "configuration": "nonroot-linux-official-deb-loopback-synthetic-ui-strong-tokens",
                      "actor": {"uid": os.getuid(), "gid": os.getgid()},
                      "input_fixture_sha256": digest(case),
                      "manifest_sha256": digest((json.dumps(corpus, indent=2) + "\n").encode()),
                      "expected_protected_effect": case["expected_protected_effect"],
                      "observed": self.clean(observed), "outcome": outcome,
                      "duration_ms": round((time.monotonic() - started) * 1000, 3),
                      "evidence": evidence_name,
                      "limitation": "Protocol probes; no browser engine, stochastic model, or native platform behavior"}
            results.append(record)
            with (self.out / "results.jsonl").open("a") as stream:
                stream.write(json.dumps(record, sort_keys=True) + "\n")
            print(json.dumps({"case_id": case["case_id"], "outcome": outcome}), flush=True)
        fields = ["run_id", "case_id", "family", "case_type", "repetition", "version", "outcome", "duration_ms", "evidence", "observed"]
        with (self.out / "results.csv").open("w", newline="") as stream:
            writer = csv.DictWriter(stream, fieldnames=fields, extrasaction="ignore")
            writer.writeheader()
            for result in results:
                writer.writerow({**result, "observed": json.dumps(result["observed"], sort_keys=True)})
        counts = {}
        for result in results:
            counts[result["outcome"]] = counts.get(result["outcome"], 0) + 1
        (self.out / "summary.json").write_text(json.dumps({"observations": len(results), "outcomes": counts}, indent=2) + "\n")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", default="http://127.0.0.1:3000")
    parser.add_argument("--secret", default="manifest-only-synthetic-secret")
    parser.add_argument("--browser-token", default="manifest-only-synthetic-browser-token")
    parser.add_argument("--output", required=True)
    parser.add_argument("--repetition", type=int, default=1)
    parser.add_argument("--version", default="1.93.4")
    parser.add_argument("--timeout", type=float, default=5.0)
    parser.add_argument("--manifest-only", action="store_true")
    args = parser.parse_args()
    corpus = manifest()
    out = Path(args.output)
    out.mkdir(parents=True, exist_ok=True)
    manifest_path = out / "case-manifest.json"
    if manifest_path.exists() and json.loads(manifest_path.read_text()) != corpus:
        raise SystemExit("Existing manifest differs; use a new output directory")
    manifest_path.write_text(json.dumps(corpus, indent=2) + "\n")
    manifest_sha256 = digest(manifest_path.read_bytes())
    (out / "case-manifest.sha256").write_text(manifest_sha256 + "  case-manifest.json\n")
    if args.manifest_only:
        print(json.dumps({"cases": len(corpus["cases"]), "manifest_sha256": manifest_sha256}))
        return
    if (out / "results.jsonl").exists():
        raise SystemExit("Results already exist; never overwrite a measured run")
    if len(args.secret) < 32 or len(args.browser_token) < 32 or args.secret == args.browser_token:
        raise SystemExit("Distinct strong synthetic credentials are required")
    Probe(args).execute(corpus)


if __name__ == "__main__":
    main()
