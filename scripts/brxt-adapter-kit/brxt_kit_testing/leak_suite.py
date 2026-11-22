"""The leak suite: proves where every credential of an adapter goes.

It runs one battery of checks against an adapter's tools while every target is
served by its own loopback HTTPS echo server (``echo_server.py``), and every
other host the model might name is an "outsider" echo server that must never
see a request. The battery is the same whether the adapter runs in process or
as a real stdio subprocess; only the session differs.

Checks, for every target T of the adapter:

* a read reaches T only, carries T's credential in T's scheme and no other
  target's credential, and uses GET (or POST on a declared read route);
* the tool result never shows a credential, although the echo server repeats
  every header and query value back;
* a full URL naming another target, an outsider (by IP and by name), plain http,
  a ``//host`` path, a user name trick or the "custom" target is refused, and
  no server sees any request;
* a full URL on T's own origin is accepted and reduced to a path;
* a redirect from T to an outsider or to another target, with each of 301, 302,
  303, 307 and 308, and a redirect from T to plain http on T's own host, is
  reported and not followed;
* a redirect within T's origin is followed and keeps the credential on T;
* more than three redirects stop with an error;
* a read never sends a write method, and a JSON body on a GET route is refused;
* a read of a write route's path is refused, and nothing is sent, unless a read
  route without ``**`` names that path (a REST service's ``GET /entries`` beside
  ``POST /entries``);
* a query credential's parameter cannot be set by the model;
* writes go only to declared routes with the declared method, and the write
  tool is absent when no route is declared;
* the status tool reports which settings are present without any value.

``rewrite_spec_for_echo`` points a real adapter's targets at echo servers (in
process only, the adapter on disk is not changed), so the same battery runs
against every adapter built on the kit.
"""

from __future__ import annotations

import asyncio
import base64
import json
import logging
import secrets as _secrets
import tempfile
import urllib.parse
from contextlib import asynccontextmanager
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, AsyncIterator, Callable, Mapping, Optional, Sequence

from .echo_server import EchoFleet, RecordedRequest

REDIRECT_CODES = (301, 302, 303, 307, 308)
OUTSIDER = "outsider"
OUTSIDER_BY_NAME = "outsider-by-name"


@dataclass
class Finding:
    check: str
    target: str
    ok: bool
    detail: str = ""


@dataclass
class LeakReport:
    findings: list[Finding] = field(default_factory=list)

    def add(self, check: str, target: str, ok: bool, detail: str = "") -> None:
        self.findings.append(Finding(check, target, ok, detail))

    def failures(self) -> list[Finding]:
        return [f for f in self.findings if not f.ok]

    def checks_run(self) -> int:
        return len(self.findings)

    def summary(self) -> str:
        lines = [f"{len(self.findings)} checks, {len(self.failures())} failed"]
        for finding in self.failures():
            lines.append(f"FAIL {finding.check} [{finding.target}]: {finding.detail}")
        return "\n".join(lines)

    def assert_clean(self) -> None:
        if not self.findings:
            raise AssertionError("The leak suite ran no checks.")
        if self.failures():
            raise AssertionError(self.summary())


@dataclass
class ToolCall:
    is_error: bool
    text: str


class Session:
    """What the battery needs from an MCP connection."""

    async def list_tools(self) -> dict[str, dict[str, Any]]:
        raise NotImplementedError

    async def call(self, name: str, arguments: dict[str, Any]) -> ToolCall:
        raise NotImplementedError

    @property
    def instructions(self) -> str:
        return ""


class FastMCPSession(Session):
    def __init__(self, client: Any) -> None:
        self._client = client

    async def list_tools(self) -> dict[str, dict[str, Any]]:
        tools = await self._client.list_tools()
        out = {}
        for tool in tools:
            annotations = tool.annotations.model_dump(by_alias=True, exclude_none=True) if tool.annotations else {}
            out[tool.name] = {"annotations": annotations, "description": tool.description or ""}
        return out

    async def call(self, name: str, arguments: dict[str, Any]) -> ToolCall:
        try:
            result = await self._client.call_tool(name, arguments, raise_on_error=False)
        except Exception as exc:  # a protocol level refusal (for example schema validation)
            return ToolCall(True, f"{type(exc).__name__}: {exc}")
        text = "\n".join(getattr(block, "text", "") for block in result.content)
        return ToolCall(bool(result.is_error), text)

    @property
    def instructions(self) -> str:
        return self._client.instructions or ""

    @property
    def server_info(self) -> Any:
        return self._client.server_info


