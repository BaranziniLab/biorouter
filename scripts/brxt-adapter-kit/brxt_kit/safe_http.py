"""The only HTTP path a BRXT adapter may use.

This module is security code. A person reviews it before any adapter built on it
is released, and every change to it needs that review again. The contract it
enforces:

1. A target's base URL is https, names one host and an optional port, and has no
   user name, password, query or fragment. A ``{NAME}`` placeholder in it is
   filled from the environment and must be one DNS label, so a configured value
   can never change which host a request reaches. A base read whole from the
   environment (``base_url_env``) is held to the same rules and, when the target
   declares one, to its ``host_pattern``.
2. A request path is relative to the target's base path. A full URL is accepted
   only when its scheme, host and port equal the target's and its path lies under
   the target's base path; it is then reduced to a relative path. A path that
   starts with ``//``, carries a backslash, a control character, a fragment, a
   user name or a dot segment (also when percent encoded) is refused before any
   connection is opened. There is no "custom" target.
3. A credential belongs to one target. It is read from the environment at the
   moment of the request and attached only to a request whose origin (scheme,
   host and port) is that target's origin. Two targets that share an origin may
   not carry different credentials.
4. The HTTP library never follows a redirect. The kit follows at most three, and
   only to the same origin. A redirect to any other origin, including the same
   host over plain http, or one whose Location carries a user name, is reported
   and not followed, so no header or query value ever reaches another host.
5. A read is GET, or POST on a route the target declares as a read route. A write
   uses a method and route the target declares. A read never sends a write
   method, and it does not reach the path of a declared write route (also as a
   server may read the path: percent decoded, a ``;`` parameter cut, a final
   extension dropped, case folded), because some services run an action
   whatever the method. Only a read route that lies within the write route's
   pattern opens such a path to a read, and never with the write's own method;
   a read POST route that could reach a write route otherwise is refused when
   it is declared.
6. Request bodies, response bodies and waits are capped. Compressed responses
   are inflated in bounded steps, the raw bytes read are capped as well, and
   nothing after the end of the compressed data is read or kept (a gzip body
   may hold several members, which are inflated in turn). Every secret value
   the adapter holds, in every encoding the kit sends it in, is removed from
   anything returned to the model, including error messages.
7. The HTTP library's own log lines print full URLs, so a query credential would
   reach the log. They are silenced here, and the kit never logs a header, a
   query string or a body.
"""

from __future__ import annotations

import base64
import fnmatch
import json
import logging
import os
import re
import ssl
import time
import unicodedata
import urllib.parse
import zlib
from dataclasses import dataclass
from typing import Any, Mapping, Optional, Sequence, Union

import httpx

from . import KIT_VERSION

for _logger_name in ("httpx", "httpcore"):
    logging.getLogger(_logger_name).setLevel(logging.WARNING)

WRITE_METHODS: tuple[str, ...] = ("POST", "PUT", "PATCH", "DELETE")
REDIRECT_STATUSES = frozenset({301, 302, 303, 307, 308})
MAX_REDIRECTS = 3
DEFAULT_TIMEOUT_S = 30.0
MAX_TIMEOUT_S = 120.0
DEFAULT_MAX_RESPONSE_BYTES = 2_000_000
MAX_RESPONSE_BYTES = 20_000_000
MAX_REQUEST_BODY_BYTES = 1_000_000
MAX_PATH_LENGTH = 4096
MAX_PATH_SEGMENTS = 256
MAX_QUERY_ITEMS = 200
MAX_QUERY_VALUE_LENGTH = 8192
MAX_ACCEPT_LENGTH = 256
# Raw bytes a reply may carry beyond what its decoded body may hold: headers of
# gzip members, deflate block overhead. Past it the read stops, so a body that
# decodes to little cannot keep the kit reading.
RAW_READ_SLACK_BYTES = 1 << 20
REDACTED = "[REDACTED]"
MIN_REDACTION_LENGTH = 6

CREDENTIAL_SCHEMES: tuple[str, ...] = ("bearer", "basic", "header", "query")

_ENV_NAME = re.compile(r"[A-Z][A-Z0-9_]{0,127}")
_TARGET_NAME = re.compile(r"[a-z][a-z0-9_]{0,39}")
_DNS_LABEL = re.compile(r"[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?")
_HOST = re.compile(r"[a-z0-9](?:[a-z0-9.-]{0,251}[a-z0-9])?")
_HEADER_NAME = re.compile(r"[A-Za-z0-9][A-Za-z0-9-]{0,63}")
_QUERY_PARAM = re.compile(r"[A-Za-z0-9_.\-\[\]]{1,64}")
# Spaces and tabs only: \s would also admit CR, LF and other line breaks.
_MEDIA_RANGE = re.compile(
    r"[A-Za-z0-9.+*-]+/[A-Za-z0-9.+*-]+(?:[ \t]*;[ \t]*[A-Za-z0-9.+-]+=[A-Za-z0-9.+-]+)*"
)
_ACCEPT = re.compile(
    rf"{_MEDIA_RANGE.pattern}(?:[ \t]*,[ \t]*{_MEDIA_RANGE.pattern}){{0,7}}"
)
_FULL_URL = re.compile(r"[A-Za-z][A-Za-z0-9+.\-]*://")
_PLACEHOLDER = re.compile(r"\{([A-Z][A-Z0-9_]*)\}")
_CONTROL_OR_SPACE = re.compile(r"[\x00-\x20\x7f]")
_CONTROL = re.compile(r"[\x00-\x1f\x7f]")
_PERCENT_U = re.compile(r"%u[0-9A-Fa-f]{4}")
_GLOB_CHARS = frozenset("*?[")

# Headers a target may not set statically: the credential, the connection and
# anything that would let a static value stand in for one.
_RESERVED_HEADERS = frozenset(
    {
        "authorization",
        "proxy-authorization",
        "cookie",
        "host",
        "content-length",
        "content-type",
        "transfer-encoding",
        "connection",
        "accept-encoding",
        "user-agent",
    }
)
# Response headers worth showing the model. Everything else (cookies above all)
# stays out of the result.
_RESPONSE_HEADERS_SHOWN = (
    "content-type",
    "content-length",
    "link",
    "location",
    "etag",
    "last-modified",
    "retry-after",
    "x-total-count",
    "x-ratelimit-limit",
    "x-ratelimit-remaining",
    "x-ratelimit-reset",
)
_TEXT_TYPES = ("text/", "json", "xml", "csv", "tab-separated", "javascript", "yaml", "x-www-form-urlencoded")


class KitError(Exception):
    """A failure whose message is safe to show the model."""


class SpecError(KitError):
    """A target declaration breaks a kit rule. This is the adapter author's bug."""


class NotConfigured(KitError):
    """A value the person sets in the extension's settings is missing or invalid."""


class RequestRefused(KitError):
    """The request breaks a rule. Nothing was sent."""


class UpstreamError(KitError):
    """The request was sent and failed (network, time limit, too many redirects)."""


