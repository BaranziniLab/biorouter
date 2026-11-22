"""The tool shapes every kit adapter exposes, and the sunset and tombstone servers.

An adapter exposes:

* ``<svc>_status``: configuration and routes, never a value. Annotated read only.
* ``<svc>_get``: a read through ``safe_http``. Annotated read only, so Biorouter's
  Smart approval runs it without asking.
* ``<svc>_write``: a declared write. Annotated destructive, so Biorouter asks the
  person every time. Registered only when some target declares a write route.

There is no ``allow_mutation`` flag and no local command runner: what a tool may
do is decided by which tool it is, and the Developer shell is the one place a
command runs.

A sunset release keeps the three tools and adds its notice to the server
instructions, to every tool description and to the status report. A tombstone
has the status tool alone, reads no environment value and builds no HTTP client.
"""

# No "from __future__ import annotations" here: the tool signatures use Literal
# types built at run time from the spec, and FastMCP reads them as real objects.
import json
import logging
from typing import Any, Callable, Literal, Optional, Union

from fastmcp import FastMCP
from fastmcp.exceptions import ToolError
from fastmcp.tools import ToolResult
from mcp.types import TextContent, ToolAnnotations
from pydantic import Field

from . import KIT_VERSION
from .safe_http import WRITE_METHODS, KitError, RequestRefused, NotConfigured, SafeHttpClient
from .spec import AdapterSpec

JsonBody = Optional[Union[dict[str, Any], list[Any]]]
TIMEOUT_HELP = "Time limit in seconds. The default, and the most, is the target's own limit."


def _result(payload: Any) -> ToolResult:
    text = json.dumps(payload, indent=2, sort_keys=True, ensure_ascii=False, default=str)
    return ToolResult(content=[TextContent(type="text", text=text)])


def _call(http: SafeHttpClient, action: Callable[[], dict[str, Any]]) -> ToolResult:
    """Run one request and turn every failure into a redacted ToolError.

    The ToolError is raised after the except block has finished, so it carries no
    chained exception whose text could hold a URL or a value. An unexpected
    exception is caught too: FastMCP would otherwise log its traceback and repeat
    its text to the model.
    """

    failure: Optional[str] = None
    level = logging.ERROR
    try:
        payload = action()
    except (RequestRefused, NotConfigured) as exc:
        failure, level = http.redact(str(exc)), logging.WARNING
    except KitError as exc:
        failure = http.redact(str(exc))
    except Exception as exc:  # a bug in the kit or the adapter: never let its text through unredacted
        failure = http.redact(f"The extension hit an internal error ({type(exc).__name__}). Nothing more is shown.")
    if failure is not None:
        raise ToolError(failure, log_level=level)
    return _result(payload)


def user_agent(spec: AdapterSpec, version: str) -> str:
    return f"BioRouter-{spec.id}/{version} (brxt-adapter-kit {KIT_VERSION})"


def server_instructions(spec: AdapterSpec) -> str:
    svc = spec.service
    parts: list[str] = []
    if spec.notice is not None:
        parts.append(spec.notice.sentence(spec.display_name))
    if spec.kind == "tombstone":
        parts.append(f"Call {svc}_status to show the person this notice. This extension holds no credentials.")
        return "\n\n".join(parts)
    if spec.instructions.strip():
        parts.append(spec.instructions.strip())
    how = (
        f"Start with {svc}_status to see which {spec.service_title} targets are configured. "
        f"{svc}_get reads from a target; Biorouter runs reads without asking in Smart approval. "
    )
    if spec.has_writes:
        how += f"{svc}_write changes data on the service, and Biorouter asks the person before every write. "
    how += (
        "Pass a path relative to the target's base URL. A full URL is accepted only on the target's own host. "
        "Credentials are attached by the extension, never appear in tool arguments, and are sent only to the "
        "host they belong to."
    )
    parts.append(how)
    return "\n\n".join(parts)


def _description(spec: AdapterSpec, text: str) -> str:
    if spec.kind == "sunset" and spec.notice is not None:
        return f"Sunset notice: {spec.notice.sentence(spec.display_name)}\n\n{text}"
    return text


def _target_lines(spec: AdapterSpec, *, writes: bool) -> str:
    lines = []
    for target in spec.targets:
        if writes:
            if not target.write_routes:
                continue
            routes = "; ".join(f"{r.method} {r.pattern}" for r in target.write_routes)
        else:
            routes = ", ".join(f"GET {p}" for p in target.get_routes)
            if target.read_post_routes:
                routes += "; POST (read) " + ", ".join(target.read_post_routes)
        title = f" ({target.title})" if target.title else ""
        example = f" Example: {target.examples[0]}" if target.examples and not writes else ""
        lines.append(f"- {target.name}{title}: {routes}.{example}")
    return "\n".join(lines)