@asynccontextmanager
async def in_process_session(server: Any) -> AsyncIterator[FastMCPSession]:
    from fastmcp import Client

    # "legacy" is the initialize handshake Biorouter's own MCP client performs.
    async with Client(server, mode="legacy") as client:
        yield FastMCPSession(client)


@asynccontextmanager
async def stdio_session(
    command: str,
    args: Sequence[str],
    *,
    env: Mapping[str, str],
    cwd: Optional[str] = None,
    log_file: Optional[Path] = None,
) -> AsyncIterator[FastMCPSession]:
    from fastmcp import Client
    from fastmcp.client.transports import StdioTransport

    transport = StdioTransport(
        command=command,
        args=list(args),
        env=dict(env),
        cwd=cwd,
        keep_alive=False,
        log_file=log_file,
    )
    async with Client(transport, mode="legacy") as client:
        yield FastMCPSession(client)


# Spec rewriting -------------------------------------------------------------


def echo_server_names(spec_data: Mapping[str, Any]) -> list[str]:
    return [f"t-{t['name']}" for t in spec_data.get("targets") or []] + [OUTSIDER, OUTSIDER_BY_NAME]


def echo_hosts(spec_data: Mapping[str, Any]) -> dict[str, str]:
    """Alternate 127.0.0.1 and localhost so targets differ by host name as well as by port."""

    hosts = {OUTSIDER: "127.0.0.1", OUTSIDER_BY_NAME: "localhost"}
    for index, target in enumerate(spec_data.get("targets") or []):
        hosts[f"t-{target['name']}"] = "localhost" if index % 2 else "127.0.0.1"
    return hosts


def _base_path(target: Mapping[str, Any]) -> str:
    template = target.get("base_url") or "https://placeholder.invalid/"
    path = urllib.parse.urlsplit(template.replace("{", "").replace("}", "")).path.rstrip("/")
    return path


def rewrite_spec_for_echo(spec_data: Mapping[str, Any], fleet: EchoFleet) -> dict[str, Any]:
    """Point each target at its own echo server, keeping its base path, routes and credential."""

    data = json.loads(json.dumps(spec_data))
    for target in data.get("targets") or []:
        server = fleet[f"t-{target['name']}"]
        target["base_url"] = server.origin + _base_path(target)
        target.pop("base_url_env", None)
        target.pop("host_pattern", None)
    return data


def make_secrets(spec: Any) -> dict[str, str]:
    """One distinct, recognisable value for every environment name the adapter reads."""

    values: dict[str, str] = {}
    for target in spec.targets:
        credential = target.credential
        if credential is not None:
            values.setdefault(credential.env, f"leak-{credential.env.lower()}-{_secrets.token_hex(12)}")
            if credential.username_env:
                values.setdefault(credential.username_env, f"user-{target.name}")
        for _, env in target.env_query:
            values.setdefault(env, "leak-contact@example.org")
    return values


# Battery ----------------------------------------------------------------------


def _sample_from_pattern(pattern: str) -> str:
    parts = [p for p in pattern.strip("/").split("/") if p]
    out = []
    for part in parts:
        if part == "**":
            out.append("probe")
        else:
            out.append(part.replace("*", "probe").replace("?", "p").replace("[", "").replace("]", ""))
    return "/" + "/".join(out)


def _read_sample(target: Any, from_kit: Callable[[str, str], bool]) -> tuple[str, str]:
    """A path T accepts for a read, and the method the kit will use for it."""

    for example in target.examples:
        path = example.split("?", 1)[0]
        if "://" in example or not path.startswith("/"):
            continue
        if any(from_kit(p, path) for p in target.read_post_routes):
            return path, "POST"
        if any(from_kit(p, path) for p in target.get_routes):
            return path, "GET"
    if target.get_routes:
        return _sample_from_pattern(target.get_routes[0]), "GET"
    if target.read_post_routes:
        return _sample_from_pattern(target.read_post_routes[0]), "POST"
    return "", ""


def secret_forms(value: str, scheme: str = "", username: Optional[str] = None) -> set[str]:
    forms = {
        value,
        urllib.parse.quote(value, safe=""),
        urllib.parse.quote_plus(value),
        base64.b64encode(value.encode()).decode(),
    }
    if scheme == "basic":
        pair = f"{username}:{value}" if username else f"{value}:"
        encoded = base64.b64encode(pair.encode()).decode()
        forms |= {encoded, encoded.rstrip("=")}
    return {f for f in forms if len(f) >= 6}