@dataclass(frozen=True)
class Credential:
    """How one secret environment value is attached to its target's requests."""

    env: str
    scheme: str
    header_name: Optional[str] = None
    value_prefix: str = ""
    query_param: Optional[str] = None
    username_env: Optional[str] = None
    required: bool = True

    def __post_init__(self) -> None:
        if not _ENV_NAME.fullmatch(self.env or ""):
            raise SpecError(f"Credential env {self.env!r} must be an upper case environment name.")
        if self.scheme not in CREDENTIAL_SCHEMES:
            raise SpecError(f"Credential scheme {self.scheme!r} must be one of {', '.join(CREDENTIAL_SCHEMES)}.")
        if self.scheme == "header":
            if not self.header_name or not _HEADER_NAME.fullmatch(self.header_name):
                raise SpecError("A header credential needs a valid header_name.")
            if self.header_name.lower() in {"host", "cookie", "content-length", "transfer-encoding", "connection"}:
                raise SpecError(f"{self.header_name} cannot carry a credential.")
        elif self.header_name is not None:
            raise SpecError("header_name belongs to the header scheme only.")
        if self.scheme == "query":
            if not self.query_param or not _QUERY_PARAM.fullmatch(self.query_param):
                raise SpecError("A query credential needs a valid query_param.")
        elif self.query_param is not None:
            raise SpecError("query_param belongs to the query scheme only.")
        if self.value_prefix and self.scheme != "header":
            raise SpecError("value_prefix belongs to the header scheme only.")
        if len(self.value_prefix) > 32 or _CONTROL.search(self.value_prefix):
            raise SpecError("value_prefix must be short printable text.")
        if self.username_env is not None:
            if self.scheme != "basic":
                raise SpecError("username_env belongs to the basic scheme only.")
            if not _ENV_NAME.fullmatch(self.username_env):
                raise SpecError(f"username_env {self.username_env!r} must be an upper case environment name.")


@dataclass(frozen=True)
class WriteRoute:
    method: str
    pattern: str

    def __post_init__(self) -> None:
        if self.method not in WRITE_METHODS:
            raise SpecError(f"Write route method {self.method!r} must be one of {', '.join(WRITE_METHODS)}.")
        _check_pattern(self.pattern)


@dataclass(frozen=True)
class Origin:
    scheme: str
    host: str
    port: int

    def __str__(self) -> str:
        default = 443 if self.scheme == "https" else 80
        suffix = "" if self.port == default else f":{self.port}"
        return f"{self.scheme}://{self.host}{suffix}"


@dataclass(frozen=True)
class ResolvedBase:
    origin: Origin
    path: str

    @property
    def url(self) -> str:
        return f"{self.origin}{self.path}"


@dataclass(frozen=True)
class Target:
    """One service endpoint an adapter may call, and the only one its credential reaches."""

    name: str
    base_url: str
    title: str = ""
    base_url_env: Optional[str] = None
    host_pattern: Optional[str] = None
    credential: Optional[Credential] = None
    get_routes: tuple[str, ...] = ("/**",)
    read_post_routes: tuple[str, ...] = ()
    write_routes: tuple[WriteRoute, ...] = ()
    headers: tuple[tuple[str, str], ...] = ()
    env_query: tuple[tuple[str, str], ...] = ()
    max_response_bytes: int = DEFAULT_MAX_RESPONSE_BYTES
    timeout_s: float = DEFAULT_TIMEOUT_S
    documentation: str = ""
    examples: tuple[str, ...] = ()

    def __post_init__(self) -> None:
        if not _TARGET_NAME.fullmatch(self.name or ""):
            raise SpecError(f"Target name {self.name!r} must be lower case letters, digits and underscores.")
        if self.name == "custom":
            raise SpecError("There is no 'custom' target: every target names its own host.")
        if self.base_url_env is not None and not _ENV_NAME.fullmatch(self.base_url_env):
            raise SpecError(f"{self.name}: base_url_env must be an upper case environment name.")
        if not self.base_url and not self.base_url_env:
            raise SpecError(f"{self.name}: declare base_url, base_url_env or both.")
        if self.host_pattern is not None:
            try:
                re.compile(self.host_pattern)
            except re.error as exc:
                raise SpecError(f"{self.name}: host_pattern does not compile: {exc}") from None
        if self.base_url:
            sample = _PLACEHOLDER.sub("placeholder", self.base_url)
            try:
                base = _parse_base(sample, origin_hint=self.name)
            except NotConfigured as exc:
                raise SpecError(f"{self.name}: base_url {self.base_url!r} is not allowed: {exc}") from None
            if self.host_pattern and not _PLACEHOLDER.search(self.base_url):
                if not re.fullmatch(self.host_pattern, base.origin.host):
                    raise SpecError(f"{self.name}: base_url host does not match its own host_pattern.")
            for placeholder in _PLACEHOLDER.findall(self.base_url):
                if not _ENV_NAME.fullmatch(placeholder):
                    raise SpecError(f"{self.name}: placeholder {placeholder!r} is not an environment name.")
        if not self.get_routes and not self.read_post_routes and not self.write_routes:
            raise SpecError(f"{self.name}: declare at least one route.")
        for pattern in (*self.get_routes, *self.read_post_routes):
            _check_pattern(pattern)
        for route in self.write_routes:
            if not isinstance(route, WriteRoute):
                raise SpecError(f"{self.name}: write_routes holds WriteRoute values.")
        for pattern in self.read_post_routes:
            for route in self.write_routes:
                if _patterns_overlap(pattern, route.pattern) and (
                    route.method == "POST" or not _pattern_within(pattern, route.pattern)
                ):
                    raise SpecError(
                        f"{self.name}: the read route POST {pattern} can reach the write route {route.method} "
                        f"{route.pattern}, so a write could run through the read tool without the person's "
                        "approval. A read POST route may not reach a POST write route at all, and may reach one "
                        "with another method only when it lies within that route. Narrow one of the two."
                    )
        seen_headers = set()
        for name, value in self.headers:
            lowered = name.lower()
            if not _HEADER_NAME.fullmatch(name) or lowered in _RESERVED_HEADERS:
                raise SpecError(f"{self.name}: header {name!r} cannot be set statically.")
            if lowered in seen_headers:
                raise SpecError(f"{self.name}: header {name!r} is declared twice.")
            if self.credential and self.credential.header_name and lowered == self.credential.header_name.lower():
                raise SpecError(f"{self.name}: header {name!r} is the credential header.")
            if not isinstance(value, str) or len(value) > 512 or not value.isascii() or _CONTROL.search(value):
                raise SpecError(f"{self.name}: header {name!r} needs a short printable value.")
            seen_headers.add(lowered)
        for param, env in self.env_query:
            if not _QUERY_PARAM.fullmatch(param) or not _ENV_NAME.fullmatch(env):
                raise SpecError(f"{self.name}: env_query entry {param!r} -> {env!r} is not valid.")
            if self.credential and self.credential.query_param and param.lower() == self.credential.query_param.lower():
                raise SpecError(f"{self.name}: env_query {param!r} is the credential parameter.")
            if self.credential and env in {self.credential.env, self.credential.username_env}:
                raise SpecError(f"{self.name}: env_query may not send the credential value.")
        if not (1_000 <= int(self.max_response_bytes) <= MAX_RESPONSE_BYTES):
            raise SpecError(f"{self.name}: max_response_bytes must be between 1000 and {MAX_RESPONSE_BYTES}.")
        if not (1.0 <= float(self.timeout_s) <= MAX_TIMEOUT_S):
            raise SpecError(f"{self.name}: timeout_s must be between 1 and {MAX_TIMEOUT_S:g}.")

    @property
    def has_writes(self) -> bool:
        return bool(self.write_routes)

    def environment_names(self) -> list[str]:
        """Every environment name this target reads, for status reports and build checks."""
        names: list[str] = []
        if self.base_url_env:
            names.append(self.base_url_env)
        names.extend(_PLACEHOLDER.findall(self.base_url or ""))
        if self.credential:
            names.append(self.credential.env)
            if self.credential.username_env:
                names.append(self.credential.username_env)
        names.extend(env for _, env in self.env_query)
        return list(dict.fromkeys(names))


