"""Read and validate an adapter's ``adapter.json``.

``adapter.json`` sits next to the adapter's ``server.py`` and declares the whole
service surface: the targets, their credentials and routes, the server
instructions and, for a sunset or tombstone release, the notice. Unknown keys are
refused, so a misspelt key (``credentail``) fails the build instead of quietly
dropping a credential binding or a host rule.
"""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from typing import Any, Mapping, Optional, Union

from .safe_http import (
    DEFAULT_MAX_RESPONSE_BYTES,
    DEFAULT_TIMEOUT_S,
    Credential,
    SpecError,
    Target,
    WriteRoute,
)

SPEC_VERSION = 1
KINDS = ("adapter", "sunset", "tombstone")
NOTICE_STATUSES = ("superseded_by", "retired")

_ID = re.compile(r"[a-z][a-z0-9]*(?:-[a-z0-9]+)*")
_SERVICE = re.compile(r"[a-z][a-z0-9_]{1,30}")
_ENV = re.compile(r"[A-Z][A-Z0-9_]{0,127}")

_TOP_KEYS = {
    "kit_spec",
    "id",
    "display_name",
    "service",
    "service_title",
    "kind",
    "instructions",
    "notice",
    "targets",
    "log_level_env",
    "extra_secrets",
}
_TARGET_KEYS = {
    "name",
    "title",
    "base_url",
    "base_url_env",
    "host_pattern",
    "credential",
    "get_routes",
    "read_post_routes",
    "write_routes",
    "headers",
    "env_query",
    "max_response_bytes",
    "timeout_s",
    "documentation",
    "examples",
}
_CREDENTIAL_KEYS = {"env", "scheme", "header_name", "value_prefix", "query_param", "username_env", "required"}
_WRITE_ROUTE_KEYS = {"method", "path"}
_ENV_QUERY_KEYS = {"param", "env"}
_NOTICE_KEYS = {"status", "replacement", "replacement_id", "reason", "use_instead", "date"}
_EXTRA_SECRET_KEYS = {"env", "used_by"}


@dataclass(frozen=True)
class Notice:
    """Why a sunset or tombstone release exists and what the person should use instead."""

    status: str
    reason: str
    use_instead: str
    replacement: str = ""
    replacement_id: str = ""
    date: str = ""

    def sentence(self, display_name: str) -> str:
        if self.status == "superseded_by":
            lead = f"{display_name} is now part of {self.replacement or self.replacement_id}"
            if self.replacement_id:
                lead += f" (BAAM id {self.replacement_id})"
            lead += ". This extension gets no further updates."
        else:
            when = f" on {self.date}" if self.date else ""
            lead = f"{display_name} was retired{when} and no longer calls any service."
        return f"{lead} Why: {self.reason} Use instead: {self.use_instead}"


@dataclass(frozen=True)
class AdapterSpec:
    id: str
    display_name: str
    service: str
    service_title: str
    kind: str
    instructions: str
    targets: tuple[Target, ...]
    notice: Optional[Notice]
    log_level_env: Optional[str]
    extra_secrets: tuple[tuple[str, str], ...] = ()

    @property
    def package(self) -> str:
        return self.id.replace("-", "_")

    @property
    def has_writes(self) -> bool:
        return any(t.has_writes for t in self.targets)

    def tool_names(self) -> list[str]:
        if self.kind == "tombstone":
            return [f"{self.service}_status"]
        names = [f"{self.service}_status", f"{self.service}_get"]
        if self.has_writes:
            names.append(f"{self.service}_write")
        return names

    def environment_names(self) -> list[str]:
        names: list[str] = []
        for target in self.targets:
            names.extend(target.environment_names())
        return list(dict.fromkeys(names))

    def credential_envs(self) -> list[str]:
        return list(dict.fromkeys(t.credential.env for t in self.targets if t.credential))

    def secret_envs(self) -> list[str]:
        """Target credentials plus the declared extra secrets of typed tools."""

        return list(dict.fromkeys([*self.credential_envs(), *(env for env, _ in self.extra_secrets)]))