def _expected_credential_ok(request: RecordedRequest, credential: Any, values: Mapping[str, str]) -> tuple[bool, str]:
    value = values[credential.env]
    if credential.scheme == "bearer":
        got = request.header("authorization")
        return got == f"Bearer {value}", f"Authorization was {_mask(got, value)}"
    if credential.scheme == "basic":
        user = values.get(credential.username_env) if credential.username_env else None
        pair = f"{user}:{value}" if credential.username_env else f"{value}:"
        expected = "Basic " + base64.b64encode(pair.encode()).decode()
        got = request.header("authorization")
        return got == expected, f"Authorization was {_mask(got, value)}"
    if credential.scheme == "header":
        got = request.header(credential.header_name)
        return got == f"{credential.value_prefix}{value}", f"{credential.header_name} was {_mask(got, value)}"
    got_values = [v for k, v in request.query if k == credential.query_param]
    return got_values == [value], f"{credential.query_param} values were {[_mask(v, value) for v in got_values]}"


def _mask(text: Optional[str], value: str) -> str:
    if text is None:
        return "absent"
    return text.replace(value, "<own secret>")


async def run_battery(
    session: Session,
    spec: Any,
    fleet: EchoFleet,
    values: Mapping[str, str],
    *,
    route_matches: Callable[[str, str], bool],
    report: Optional[LeakReport] = None,
) -> LeakReport:
    """Run every check. ``spec`` is the rewritten spec the session's server was built from."""

    report = report or LeakReport()
    svc = spec.service
    tools = await session.list_tools()
    log = fleet.log

    all_secret_forms: dict[str, set[str]] = {}
    for target in spec.targets:
        if target.credential:
            username = values.get(target.credential.username_env) if target.credential.username_env else None
            all_secret_forms.setdefault(target.credential.env, set()).update(
                secret_forms(values[target.credential.env], target.credential.scheme, username)
            )
    every_form = set().union(*all_secret_forms.values()) if all_secret_forms else set()

    def leaked_in(text: str, forms: set[str]) -> list[str]:
        return sorted(f for f in forms if f in text)

    def server_of(target: Any) -> Any:
        return fleet[f"t-{target.name}"]

    async def refused(check: str, target_name: str, arguments: dict[str, Any], tool: str = f"{svc}_get") -> None:
        log.clear()
        call = await session.call(tool, arguments)
        sent = log.snapshot()
        report.add(check, target_name, call.is_error, f"expected a refusal, got: {call.text[:200]}")
        report.add(check + ":nothing_sent", target_name, not sent, f"requests reached {[(r.server, r.method, r.path) for r in sent]}")
        report.add(check + ":no_secret_in_result", target_name, not leaked_in(call.text, every_form), "a credential appeared in the result")

    # Tool shapes --------------------------------------------------------------
    get_tool = tools.get(f"{svc}_get", {})
    status_tool = tools.get(f"{svc}_status", {})
    report.add("tool_shapes:get_is_read_only", "*", get_tool.get("annotations", {}).get("readOnlyHint") is True, str(get_tool))
    report.add("tool_shapes:get_not_destructive", "*", get_tool.get("annotations", {}).get("destructiveHint") is False, str(get_tool))
    report.add("tool_shapes:status_is_read_only", "*", status_tool.get("annotations", {}).get("readOnlyHint") is True, str(status_tool))
    has_writes = any(t.write_routes for t in spec.targets)
    write_tool = tools.get(f"{svc}_write")
    if has_writes:
        report.add(
            "tool_shapes:write_is_destructive",
            "*",
            bool(write_tool) and write_tool["annotations"].get("destructiveHint") is True and write_tool["annotations"].get("readOnlyHint") is False,
            str(write_tool),
        )
    else:
        report.add("tool_shapes:no_write_tool_without_write_routes", "*", write_tool is None, str(sorted(tools)))
    no_runner = not any(("command" in name) or name.startswith(("plan_", "run_")) for name in tools)
    report.add("tool_shapes:no_command_runner", "*", no_runner, str(sorted(tools)))
    report.add("tool_shapes:no_mutation_flag", "*", "allow_mutation" not in json.dumps(tools), "allow_mutation is still a parameter")

    # Status ---------------------------------------------------------------------
    log.clear()
    status = await session.call(f"{svc}_status", {})
    report.add("status:no_network", "*", not log.snapshot(), "the status tool made a request")
    report.add("status:no_values", "*", not leaked_in(status.text, every_form), "the status report shows a credential")
    for env in all_secret_forms:
        report.add("status:names_setting", env, env in status.text, "the status report does not name the setting")

    outsider = fleet[OUTSIDER]
    outsider_by_name = fleet[OUTSIDER_BY_NAME]

    for target in spec.targets:
        name = target.name
        server = server_of(target)
        base_path = _base_path_of(target)
        sample, method = _read_sample(target, route_matches)
        if not sample:
            report.add("setup:read_sample", name, False, "no read route to probe")
            continue
        own_forms = all_secret_forms.get(target.credential.env, set()) if target.credential else set()
        foreign_forms = every_form - own_forms

        def check_request_set(check: str, sent: list[RecordedRequest], expected_count: Optional[int] = 1) -> None:
            elsewhere = [r for r in sent if r.server != server.name]
            report.add(check + ":only_own_host", name, not elsewhere, f"requests reached {[(r.server, r.path) for r in elsewhere]}")
            if expected_count is not None:
                report.add(check + ":count", name, len(sent) == expected_count, f"{len(sent)} requests, expected {expected_count}")
            for request in sent:
                leaked = leaked_in(request.raw_text(), foreign_forms)
                report.add(check + ":no_foreign_credential", name, not leaked, f"another target's credential reached {request.server}")
                if target.credential is not None and request.server == server.name:
                    ok, detail = _expected_credential_ok(request, target.credential, values)
                    report.add(check + ":own_credential_attached", name, ok, detail)
                if target.credential is None:
                    report.add(check + ":no_credential_on_open_target", name, not leaked_in(request.raw_text(), every_form), "a credential reached a target that has none")
                read_ok = request.method == "GET" or (
                    request.method == "POST" and any(route_matches(p, _rel(request.path, base_path)) for p in target.read_post_routes)
                )
                report.add(check + ":read_method", name, read_ok, f"a read sent {request.method} {request.path}")

        # 1. A plain read.
        log.clear()
        call = await session.call(f"{svc}_get", {"target": name, "path": sample})
        report.add("read:succeeds", name, not call.is_error, call.text[:300])
        check_request_set("read", log.snapshot())
        report.add("read:no_secret_in_result", name, not leaked_in(call.text, every_form), "a credential appeared in the result")
        if not call.is_error:
            try:
                body = json.loads(call.text)
                report.add("read:method", name, body.get("method") == method, f"sent {body.get('method')}, expected {method}")
            except ValueError:
                report.add("read:result_is_json", name, False, call.text[:200])

        # 2. Full URL on the own origin.
        log.clear()
        call = await session.call(f"{svc}_get", {"target": name, "path": server.origin + base_path + sample})
        report.add("full_url:own_origin_accepted", name, not call.is_error, call.text[:300])
        check_request_set("full_url_own", log.snapshot())

        # 3. Full URLs off the target.
        port_of = {s: fleet[s].port for s in fleet.servers}
        hostile = {
            "full_url:outsider_by_ip": f"{outsider.origin}/steal",
            "full_url:outsider_by_name": f"{outsider_by_name.origin}/steal",
            "full_url:own_host_other_port": f"https://{server.host}:{outsider.port}{base_path}{sample}",
            "full_url:plain_http_same_host": f"http://{server.host}:{server.port}{base_path}{sample}",
            "full_url:scheme_relative": f"//127.0.0.1:{port_of[OUTSIDER]}/steal",
            "full_url:userinfo_trick": f"https://{server.host}:{server.port}@127.0.0.1:{port_of[OUTSIDER]}/steal",
            "full_url:backslash_trick": f"https://127.0.0.1:{port_of[OUTSIDER]}\\@{server.host}:{server.port}{base_path}{sample}",
            "full_url:fragment_trick": f"https://127.0.0.1:{port_of[OUTSIDER]}/steal#@{server.host}",
            "path:dot_segments": f"{sample}/../../../steal",
            "path:encoded_dot_segments": f"{sample}/%2e%2e/%2E%2E/steal",
            "path:double_encoded_dot_segments": f"{sample}/%252e%252e/steal",
            "path:crlf": f"{sample}%0d%0aX-Injected:%201",
        }
        for other in spec.targets:
            if other.name != name:
                other_server = server_of(other)
                hostile[f"full_url:other_target_{other.name}"] = other_server.origin + _base_path_of(other) + _read_sample(other, route_matches)[0]
        for check, path in hostile.items():
            await refused(check, name, {"target": name, "path": path})

        # 4. The custom target and unknown names.
        await refused("custom_target", name, {"target": "custom", "path": f"{outsider.origin}/steal"})
        await refused("unknown_target", name, {"target": "not_a_target", "path": sample})

        # 5. Redirects off the origin are not followed.
        redirect_targets = {"outsider": outsider.origin + "/stolen", "outsider_by_name": outsider_by_name.origin + "/stolen",
                            "plain_http_same_host": f"http://{server.host}:{server.port}{base_path}{sample}"}
        for other in spec.targets:
            if other.name != name:
                redirect_targets[f"target_{other.name}"] = server_of(other).origin + _base_path_of(other) + "/stolen"
        for label, location in redirect_targets.items():
            for code in REDIRECT_CODES:
                log.clear()
                call = await session.call(
                    f"{svc}_get",
                    {"target": name, "path": sample, "query": {"__redirect_to": location, "__redirect_code": str(code)}},
                )
                sent = log.snapshot()
                check = f"redirect:{label}:{code}"
                report.add(check + ":reported", name, (not call.is_error) and "redirect_refused" in call.text, call.text[:300])
                check_request_set(check, sent)
                report.add(check + ":no_secret_in_result", name, not leaked_in(call.text, every_form), "a credential appeared in the result")

        # 6. A redirect within the origin is followed and keeps the credential there.
        for code in REDIRECT_CODES:
            log.clear()
            location = server.origin + base_path + sample
            call = await session.call(
                f"{svc}_get",
                {"target": name, "path": sample, "query": {"__redirect_to": location, "__redirect_code": str(code)}},
            )
            followed = False
            if not call.is_error:
                try:
                    followed = len(json.loads(call.text).get("redirects", [])) == 1 and "redirect_refused" not in call.text
                except ValueError:
                    followed = False
            report.add(f"redirect:same_origin:{code}:followed", name, followed, call.text[:300])
            check_request_set(f"redirect:same_origin:{code}", log.snapshot(), expected_count=2)

        # 7. More than three redirects stop.
        location = server.origin + base_path + sample
        for _ in range(5):
            location = server.origin + base_path + sample + "?" + urllib.parse.urlencode({"__redirect_to": location, "__redirect_code": "302"})
        log.clear()
        call = await session.call(f"{svc}_get", {"target": name, "path": location})
        sent = log.snapshot()
        report.add("redirect:limit:stops", name, call.is_error and "redirect" in call.text.lower(), call.text[:300])
        check_request_set("redirect:limit", sent, expected_count=4)

        # 8. A read never sends a body to a GET route.
        if method == "GET":
            await refused("read:json_body_on_get_route", name, {"target": name, "path": sample, "json_body": {"x": 1}})

        # 9. The query credential's parameter is the kit's alone.
        if target.credential is not None and target.credential.scheme == "query":
            await refused("query_credential:reserved", name, {"target": name, "path": sample, "query": {target.credential.query_param: "attacker"}})
            await refused("query_credential:reserved_in_path", name, {"target": name, "path": f"{sample}?{target.credential.query_param}=attacker"})

        # 10. A read does not reach the path of a write route that no read route names.
        read_patterns = (*target.get_routes, *target.read_post_routes)
        for route in target.write_routes:
            if "**" in route.pattern:
                continue
            write_path = _sample_from_pattern(route.pattern)
            if any(route_matches(p, write_path) and "**" not in p for p in read_patterns):
                continue
            await refused(f"read:write_path_refused:{route.method} {route.pattern}", name, {"target": name, "path": write_path})

        # 11. Writes.
        if target.write_routes and write_tool:
            route = target.write_routes[0]
            write_sample = _sample_from_pattern(route.pattern)
            log.clear()
            call = await session.call(f"{svc}_write", {"target": name, "method": route.method, "path": write_sample, "json_body": {"probe": True}})
            sent = log.snapshot()
            report.add("write:declared_route_sent", name, not call.is_error, call.text[:300])
            elsewhere = [r for r in sent if r.server != server.name]
            report.add("write:only_own_host", name, not elsewhere and len(sent) == 1, f"{[(r.server, r.method) for r in sent]}")
            if sent:
                report.add("write:declared_method", name, sent[0].method == route.method, f"sent {sent[0].method}")
                if target.credential is not None:
                    ok, detail = _expected_credential_ok(sent[0], target.credential, values)
                    report.add("write:own_credential_attached", name, ok, detail)
                report.add("write:no_foreign_credential", name, not leaked_in(sent[0].raw_text(), foreign_forms), "foreign credential")
            await refused("write:undeclared_route", name, {"target": name, "method": route.method, "path": "/__undeclared__/x"}, tool=f"{svc}_write")
            await refused("write:full_url_outsider", name, {"target": name, "method": route.method, "path": outsider.origin + "/steal"}, tool=f"{svc}_write")
            undeclared_methods = [m for m in ("DELETE", "PUT", "PATCH", "POST") if not any(r.method == m and route_matches(r.pattern, write_sample) for r in target.write_routes)]
            if undeclared_methods:
                await refused("write:undeclared_method", name, {"target": name, "method": undeclared_methods[0], "path": write_sample}, tool=f"{svc}_write")
        elif write_tool:
            await refused("write:read_only_target", name, {"target": name, "method": "POST", "path": sample}, tool=f"{svc}_write")

    report.add("battery:covered_targets", "*", bool(spec.targets), "no targets")
    return report