def _check_pattern(pattern: str) -> None:
    if not isinstance(pattern, str) or not pattern.startswith("/"):
        raise SpecError(f"Route {pattern!r} must start with '/'.")
    if _CONTROL_OR_SPACE.search(pattern) or "\\" in pattern or "?" in pattern or "#" in pattern:
        raise SpecError(f"Route {pattern!r} may hold only path characters.")
    segments = [s for s in pattern.strip("/").split("/")] if pattern.strip("/") else []
    if any(s in {"", ".", ".."} for s in segments):
        raise SpecError(f"Route {pattern!r} has an empty or dot segment.")
    if sum(1 for s in segments if s == "**") > 2:
        raise SpecError(f"Route {pattern!r} may use '**' at most twice.")
    for segment in segments:
        if "**" in segment and segment != "**":
            raise SpecError(f"Route {pattern!r}: '**' must be a whole segment.")


def route_matches(pattern: str, rel_path: str) -> bool:
    """Match a relative path against a route pattern.

    ``*`` matches within one path segment, ``**`` matches any number of whole
    segments (including none). Matching is case sensitive and runs on the path as
    sent, so percent encoding is not undone.
    """

    pattern_parts = pattern.strip("/").split("/") if pattern.strip("/") else []
    path_parts = rel_path.strip("/").split("/") if rel_path.strip("/") else []
    return _match_parts(tuple(pattern_parts), tuple(path_parts))


def _match_parts(pattern: tuple[str, ...], path: tuple[str, ...]) -> bool:
    if not pattern:
        return not path
    head = pattern[0]
    if head == "**":
        return any(_match_parts(pattern[1:], path[i:]) for i in range(len(path) + 1))
    if not path:
        return False
    return fnmatch.fnmatchcase(path[0], head) and _match_parts(pattern[1:], path[1:])


def _pattern_parts(pattern: str) -> tuple[str, ...]:
    return tuple(pattern.strip("/").split("/")) if pattern.strip("/") else ()


def _has_glob(segment: str) -> bool:
    return any(c in _GLOB_CHARS for c in segment)


_STAR, _ONE = "*", "?"


def _segment_tokens(segment: str) -> list[Union[str, tuple[str]]]:
    """A segment pattern as tokens: ``*``, one character of any kind, or a literal character.

    A ``[...]`` class is read as one character of any kind, so the overlap test
    below can only answer yes more often than the exact one would.
    """

    tokens: list[Union[str, tuple[str]]] = []
    i = 0
    while i < len(segment):
        c = segment[i]
        if c == "*":
            if not tokens or tokens[-1] != _STAR:
                tokens.append(_STAR)
        elif c == "?":
            tokens.append(_ONE)
        elif c == "[":
            j = i + 1
            if j < len(segment) and segment[j] == "!":
                j += 1
            if j < len(segment) and segment[j] == "]":
                j += 1
            end = segment.find("]", j)
            if end == -1:
                tokens.append((c,))  # fnmatch reads an unclosed '[' as itself
            else:
                tokens.append(_ONE)
                i = end
        else:
            tokens.append((c,))
        i += 1
    return tokens


def _segments_overlap(a: str, b: str) -> bool:
    """Whether some one segment matches both segment patterns."""

    if not _has_glob(a) and not _has_glob(b):
        return a == b
    if not _has_glob(a):
        return fnmatch.fnmatchcase(a, b)
    if not _has_glob(b):
        return fnmatch.fnmatchcase(b, a)
    ta, tb = _segment_tokens(a), _segment_tokens(b)
    seen: set[tuple[int, int]] = set()
    stack = [(0, 0)]
    while stack:
        i, j = stack.pop()
        if (i, j) in seen:
            continue
        seen.add((i, j))
        if i == len(ta) and j == len(tb):
            return True
        if i < len(ta) and ta[i] == _STAR:
            stack.append((i + 1, j))
        if j < len(tb) and tb[j] == _STAR:
            stack.append((i, j + 1))
        if i < len(ta) and j < len(tb):
            x, y = ta[i], tb[j]
            if x in (_STAR, _ONE) or y in (_STAR, _ONE) or x == y:
                stack.append((i if x == _STAR else i + 1, j if y == _STAR else j + 1))
    return False


def _patterns_overlap(a: str, b: str) -> bool:
    """Whether some path matches both route patterns. It may answer yes for a pair that has none, never no for one that has."""

    pa, pb = _pattern_parts(a), _pattern_parts(b)
    seen: set[tuple[int, int]] = set()
    stack = [(0, 0)]
    while stack:
        i, j = stack.pop()
        if (i, j) in seen:
            continue
        seen.add((i, j))
        if i == len(pa) and j == len(pb):
            return True
        if i < len(pa) and pa[i] == "**":
            stack.append((i + 1, j))
            if j < len(pb):
                stack.append((i, j + 1))
        if j < len(pb) and pb[j] == "**":
            stack.append((i, j + 1))
            if i < len(pa):
                stack.append((i + 1, j))
        if i < len(pa) and j < len(pb) and "**" not in (pa[i], pb[j]) and _segments_overlap(pa[i], pb[j]):
            stack.append((i + 1, j + 1))
    return False


def _pattern_within(inner: str, outer: str) -> bool:
    """Whether every path ``inner`` matches is matched by ``outer`` too. It may answer no for a pattern that is within, never yes for one that is not."""

    inner_parts, outer_parts = _pattern_parts(inner), _pattern_parts(outer)
    if not any(_has_glob(s) for s in inner_parts):
        return route_matches(outer, inner)
    if len(inner_parts) != len(outer_parts):
        return False
    return all(
        i == o or (i != "**" and o == "*") or (not _has_glob(i) and o != "**" and fnmatch.fnmatchcase(i, o))
        for i, o in zip(inner_parts, outer_parts)
    )