def _reject_unknown(kind: str, data: Mapping[str, Any], allowed: set[str]) -> None:
    unknown = sorted(set(data) - allowed)
    if unknown:
        raise SpecError(f"{kind} has unknown keys: {', '.join(unknown)}.")


def _text(data: Mapping[str, Any], key: str, *, required: bool = False, default: str = "") -> str:
    value = data.get(key, default)
    if value is None:
        value = default
    if not isinstance(value, str):
        raise SpecError(f"{key} must be text.")
    if required and not value.strip():
        raise SpecError(f"{key} is required.")
    return value


def _strings(data: Mapping[str, Any], key: str, default: tuple[str, ...] = ()) -> tuple[str, ...]:
    value = data.get(key)
    if value is None:
        return default
    if not isinstance(value, list) or not all(isinstance(v, str) for v in value):
        raise SpecError(f"{key} must be a list of text.")
    return tuple(value)


def _credential(data: Any, target: str) -> Optional[Credential]:
    if data is None:
        return None
    if not isinstance(data, Mapping):
        raise SpecError(f"{target}: credential must be an object.")
    _reject_unknown(f"{target}.credential", data, _CREDENTIAL_KEYS)
    required = data.get("required", True)
    if not isinstance(required, bool):
        raise SpecError(f"{target}: credential.required must be true or false.")
    return Credential(
        env=_text(data, "env", required=True),
        scheme=_text(data, "scheme", required=True),
        header_name=data.get("header_name"),
        value_prefix=_text(data, "value_prefix"),
        query_param=data.get("query_param"),
        username_env=data.get("username_env"),
        required=required,
    )


def _target(data: Any) -> Target:
    if not isinstance(data, Mapping):
        raise SpecError("Each target must be an object.")
    name = _text(data, "name", required=True)
    _reject_unknown(f"target {name}", data, _TARGET_KEYS)
    write_routes = []
    for route in data.get("write_routes") or []:
        if not isinstance(route, Mapping):
            raise SpecError(f"{name}: each write route is an object with method and path.")
        _reject_unknown(f"{name}.write_routes", route, _WRITE_ROUTE_KEYS)
        write_routes.append(WriteRoute(method=_text(route, "method", required=True).upper(), pattern=_text(route, "path", required=True)))
    headers = data.get("headers") or {}
    if not isinstance(headers, Mapping) or not all(isinstance(k, str) and isinstance(v, str) for k, v in headers.items()):
        raise SpecError(f"{name}: headers must map names to text.")
    env_query = []
    for item in data.get("env_query") or []:
        if not isinstance(item, Mapping):
            raise SpecError(f"{name}: each env_query entry is an object with param and env.")
        _reject_unknown(f"{name}.env_query", item, _ENV_QUERY_KEYS)
        env_query.append((_text(item, "param", required=True), _text(item, "env", required=True)))
    max_bytes = data.get("max_response_bytes", DEFAULT_MAX_RESPONSE_BYTES)
    timeout = data.get("timeout_s", DEFAULT_TIMEOUT_S)
    if isinstance(max_bytes, bool) or not isinstance(max_bytes, int):
        raise SpecError(f"{name}: max_response_bytes must be a whole number.")
    if isinstance(timeout, bool) or not isinstance(timeout, (int, float)):
        raise SpecError(f"{name}: timeout_s must be a number.")
    get_routes = _strings(data, "get_routes", ("/**",))
    return Target(
        name=name,
        title=_text(data, "title"),
        base_url=_text(data, "base_url"),
        base_url_env=data.get("base_url_env"),
        host_pattern=data.get("host_pattern"),
        credential=_credential(data.get("credential"), name),
        get_routes=get_routes,
        read_post_routes=_strings(data, "read_post_routes"),
        write_routes=tuple(write_routes),
        headers=tuple(headers.items()),
        env_query=tuple(env_query),
        max_response_bytes=max_bytes,
        timeout_s=float(timeout),
        documentation=_text(data, "documentation"),
        examples=_strings(data, "examples"),
    )