def _base_path_of(target: Any) -> str:
    return urllib.parse.urlsplit(target.base_url).path.rstrip("/")


def _rel(path: str, base_path: str) -> str:
    if base_path and path.startswith(base_path):
        return path[len(base_path):] or "/"
    return path


# Drivers ---------------------------------------------------------------------


def run_in_process(
    spec_data: Mapping[str, Any],
    *,
    kit_package: str,
    version: str = "0.0.0",
    workdir: Optional[Path] = None,
) -> LeakReport:
    """Run the battery against ``spec_data`` built with the kit at ``kit_package``.

    ``kit_package`` is ``brxt_kit`` inside the kit itself and ``<package>._kit``
    inside an adapter, so the battery tests the adapter's own vendored copy.
    """

    import importlib

    safe_http = importlib.import_module(f"{kit_package}.safe_http")
    spec_mod = importlib.import_module(f"{kit_package}.spec")
    tools_mod = importlib.import_module(f"{kit_package}.tools")
    directory = workdir or Path(tempfile.mkdtemp(prefix="brxt-leak-"))
    with EchoFleet(directory, echo_server_names(spec_data), hosts=echo_hosts(spec_data)) as fleet:
        rewritten = rewrite_spec_for_echo(spec_data, fleet)
        spec = spec_mod.load_adapter_spec(rewritten)
        values = make_secrets(spec)
        if spec.kind == "tombstone":
            raise ValueError("A tombstone has no targets; use check_tombstone instead.")
        client = safe_http.SafeHttpClient(
            spec.targets,
            user_agent="brxt-leak-suite",
            environ=values,
            verify=fleet.ca.client_context(),
            trust_env=False,
        )
        server = tools_mod.build_server(spec, version=version, client=client)

        async def go() -> LeakReport:
            async with in_process_session(server) as session:
                return await run_battery(session, spec, fleet, values, route_matches=safe_http.route_matches)

        # FastMCP logs every refused call; the battery makes thousands on purpose.
        fastmcp_logger = logging.getLogger("fastmcp")
        previous_level = fastmcp_logger.level
        fastmcp_logger.setLevel(logging.CRITICAL)
        try:
            return asyncio.run(go())
        finally:
            fastmcp_logger.setLevel(previous_level)


def run_over_stdio(
    spec_data: Mapping[str, Any],
    fleet: EchoFleet,
    command: str,
    args: Sequence[str],
    *,
    kit_package: str,
    env: Mapping[str, str],
    values: Mapping[str, str],
    cwd: Optional[str] = None,
    log_file: Optional[Path] = None,
) -> LeakReport:
    """Run the battery against a real adapter subprocess whose spec already points at ``fleet``."""

    import importlib

    safe_http = importlib.import_module(f"{kit_package}.safe_http")
    spec_mod = importlib.import_module(f"{kit_package}.spec")
    spec = spec_mod.load_adapter_spec(spec_data)

    async def go() -> LeakReport:
        async with stdio_session(command, args, env=env, cwd=cwd, log_file=log_file) as session:
            return await run_battery(session, spec, fleet, values, route_matches=safe_http.route_matches)

    return asyncio.run(go())


__all__ = [
    "Finding",
    "LeakReport",
    "echo_hosts",
    "echo_server_names",
    "in_process_session",
    "make_secrets",
    "rewrite_spec_for_echo",
    "run_battery",
    "run_in_process",
    "run_over_stdio",
    "secret_forms",
    "stdio_session",
]