def _server_views(rel_path: str) -> set[str]:
    """The paths a server may take ``rel_path`` for.

    Percent decoded once and twice, read under NFKC, with ``\\`` read as ``/`` and
    empty segments merged, with and without each segment's ``;`` parameter (a
    servlet container cuts it), and with and without a final ``.ext`` (suffix
    routing in Spring and Rails).
    """

    once = urllib.parse.unquote(rel_path)
    twice = urllib.parse.unquote(once)
    views = {rel_path}
    for form in (rel_path, once, twice):
        segments = [s for s in re.split(r"[/\\]", unicodedata.normalize("NFKC", form)) if s]
        for cut in (False, True):
            parts = [s.split(";", 1)[0] for s in segments] if cut else list(segments)
            parts = [s for s in parts if s]
            variants = [parts]
            if parts and "." in parts[-1].lstrip("."):
                last = parts[-1]
                variants += [parts[:-1] + [last.rsplit(".", 1)[0]], parts[:-1] + [last.split(".", 1)[0]]]
            views.update("/" + "/".join(v) for v in variants)
    return views


def _reaches(pattern: str, views: set[str]) -> bool:
    folded = pattern.casefold()
    return any(route_matches(pattern, v) or route_matches(folded, v.casefold()) for v in views)


def _unnamed_write_routes(target: "Target", method: str, rel_path: str, read_patterns: Sequence[str]) -> list[WriteRoute]:
    """The write routes a read with ``method`` on ``rel_path`` would reach.

    A write route counts unless a read route that matches the path lies within
    the write route's own pattern (a REST service names ``GET /entries`` beside
    ``POST /entries``), and it always counts when the read uses the write's own
    method. A catch-all such as ``/**`` never names a write route.
    """

    if not target.write_routes:
        return []
    views = _server_views(rel_path)
    reached = []
    for route in target.write_routes:
        if not _reaches(route.pattern, views):
            continue
        named = route.method != method and any(
            route_matches(p, rel_path) and _pattern_within(p, route.pattern) for p in read_patterns
        )
        if not named:
            reached.append(route)
    return reached


def _parse_base(url: str, *, origin_hint: str) -> ResolvedBase:
    if _CONTROL_OR_SPACE.search(url) or "\\" in url:
        raise NotConfigured(f"{origin_hint}: the base URL holds a space, a backslash or a control character.")
    try:
        parts = urllib.parse.urlsplit(url)
    except ValueError:
        raise NotConfigured(f"{origin_hint}: the base URL cannot be read as a URL.") from None
    if parts.scheme.lower() != "https":
        raise NotConfigured(f"{origin_hint}: the base URL must use https.")
    if "@" in parts.netloc:
        raise NotConfigured(f"{origin_hint}: the base URL must not carry a user name or password.")
    if parts.query or parts.fragment or "?" in url or "#" in url:
        raise NotConfigured(f"{origin_hint}: the base URL must not carry a query or a fragment.")
    host = (parts.hostname or "").lower()
    if not _HOST.fullmatch(host):
        raise NotConfigured(f"{origin_hint}: the base URL needs a plain host name.")
    try:
        port = parts.port
    except ValueError:
        raise NotConfigured(f"{origin_hint}: the base URL has an invalid port.") from None
    path = parts.path.rstrip("/")
    _check_rel_path(path or "/", refuse=NotConfigured)
    return ResolvedBase(Origin("https", host, 443 if port is None else port), path)


def _check_rel_path(rel: str, *, refuse: type[KitError] = RequestRefused) -> None:
    segments = rel.split("/")
    if len(segments) > MAX_PATH_SEGMENTS:
        raise refuse("The path has too many segments.")
    once = urllib.parse.unquote(rel)
    twice = urllib.parse.unquote(once)
    for decoded in (rel, once, twice):
        if _CONTROL.search(decoded):
            raise refuse("The path holds a control character, also when decoded.")
        if _PERCENT_U.search(decoded):
            raise refuse("The path holds a %u escape, which some servers decode. Percent encode as %XX.")
        # Servlet containers cut a ';' parameter before they resolve dots, so '..;' acts as '..',
        # and some servers read fullwidth full stops under NFKC.
        for part in re.split(r"[/\\]", unicodedata.normalize("NFKC", decoded)):
            if part.split(";", 1)[0] in {".", ".."}:
                raise refuse("The path holds a '.' or '..' segment, also when decoded. Name the resource directly.")


def _origin_of(url: httpx.URL) -> Optional[Origin]:
    scheme = (url.scheme or "").lower()
    # raw_host is what the connection, SNI and Host header use; url.host decodes punycode.
    host = url.raw_host.decode("ascii", errors="replace").lower()
    if scheme not in {"http", "https"} or not host:
        return None
    port = url.port if url.port is not None else (443 if scheme == "https" else 80)
    return Origin(scheme, host, port)


@dataclass(frozen=True)
class PreparedRequest:
    rel_path: str
    query: tuple[tuple[str, str], ...]