def _notice(data: Any) -> Optional[Notice]:
    if data is None:
        return None
    if not isinstance(data, Mapping):
        raise SpecError("notice must be an object.")
    _reject_unknown("notice", data, _NOTICE_KEYS)
    status = _text(data, "status", required=True)
    if status not in NOTICE_STATUSES:
        raise SpecError(f"notice.status must be one of {', '.join(NOTICE_STATUSES)}.")
    notice = Notice(
        status=status,
        reason=_text(data, "reason", required=True),
        use_instead=_text(data, "use_instead", required=True),
        replacement=_text(data, "replacement"),
        replacement_id=_text(data, "replacement_id"),
        date=_text(data, "date"),
    )
    if status == "superseded_by" and not (notice.replacement or notice.replacement_id):
        raise SpecError("A superseded_by notice names its replacement.")
    return notice


def load_adapter_spec(source: Union[str, bytes, Mapping[str, Any]]) -> AdapterSpec:
    """Parse ``adapter.json`` text (or an already parsed object) into a validated spec."""

    data: Any = json.loads(source) if isinstance(source, (str, bytes)) else source
    if not isinstance(data, Mapping):
        raise SpecError("adapter.json must hold one object.")
    _reject_unknown("adapter.json", data, _TOP_KEYS)
    if data.get("kit_spec") != SPEC_VERSION:
        raise SpecError(f"adapter.json must declare kit_spec {SPEC_VERSION}.")
    adapter_id = _text(data, "id", required=True)
    if not _ID.fullmatch(adapter_id):
        raise SpecError("id must be lower case letters and digits, optionally joined by single hyphens.")
    service = _text(data, "service", required=True)
    if not _SERVICE.fullmatch(service):
        raise SpecError("service must be 2 to 31 lower case letters, digits or underscores, starting with a letter.")
    kind = _text(data, "kind", required=True)
    if kind not in KINDS:
        raise SpecError(f"kind must be one of {', '.join(KINDS)}.")
    log_level_env = data.get("log_level_env")
    if log_level_env is not None and (not isinstance(log_level_env, str) or not _ENV.fullmatch(log_level_env)):
        raise SpecError("log_level_env must be an upper case environment name.")
    raw_targets = data.get("targets") or []
    if not isinstance(raw_targets, list):
        raise SpecError("targets must be a list.")
    targets = tuple(_target(t) for t in raw_targets)
    names = [t.name for t in targets]
    if len(set(names)) != len(names):
        raise SpecError("Target names must be unique.")
    notice = _notice(data.get("notice"))
    extra_secrets = []
    for item in data.get("extra_secrets") or []:
        if not isinstance(item, Mapping):
            raise SpecError("Each extra_secrets entry is an object with env and used_by.")
        _reject_unknown("extra_secrets", item, _EXTRA_SECRET_KEYS)
        env = _text(item, "env", required=True)
        if not _ENV.fullmatch(env):
            raise SpecError(f"extra_secrets env {env!r} must be an upper case environment name.")
        extra_secrets.append((env, _text(item, "used_by", required=True)))
    if kind == "adapter":
        if notice is not None:
            raise SpecError("An adapter has no notice. Use kind sunset or tombstone.")
        if not targets:
            raise SpecError("An adapter needs at least one target.")
    elif kind == "sunset":
        if notice is None or not targets:
            raise SpecError("A sunset release keeps its targets and carries a notice.")
    else:
        if notice is None:
            raise SpecError("A tombstone carries a notice.")
        if targets or extra_secrets:
            raise SpecError("A tombstone has no targets and holds no credentials.")
    return AdapterSpec(
        id=adapter_id,
        display_name=_text(data, "display_name", required=True),
        service=service,
        service_title=_text(data, "service_title", required=True),
        kind=kind,
        instructions=_text(data, "instructions"),
        targets=targets,
        notice=notice,
        log_level_env=log_level_env,
        extra_secrets=tuple(extra_secrets),
    )


__all__ = ["AdapterSpec", "KINDS", "Notice", "SPEC_VERSION", "load_adapter_spec"]