def _status_payload(spec: AdapterSpec, version: str, client: Optional[SafeHttpClient]) -> dict[str, Any]:
    payload: dict[str, Any] = {
        "extension": spec.id,
        "display_name": spec.display_name,
        "service": spec.service_title,
        "version": version,
        "kit_version": KIT_VERSION,
        "kind": spec.kind,
    }
    if spec.notice is not None:
        payload["notice"] = {
            "status": spec.notice.status,
            "message": spec.notice.sentence(spec.display_name),
            "replacement": spec.notice.replacement,
            "replacement_id": spec.notice.replacement_id,
            "reason": spec.notice.reason,
            "use_instead": spec.notice.use_instead,
        }
    if client is not None:
        payload["targets"] = client.describe()
        payload["tools"] = spec.tool_names()
        if spec.extra_secrets:
            environ = client.env()
            payload["extra_secrets"] = [
                {"env": env, "used_by": used_by, "present": bool((environ.get(env) or "").strip())}
                for env, used_by in spec.extra_secrets
            ]
    return payload


def build_server(
    spec: AdapterSpec,
    *,
    version: str,
    client: Optional[SafeHttpClient] = None,
) -> FastMCP:
    """Build the FastMCP server for an adapter, sunset or tombstone spec.

    ``client`` is for tests (a client that trusts a test certificate); a released
    adapter passes none and the kit builds one from the spec.
    """

    mcp = FastMCP(spec.display_name, instructions=server_instructions(spec), version=version)
    svc = spec.service

    if spec.kind == "tombstone":

        def tombstone_status() -> ToolResult:
            return _result(_status_payload(spec, version, None))

        mcp.tool(
            tombstone_status,
            name=f"{svc}_status",
            description=(
                f"Explain why {spec.display_name} was retired and what to use instead. "
                "Makes no network call and reads no credential."
            ),
            annotations=ToolAnnotations(
                title=f"{spec.display_name} retirement notice",
                readOnlyHint=True,
                destructiveHint=False,
                idempotentHint=True,
                openWorldHint=False,
            ),
        )
        return mcp

    http = client or SafeHttpClient(spec.targets, user_agent=user_agent(spec, version))
    names = tuple(t.name for t in spec.targets)
    TargetName = Literal[names]  # type: ignore[valid-type]
    target_help = "Which declared target to call. There is no custom target."

    def status() -> ToolResult:
        return _call(http, lambda: _status_payload(spec, version, http))

    mcp.tool(
        status,
        name=f"{svc}_status",
        description=_description(
            spec,
            f"Report which {spec.service_title} targets are configured, their routes and which settings are "
            "present. Never shows a credential value and makes no network call.",
        ),
        annotations=ToolAnnotations(
            title=f"{spec.display_name} status",
            readOnlyHint=True,
            destructiveHint=False,
            idempotentHint=True,
            openWorldHint=False,
        ),
    )

    def get(
        target: TargetName = Field(description=target_help),  # type: ignore[valid-type]
        path: str = Field(description="Path relative to the target's base URL, for example /projects. A full URL is accepted only on the target's own host."),
        query: Optional[dict[str, Any]] = Field(default=None, description="Query parameters as names and values."),
        json_body: JsonBody = Field(default=None, description="JSON body, only for read routes that use POST."),
        accept: Optional[str] = Field(default=None, description="Response media type, default application/json."),
        timeout_s: Optional[float] = Field(default=None, ge=1.0, le=120.0, description=TIMEOUT_HELP),
    ) -> ToolResult:
        return _call(
            http,
            lambda: http.read(target, path, query=query, json_body=json_body, accept=accept, timeout_s=timeout_s),
        )

    mcp.tool(
        get,
        name=f"{svc}_get",
        description=_description(
            spec,
            f"Read from {spec.service_title}. Sends GET, or POST on the routes this extension declares as reads; "
            "never a method that changes data. Read routes:\n" + _target_lines(spec, writes=False),
        ),
        annotations=ToolAnnotations(
            title=f"Read from {spec.service_title}",
            readOnlyHint=True,
            destructiveHint=False,
            idempotentHint=True,
            openWorldHint=True,
        ),
    )

    if spec.has_writes:
        methods = tuple(sorted({r.method for t in spec.targets for r in t.write_routes}, key=WRITE_METHODS.index))
        Method = Literal[methods]  # type: ignore[valid-type]
        write_names = tuple(t.name for t in spec.targets if t.write_routes)
        WriteTarget = Literal[write_names]  # type: ignore[valid-type]

        def write(
            target: WriteTarget = Field(description=target_help),  # type: ignore[valid-type]
            method: Method = Field(description="HTTP method of a declared write route."),  # type: ignore[valid-type]
            path: str = Field(description="Path of a declared write route, relative to the target's base URL."),
            query: Optional[dict[str, Any]] = Field(default=None, description="Query parameters as names and values."),
            json_body: JsonBody = Field(default=None, description="JSON body to send."),
            timeout_s: Optional[float] = Field(default=None, ge=1.0, le=120.0, description=TIMEOUT_HELP),
        ) -> ToolResult:
            return _call(
                http,
                lambda: http.write(target, method, path, query=query, json_body=json_body, timeout_s=timeout_s),
            )

        mcp.tool(
            write,
            name=f"{svc}_write",
            description=_description(
                spec,
                f"Change data on {spec.service_title}. Biorouter asks the person before every call. Only these "
                "routes are accepted:\n" + _target_lines(spec, writes=True),
            ),
            annotations=ToolAnnotations(
                title=f"Change data on {spec.service_title}",
                readOnlyHint=False,
                destructiveHint=True,
                idempotentHint=False,
                openWorldHint=True,
            ),
        )

    return mcp


__all__ = ["build_server", "server_instructions", "user_agent"]