class SafeHttpClient:
    """Sends requests to declared targets only, with each credential bound to its own origin."""

    def __init__(
        self,
        targets: Sequence[Target],
        *,
        user_agent: str,
        environ: Optional[Mapping[str, str]] = None,
        verify: Union[ssl.SSLContext, bool, None] = None,
        trust_env: bool = True,
    ) -> None:
        if not targets:
            raise SpecError("An adapter needs at least one target.")
        names = [t.name for t in targets]
        if len(set(names)) != len(names):
            raise SpecError("Target names must be unique.")
        if not user_agent or len(user_agent) > 200 or _CONTROL.search(user_agent):
            raise SpecError("user_agent must be short printable text.")
        self._targets: dict[str, Target] = {t.name: t for t in targets}
        self._user_agent = user_agent
        self._environ = environ
        self._verify = verify
        self._trust_env = trust_env
        self._check_static_origin_sharing()

    # Introspection -------------------------------------------------------

    @property
    def targets(self) -> tuple[Target, ...]:
        return tuple(self._targets.values())

    @property
    def has_writes(self) -> bool:
        return any(t.has_writes for t in self._targets.values())

    def env(self) -> Mapping[str, str]:
        return os.environ if self._environ is None else self._environ

    def target(self, name: str) -> Target:
        if name not in self._targets:
            known = ", ".join(sorted(self._targets))
            raise RequestRefused(
                f"Unknown target {name!r}. This extension reaches only its declared targets ({known}); "
                "there is no custom target."
            )
        return self._targets[name]

    def describe(self) -> list[dict[str, Any]]:
        """What the status tool reports: configuration, never a value."""

        environ = self.env()
        rows = []
        for target in self._targets.values():
            row: dict[str, Any] = {
                "name": target.name,
                "title": target.title,
                "documentation": target.documentation,
                "examples": list(target.examples),
                "read_routes": {"GET": list(target.get_routes), "POST": list(target.read_post_routes)},
                "write_routes": [{"method": r.method, "path": r.pattern} for r in target.write_routes],
                "settings": {name: bool((environ.get(name) or "").strip()) for name in target.environment_names()},
            }
            try:
                row["base_url"] = self._resolve(target, environ).url
                row["configured"] = True
            except NotConfigured as exc:
                row["base_url"] = None
                row["configured"] = False
                row["problem"] = str(exc)
            if target.credential:
                credential = target.credential
                present = bool((environ.get(credential.env) or "").strip())
                row["credential"] = {
                    "env": credential.env,
                    "scheme": credential.scheme,
                    "required": credential.required,
                    "present": present,
                }
                if credential.required and not present:
                    row["configured"] = False
                    row.setdefault("problem", f"Set {credential.env} in the extension's settings in Biorouter.")
            rows.append(row)
        return rows

    # Requests -------------------------------------------------------------

    def read(
        self,
        target_name: str,
        path: str,
        *,
        query: Optional[Mapping[str, Any]] = None,
        json_body: Any = None,
        accept: Optional[str] = None,
        timeout_s: Optional[float] = None,
    ) -> dict[str, Any]:
        """GET, or POST on a declared read route. Never a write method."""

        target = self.target(target_name)
        environ = self.env()
        base = self._resolve(target, environ)
        prepared = self._prepare(target, base, path, query)
        if any(route_matches(p, prepared.rel_path) for p in target.read_post_routes):
            method = "POST"
            read_patterns = target.read_post_routes
            body = {} if json_body is None else json_body
        else:
            if json_body is not None:
                raise RequestRefused(
                    f"{target.name}: json_body is accepted only on read routes that use POST "
                    f"({', '.join(target.read_post_routes) or 'none declared'})."
                )
            if not any(route_matches(p, prepared.rel_path) for p in target.get_routes):
                raise RequestRefused(
                    f"{target.name}: {prepared.rel_path} is not one of this target's read routes "
                    f"({', '.join(target.get_routes) or 'none'})."
                )
            method = "GET"
            read_patterns = target.get_routes
            body = None
        reached = _unnamed_write_routes(target, method, prepared.rel_path, read_patterns)
        if reached:
            declared = ", ".join(f"{r.method} {r.pattern}" for r in reached)
            raise RequestRefused(
                f"{target.name}: {prepared.rel_path} is the path of a declared write ({declared}), so the read tool "
                f"does not send {method} there: some services run the action whatever the method. If this is a "
                "change, use the write tool, which asks the person first. If it is a read, the extension's author "
                "has to name the path in a read route."
            )
        return self._send(target, base, method, prepared, body, accept, timeout_s, environ)

    def write(
        self,
        target_name: str,
        method: str,
        path: str,
        *,
        query: Optional[Mapping[str, Any]] = None,
        json_body: Any = None,
        accept: Optional[str] = None,
        timeout_s: Optional[float] = None,
    ) -> dict[str, Any]:
        """A declared write: the method and route must both be in the target's write routes."""

        target = self.target(target_name)
        method_upper = (method or "").upper()
        if method_upper not in WRITE_METHODS:
            raise RequestRefused(f"Write method {method!r} must be one of {', '.join(WRITE_METHODS)}.")
        if not target.write_routes:
            raise RequestRefused(f"{target.name} has no write routes. It is read only.")
        environ = self.env()
        base = self._resolve(target, environ)
        prepared = self._prepare(target, base, path, query)
        allowed = [r for r in target.write_routes if r.method == method_upper and route_matches(r.pattern, prepared.rel_path)]
        if not allowed:
            declared = ", ".join(f"{r.method} {r.pattern}" for r in target.write_routes)
            raise RequestRefused(f"{target.name}: {method_upper} {prepared.rel_path} is not a declared write route ({declared}).")
        return self._send(target, base, method_upper, prepared, json_body, accept, timeout_s, environ)

    def redact(self, text: str) -> str:
        return _redact_text(text, self._secret_forms(self.env()))

    # Internals --------------------------------------------------------------

    def _resolve(self, target: Target, environ: Mapping[str, str]) -> ResolvedBase:
        raw = (environ.get(target.base_url_env) or "").strip() if target.base_url_env else ""
        if raw:
            base = _parse_base(raw, origin_hint=f"{target.name} ({target.base_url_env})")
        else:
            if not target.base_url:
                raise NotConfigured(f"{target.name}: set {target.base_url_env} in the extension's settings in Biorouter.")

            def fill(match: re.Match[str]) -> str:
                name = match.group(1)
                value = (environ.get(name) or "").strip()
                if not value:
                    raise NotConfigured(f"{target.name}: set {name} in the extension's settings in Biorouter.")
                if not _DNS_LABEL.fullmatch(value):
                    raise NotConfigured(
                        f"{target.name}: {name} must be one name made of letters, digits and inner hyphens "
                        "(for example mylab), not a URL or a host name."
                    )
                return value.lower()

            base = _parse_base(_PLACEHOLDER.sub(fill, target.base_url), origin_hint=target.name)
        if target.host_pattern and not re.fullmatch(target.host_pattern, base.origin.host):
            raise NotConfigured(f"{target.name}: host {base.origin.host} is not a host this target may use.")
        self._check_origin_conflict(target, base, environ)
        return base

    def _check_static_origin_sharing(self) -> None:
        static: dict[Origin, list[Target]] = {}
        for target in self._targets.values():
            if target.base_url_env or _PLACEHOLDER.search(target.base_url or ""):
                continue
            origin = _parse_base(target.base_url, origin_hint=target.name).origin
            for other in static.get(origin, []):
                if other.credential and target.credential and _credential_env(other) != _credential_env(target):
                    raise SpecError(f"Targets {other.name} and {target.name} share {origin} but carry different credentials.")
            static.setdefault(origin, []).append(target)

    def _check_origin_conflict(self, target: Target, base: ResolvedBase, environ: Mapping[str, str]) -> None:
        if not target.credential:
            return
        for other in self._targets.values():
            if other is target or not other.credential or _credential_env(other) == _credential_env(target):
                continue
            try:
                other_base = self._resolve_quiet(other, environ)
            except NotConfigured:
                continue
            if other_base.origin == base.origin:
                raise NotConfigured(
                    f"{target.name} and {other.name} resolve to the same host {base.origin} but carry different "
                    "credentials. Point them at their own hosts."
                )

    def _resolve_quiet(self, target: Target, environ: Mapping[str, str]) -> ResolvedBase:
        raw = (environ.get(target.base_url_env) or "").strip() if target.base_url_env else ""
        if raw:
            return _parse_base(raw, origin_hint=target.name)
        if not target.base_url:
            raise NotConfigured(target.name)
        filled = _PLACEHOLDER.sub(lambda m: (environ.get(m.group(1)) or "").strip().lower() or "unset", target.base_url)
        return _parse_base(filled, origin_hint=target.name)

    def _prepare(
        self,
        target: Target,
        base: ResolvedBase,
        path: str,
        query: Optional[Mapping[str, Any]],
    ) -> PreparedRequest:
        if not isinstance(path, str):
            raise RequestRefused("path must be text.")
        if len(path) > MAX_PATH_LENGTH:
            raise RequestRefused("path is too long.")
        if path == "":
            path = "/"
        if _CONTROL_OR_SPACE.search(path):
            raise RequestRefused("path holds a space or a control character. Percent encode it, or pass values in query.")
        if "\\" in path:
            raise RequestRefused("path holds a backslash.")
        if path.startswith("//"):
            raise RequestRefused(
                f"path starts with '//', which names another host. Pass a path relative to {base.url}."
            )
        if _FULL_URL.match(path):
            rel_path, path_query = self._reduce_full_url(target, base, path)
        else:
            if "#" in path:
                raise RequestRefused("path holds a fragment ('#').")
            raw_path, _, raw_query = path.partition("?")
            rel_path = raw_path if raw_path.startswith("/") else "/" + raw_path
            _check_rel_path(rel_path)
            path_query = _parse_query_string(raw_query)
        model_query = _normalise_query(query)
        merged = (*path_query, *model_query)
        if len(merged) > MAX_QUERY_ITEMS:
            raise RequestRefused("Too many query parameters.")
        reserved = _reserved_params(target)
        for key, _ in merged:
            if key.lower() in reserved:
                raise RequestRefused(f"Query parameter {key!r} is set by the extension and cannot be passed.")
        return PreparedRequest(rel_path, tuple(merged))

    def _reduce_full_url(self, target: Target, base: ResolvedBase, url: str) -> tuple[str, tuple[tuple[str, str], ...]]:
        own = f"Pass a path relative to {base.url}, or a full URL on {base.origin} only."
        try:
            parts = urllib.parse.urlsplit(url)
        except ValueError:
            raise RequestRefused(f"{target.name}: the URL cannot be read. {own}") from None
        if parts.scheme.lower() != "https":
            raise RequestRefused(f"{target.name}: only https URLs on this target's own host are accepted. {own}")
        if "@" in parts.netloc:
            raise RequestRefused(f"{target.name}: a URL with a user name or password is refused. {own}")
        if parts.fragment or "#" in url:
            raise RequestRefused(f"{target.name}: a URL with a fragment ('#') is refused. {own}")
        host = (parts.hostname or "").lower()
        try:
            port = parts.port
        except ValueError:
            raise RequestRefused(f"{target.name}: the URL has an invalid port. {own}") from None
        if Origin("https", host, 443 if port is None else port) != base.origin:
            raise RequestRefused(
                f"{target.name}: the URL names a host other than {base.origin}. Credentials are never sent to "
                f"another host, and this target reaches only its own. {own}"
            )
        full_path = parts.path or "/"
        if base.path:
            if full_path == base.path:
                rel = "/"
            elif full_path.startswith(base.path + "/"):
                rel = full_path[len(base.path):]
            else:
                raise RequestRefused(f"{target.name}: the URL is outside the base path {base.path}. {own}")
        else:
            rel = full_path if full_path.startswith("/") else "/" + full_path
        _check_rel_path(rel)
        return rel, _parse_query_string(parts.query)

    def _auth(self, target: Target, environ: Mapping[str, str]) -> tuple[dict[str, str], list[tuple[str, str]]]:
        headers: dict[str, str] = {}
        params: list[tuple[str, str]] = []
        credential = target.credential
        if credential is None:
            return headers, params
        value = (environ.get(credential.env) or "").strip()
        if not value:
            if credential.required:
                raise NotConfigured(f"{target.name} needs {credential.env}. Set it in the extension's settings in Biorouter.")
            return headers, params
        if _CONTROL.search(value):
            raise NotConfigured(f"{credential.env} holds a control character.")
        if len(value) < MIN_REDACTION_LENGTH:
            raise NotConfigured(
                f"{credential.env} is shorter than {MIN_REDACTION_LENGTH} characters, too short for the extension "
                "to keep it out of results. Check the value in the extension's settings in Biorouter."
            )
        if credential.scheme in {"bearer", "header"} and not value.isascii():
            raise NotConfigured(f"{credential.env} holds a character outside ASCII, which a header cannot carry.")
        if credential.scheme == "bearer":
            headers["Authorization"] = f"Bearer {value}"
        elif credential.scheme == "basic":
            if credential.username_env:
                user = (environ.get(credential.username_env) or "").strip()
                if not user:
                    raise NotConfigured(f"{target.name} needs {credential.username_env}.")
                pair = f"{user}:{value}"
            else:
                pair = f"{value}:"
            headers["Authorization"] = "Basic " + base64.b64encode(pair.encode()).decode()
        elif credential.scheme == "header":
            headers[credential.header_name or ""] = f"{credential.value_prefix}{value}"
        elif credential.scheme == "query":
            params.append((credential.query_param or "", value))
        return headers, params

    def _env_params(self, target: Target, environ: Mapping[str, str]) -> list[tuple[str, str]]:
        params = []
        for param, env in target.env_query:
            value = (environ.get(env) or "").strip()
            if value and not _CONTROL.search(value):
                params.append((param, value))
        return params

    def _secret_forms(self, environ: Mapping[str, str]) -> list[str]:
        forms: set[str] = set()
        for target in self._targets.values():
            credential = target.credential
            if credential is None:
                continue
            value = (environ.get(credential.env) or "").strip()
            if not value:
                continue
            candidates = {
                value,
                urllib.parse.quote(value, safe=""),
                urllib.parse.quote_plus(value),
                base64.b64encode(value.encode()).decode(),
                json.dumps(value)[1:-1],
            }
            if credential.scheme == "basic":
                user = (environ.get(credential.username_env) or "").strip() if credential.username_env else ""
                pair = f"{user}:{value}" if credential.username_env else f"{value}:"
                encoded = base64.b64encode(pair.encode()).decode()
                candidates.update({encoded, encoded.rstrip("=")})
            forms.update(c for c in candidates if len(c) >= MIN_REDACTION_LENGTH)
        return sorted(forms, key=len, reverse=True)

    def _send(
        self,
        target: Target,
        base: ResolvedBase,
        method: str,
        prepared: PreparedRequest,
        json_body: Any,
        accept: Optional[str],
        timeout_s: Optional[float],
        environ: Mapping[str, str],
    ) -> dict[str, Any]:
        if accept is not None and (
            not isinstance(accept, str)
            or len(accept) > MAX_ACCEPT_LENGTH
            or not accept.isascii()
            or not _ACCEPT.fullmatch(accept.strip())
        ):
            raise RequestRefused("accept must be a media type such as application/json.")
        content: Optional[bytes] = None
        if json_body is not None:
            try:
                content = json.dumps(json_body, ensure_ascii=False, allow_nan=False).encode()
            except (TypeError, ValueError):
                raise RequestRefused("json_body must be plain JSON.") from None
            if len(content) > MAX_REQUEST_BODY_BYTES:
                raise RequestRefused("json_body is too large.")
        # A caller may shorten the target's own time limit, never lengthen it.
        limit = float(target.timeout_s) if timeout_s is None else max(1.0, min(float(timeout_s), float(target.timeout_s)))
        cap = int(target.max_response_bytes)
        secret_forms = self._secret_forms(environ)
        auth_headers, auth_params = self._auth(target, environ)
        env_params = self._env_params(target, environ)
        reserved = _reserved_params(target)

        params: list[tuple[str, str]] = list(prepared.query)
        hops: list[dict[str, Any]] = []
        deadline = time.monotonic() + limit
        error: Optional[str] = None
        result: Optional[dict[str, Any]] = None
        try:
            try:
                url = httpx.URL(base.url + prepared.rel_path)
            except (httpx.InvalidURL, ValueError, TypeError):
                raise RequestRefused(f"{target.name}: the path is not a valid URL path.") from None
            raw_path = url.raw_path.decode("ascii", errors="replace").split("?", 1)[0]
            if _origin_of(url) != base.origin or not (
                not base.path or raw_path == base.path or raw_path.startswith(base.path + "/")
            ):
                raise RequestRefused(f"{target.name}: the request does not resolve under {base.url}.")
            with httpx.Client(
                timeout=httpx.Timeout(limit),
                follow_redirects=False,
                trust_env=self._trust_env,
                verify=True if self._verify is None else self._verify,
            ) as client:
                for hop in range(MAX_REDIRECTS + 1):
                    if _origin_of(url) != base.origin:
                        raise RequestRefused(f"{target.name}: refusing to send to another host.")
                    headers = {
                        "User-Agent": self._user_agent,
                        "Accept": accept.strip() if accept else "application/json",
                        "Accept-Encoding": "gzip, deflate",
                    }
                    for name, value in target.headers:
                        if accept and name.lower() == "accept":
                            continue
                        headers[name] = value
                    headers.update(auth_headers)
                    if content is not None:
                        headers["Content-Type"] = "application/json"
                    send_params = [*params, *env_params, *auth_params]
                    request = client.build_request(
                        method,
                        url.copy_with(query=None),
                        params=send_params or None,
                        headers=headers,
                        content=content,
                    )
                    response = client.send(request, stream=True)
                    try:
                        location = response.headers.get("location")
                        if response.status_code in REDIRECT_STATUSES and location:
                            try:
                                next_url = response.url.join(location)
                                next_origin = _origin_of(next_url)
                            except (httpx.InvalidURL, ValueError, TypeError):
                                next_url, next_origin = None, None
                            # A user name in the Location would make the HTTP library
                            # replace the kit's own Authorization header, so it is not
                            # followed either.
                            if next_url is None or next_origin != base.origin or next_url.userinfo:
                                result = self._result(
                                    target,
                                    method,
                                    request.url,
                                    response,
                                    b"",
                                    False,
                                    hops,
                                    redirect_refused={
                                        "status_code": response.status_code,
                                        "to_origin": str(next_origin) if next_origin else "unreadable",
                                        "reason": (
                                            "The service redirected to another host. The extension does not follow "
                                            "redirects off the target's own host, so no credential left it."
                                        ),
                                    },
                                )
                                break
                            if hop == MAX_REDIRECTS:
                                raise UpstreamError(f"{target.name}: stopped after {MAX_REDIRECTS} redirects.")
                            hops.append({"status_code": response.status_code, "to": next_url.path})
                            if response.status_code == 303 or (response.status_code in (301, 302) and method == "POST"):
                                method = "GET"
                                content = None
                            url = next_url
                            params = [
                                (k, v)
                                for k, v in next_url.params.multi_items()
                                if k.lower() not in reserved
                            ]
                            continue
                        body, truncated = _read_capped(response, cap, deadline)
                        result = self._result(target, method, request.url, response, body, truncated, hops)
                        break
                    finally:
                        response.close()
        except KitError as exc:
            error_type: type[KitError] = type(exc)
            error = str(exc)
        except httpx.InvalidURL:
            error_type = RequestRefused
            error = f"{target.name}: the service or the request named an invalid URL."
        except httpx.TimeoutException:
            error_type = UpstreamError
            error = f"{target.name}: no answer within {limit:g} seconds."
        except httpx.HTTPError as exc:
            error_type = UpstreamError
            error = f"{target.name}: the request failed ({type(exc).__name__}: {exc})."
        if error is not None:
            raise error_type(_redact_text(_strip_query_strings(error), secret_forms))
        assert result is not None
        return _redact_obj(result, secret_forms)

    def _display_url(self, target: Target, url: httpx.URL) -> str:
        hidden = set()
        if target.credential and target.credential.query_param:
            hidden.add(target.credential.query_param.lower())
        pairs = [(k, REDACTED if k.lower() in hidden else v) for k, v in url.params.multi_items()]
        shown = url.copy_with(query=None)
        if pairs:
            return f"{shown}?{urllib.parse.urlencode(pairs, safe='[]')}"
        return str(shown)

    def _result(
        self,
        target: Target,
        method: str,
        url: httpx.URL,
        response: httpx.Response,
        body: bytes,
        truncated: bool,
        hops: list[dict[str, Any]],
        redirect_refused: Optional[dict[str, Any]] = None,
    ) -> dict[str, Any]:
        content_type = response.headers.get("content-type", "")
        out: dict[str, Any] = {
            "target": target.name,
            "method": method,
            "url": self._display_url(target, url),
            "status_code": response.status_code,
            "ok": 200 <= response.status_code < 300,
            "content_type": content_type,
            "headers": {k: response.headers[k] for k in _RESPONSE_HEADERS_SHOWN if k in response.headers},
            "bytes_read": len(body),
            "truncated": truncated,
            "redirects": hops,
            "kit_version": KIT_VERSION,
        }
        if redirect_refused is not None:
            out["redirect_refused"] = redirect_refused
            out["body"] = None
            return out
        lowered = content_type.lower()
        if not body:
            out["body"] = None
        elif any(t in lowered for t in _TEXT_TYPES) or not lowered:
            charset = response.charset_encoding or "utf-8"
            try:
                text = body.decode(charset, errors="replace")
            except LookupError:
                text = body.decode("utf-8", errors="replace")
            out["body"] = text
            if "json" in lowered and not truncated:
                try:
                    out["body"] = json.loads(text)
                except ValueError:
                    pass
        else:
            out["body"] = None
            out["note"] = f"Binary content ({content_type}) of {len(body)} bytes is not shown."
        return out


def _credential_env(target: Target) -> Optional[str]:
    return target.credential.env if target.credential else None


def _reserved_params(target: Target) -> set[str]:
    reserved = {param.lower() for param, _ in target.env_query}
    if target.credential and target.credential.query_param:
        reserved.add(target.credential.query_param.lower())
    return reserved


def _parse_query_string(raw: str) -> tuple[tuple[str, str], ...]:
    if not raw:
        return ()
    pairs = urllib.parse.parse_qsl(raw, keep_blank_values=True)
    for key, value in pairs:
        if _CONTROL.search(key) or _CONTROL.search(value):
            raise RequestRefused("The query holds a control character.")
    return tuple(pairs)


def _normalise_query(query: Optional[Mapping[str, Any]]) -> list[tuple[str, str]]:
    if query is None:
        return []
    if not isinstance(query, Mapping):
        raise RequestRefused("query must be an object of names and values.")
    out: list[tuple[str, str]] = []
    for key, value in query.items():
        if not isinstance(key, str) or not key or len(key) > 200 or _CONTROL.search(key):
            raise RequestRefused("Query names must be short printable text.")
        values = value if isinstance(value, (list, tuple)) else [value]
        if len(values) > 100:
            raise RequestRefused(f"Query parameter {key!r} has too many values.")
        for item in values:
            if item is None:
                continue
            if isinstance(item, bool):
                text = "true" if item else "false"
            elif isinstance(item, (int, float)):
                text = str(item)
            elif isinstance(item, str):
                text = item
            else:
                raise RequestRefused("Query values must be text, numbers, booleans or lists of them.")
            if len(text) > MAX_QUERY_VALUE_LENGTH or _CONTROL.search(text):
                raise RequestRefused(f"Query parameter {key!r} has a value that is too long or holds a control character.")
            out.append((key, text))
    return out


class _Inflater:
    """Undo gzip or deflate in bounded steps, so a small reply cannot grow without limit.

    Each call inflates at most ``budget + 1`` bytes. A gzip body may be a series
    of members (RFC 1952; bgzip writes one, and so does ``cat a.gz b.gz``), and
    each member is inflated in turn within the same budget. Once the compressed
    data ends nothing more is inflated or kept: bytes after it that do not start
    another gzip member set ``trailing``, and the caller stops reading. A body that
    cannot be inflated is an ``UpstreamError``.
    """

    def __init__(self, encoding: str) -> None:
        encoding = encoding.strip().lower()
        self.trailing = False
        self._held = False
        self._ended = False
        self._pending = b""
        self._z: Any = None
        if encoding in {"", "identity"}:
            self._mode = "identity"
        elif encoding in {"gzip", "x-gzip"}:
            self._mode = "gzip"
        elif encoding == "deflate":
            self._mode = "deflate"
        else:
            raise UpstreamError(f"The service answered with an encoding the extension did not ask for ({encoding}).")

    @property
    def compressed(self) -> bool:
        return self._mode != "identity"

    def feed(self, data: bytes, budget: int) -> bytes:
        if self._mode == "identity":
            return data[: budget + 1]
        try:
            return self._inflate(data, budget + 1)
        except zlib.error:
            raise UpstreamError("The service sent a compressed reply that cannot be decoded.") from None

    def _inflate(self, data: bytes, room: int) -> bytes:
        out: list[bytes] = []
        while data and not self.trailing:
            if self._z is None:
                data, self._pending = self._pending + data, b""
                if self._ended:  # a deflate body is one stream
                    self.trailing = True
                    break
                if len(data) < 2:
                    self._pending = data
                    break
                if self._mode == "gzip":
                    if data[:2] != b"\x1f\x8b":
                        self.trailing = True
                        break
                    self._z = zlib.decompressobj(16 + zlib.MAX_WBITS)
                else:
                    # RFC 9110 deflate is zlib wrapped; some servers send raw deflate.
                    wrapped = (data[0] & 0x0F) == 8 and ((data[0] << 8) | data[1]) % 31 == 0
                    self._z = zlib.decompressobj(zlib.MAX_WBITS if wrapped else -zlib.MAX_WBITS)
            if room <= 0:
                self._held = True
                break
            chunk = self._z.decompress(data, room)
            out.append(chunk)
            room -= len(chunk)
            if self._z.unconsumed_tail:
                self._held = True
                break
            if not self._z.eof:
                break
            # The stream ended. Hand what follows it to the next member, never back to this one,
            # which would keep every further byte in unused_data.
            data, self._z = self._z.unused_data, None
            self._ended = self._mode == "deflate"
        return b"".join(out)

    def leftover(self) -> bool:
        return self._held

    def finish(self, budget: int) -> bytes:
        if self._pending:
            self.trailing = True
            return b""
        if self._z is None:
            return b""
        try:
            return self._z.flush()[: budget + 1]
        except zlib.error:
            raise UpstreamError("The service sent a compressed reply that cannot be decoded.") from None


def _raw_limit(cap: int, compressed: bool) -> int:
    """How many raw bytes a reply may carry for a decoded body of ``cap`` bytes."""

    return (min(2 * cap, MAX_RESPONSE_BYTES) if compressed else cap) + RAW_READ_SLACK_BYTES


def _read_capped(response: httpx.Response, cap: int, deadline: float) -> tuple[bytes, bool]:
    inflater = _Inflater(response.headers.get("content-encoding", ""))
    raw_left = _raw_limit(cap, inflater.compressed)
    chunks: list[bytes] = []
    size = 0
    for raw in response.iter_raw():
        if time.monotonic() > deadline:
            raise UpstreamError("The reply took longer than the time limit.")
        over = len(raw) > raw_left
        raw = raw[:raw_left]
        raw_left -= len(raw)
        budget = cap - size
        data = inflater.feed(raw, budget)
        if over or len(data) > budget or inflater.leftover() or inflater.trailing:
            chunks.append(data[:budget])
            return b"".join(chunks), True
        chunks.append(data)
        size += len(data)
    tail = inflater.finish(cap - size)
    if len(tail) > cap - size or inflater.trailing:
        chunks.append(tail[: cap - size])
        return b"".join(chunks), True
    chunks.append(tail)
    return b"".join(chunks), False


def _strip_query_strings(text: str) -> str:
    """Drop query strings from URLs inside an error message: they may hold a query credential."""

    return re.sub(r"(https?://[^\s'\"?#]+)\?[^\s'\"#]*", r"\1?[query hidden]", text)


def _redact_text(text: str, forms: Sequence[str]) -> str:
    for form in forms:
        if form and form in text:
            text = text.replace(form, REDACTED)
    return text


def _redact_obj(value: Any, forms: Sequence[str]) -> Any:
    if not forms:
        return value
    if isinstance(value, str):
        return _redact_text(value, forms)
    if isinstance(value, (int, float)) and not isinstance(value, bool):
        return REDACTED if _redact_text(str(value), forms) != str(value) else value  # a key echoed as a JSON number
    if isinstance(value, list):
        return [_redact_obj(v, forms) for v in value]
    if isinstance(value, dict):
        return {_redact_text(str(k), forms) if isinstance(k, str) else k: _redact_obj(v, forms) for k, v in value.items()}
    return value


__all__ = [
    "CREDENTIAL_SCHEMES",
    "Credential",
    "KitError",
    "MAX_REDIRECTS",
    "NotConfigured",
    "Origin",
    "RequestRefused",
    "SafeHttpClient",
    "SpecError",
    "Target",
    "UpstreamError",
    "WRITE_METHODS",
    "WriteRoute",
    "route_matches",
]
