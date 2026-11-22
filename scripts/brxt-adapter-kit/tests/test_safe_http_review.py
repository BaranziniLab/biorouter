"""Regressions for the 2026-11-20 security review of safe_http (kit 1.0.0).

Each class names the review finding it closes. Every test here fails on kit
1.0.0, and each fix has a mutant run that shows its test fails without it.
"""

import gzip
import json
import random
import time
import unittest
import zlib

import support  # noqa: F401  (sets sys.path)
from support import SECRETS, EchoTestCase
from brxt_kit import safe_http
from brxt_kit import spec as spec_module
from brxt_kit import tools
from brxt_kit.safe_http import (
    MIN_REDACTION_LENGTH,
    REDACTED,
    Credential,
    NotConfigured,
    RequestRefused,
    SafeHttpClient,
    SpecError,
    Target,
    UpstreamError,
    WriteRoute,
    route_matches,
)
from brxt_kit_testing.echo_server import gzip_member

BASE = "https://api.example.org/v2"
# A closed loopback port: a test that must be refused before any connection points
# here, so a broken kit fails on a refused connection rather than reaching a real host.
CLOSED = "https://127.0.0.1:9"


class FakeResponse:
    """What _read_capped reads: headers and raw chunks. Counts how many raw bytes were taken."""

    def __init__(self, chunks, encoding="gzip"):
        self.headers = {"content-encoding": encoding}
        self._chunks = chunks
        self.raw_taken = 0

    def iter_raw(self):
        for chunk in self._chunks:
            self.raw_taken += len(chunk)
            yield chunk


def endless(first: bytes, then: bytes, chunk: int = 65536):
    yield first
    while True:
        yield then * (chunk // len(then))


def read(chunks, cap=1000, encoding="gzip", seconds=30):
    response = FakeResponse(chunks, encoding)
    started = time.monotonic()
    body, truncated = safe_http._read_capped(response, cap, time.monotonic() + seconds)
    return body, truncated, response.raw_taken, time.monotonic() - started


class Finding1Decompression(unittest.TestCase):
    """Finding 1: raw bytes were unbounded, and bytes after a gzip stream were kept in unused_data."""

    def test_bytes_after_the_end_of_a_gzip_stream_end_the_read(self):
        body, truncated, taken, elapsed = read(endless(gzip.compress(b"hi"), b"\0"))
        self.assertEqual(body, b"hi")
        self.assertTrue(truncated)
        self.assertLessEqual(taken, 65536 + 100, "the kit kept reading after the stream ended")
        self.assertLess(elapsed, 1.0)

    def test_bytes_after_the_end_of_a_deflate_stream_end_the_read(self):
        for wrapped in (True, False):
            with self.subTest(wrapped=wrapped):
                z = zlib.compressobj(wbits=zlib.MAX_WBITS if wrapped else -zlib.MAX_WBITS)
                stream = z.compress(b"hello") + z.flush()
                body, truncated, taken, _ = read(endless(stream, b"x"), encoding="deflate")
                self.assertEqual(body, b"hello")
                self.assertTrue(truncated)
                self.assertLessEqual(taken, 65536 + 100)
                body, truncated, _, _ = read([stream], encoding="deflate")
                self.assertEqual((body, truncated), (b"hello", False))

    def test_raw_bytes_are_capped_when_the_body_decodes_to_nothing(self):
        # Empty gzip members are valid and inflate to nothing, so only the raw cap can stop them.
        empty = gzip.compress(b"")
        cap = 1000
        body, truncated, taken, elapsed = read(endless(empty, empty), cap=cap)
        self.assertEqual(body, b"")
        self.assertTrue(truncated)
        self.assertLessEqual(taken, safe_http._raw_limit(cap, True) + 65536)
        self.assertLess(elapsed, 5.0)

    def test_raw_cap_never_exceeds_the_largest_response_plus_slack(self):
        self.assertEqual(
            safe_http._raw_limit(safe_http.MAX_RESPONSE_BYTES, True),
            safe_http.MAX_RESPONSE_BYTES + safe_http.RAW_READ_SLACK_BYTES,
        )
        self.assertEqual(safe_http._raw_limit(1000, False), 1000 + safe_http.RAW_READ_SLACK_BYTES)

    def test_every_gzip_member_is_inflated_in_order_within_the_cap(self):
        members = [gzip_member(k, 4000) for k in range(500)]
        stream = b"".join(gzip.compress(m) for m in members)
        chunks = [stream[i:i + 4096] for i in range(0, len(stream), 4096)]
        body, truncated, _, _ = read(chunks, cap=100_000)
        self.assertTrue(truncated)
        self.assertEqual(body, b"".join(members)[:100_000])
        three = b"".join(gzip.compress(m) for m in members[:3])
        body, truncated, _, _ = read([three[i:i + 7] for i in range(0, len(three), 7)], cap=100_000)
        self.assertEqual((body, truncated), (b"".join(members[:3]), False))

    def test_a_corrupt_body_is_an_upstream_error(self):
        # A stored block whose length and its complement disagree: zlib always refuses it.
        bad_block = b"\x01\x05\x00\x00\x00hello"
        with self.assertRaises(UpstreamError):
            read([b"\x1f\x8b\x08\x00\x00\x00\x00\x00\x00\xff" + bad_block])
        with self.assertRaises(UpstreamError):
            read([b"\x78\x9c" + bad_block], encoding="deflate")


class Finding1OverTheWire(EchoTestCase):
    def target(self, cap):
        return Target(name="t", base_url=self.origin("a"), max_response_bytes=cap)

    def test_a_gzip_stream_with_a_long_tail_ends_at_once(self):
        started = time.monotonic()
        result = self.client([self.target(1000)]).read("t", "/tail", query={"__gzip_tail": str(300 << 20)})
        self.assertLess(time.monotonic() - started, 5.0)
        self.assertTrue(result["truncated"])
        self.assertEqual(result["body"], "hi")

    def test_a_multi_member_body_returns_the_cap_in_order(self):
        cap = 2_000_000
        result = self.client([self.target(cap)]).read("t", "/vcf.gz", query={"__gzip_members": "2000", "__member_bytes": "48000"})
        self.assertTrue(result["truncated"])
        self.assertEqual(result["bytes_read"], cap)
        expected = b"".join(gzip_member(k, 48000) for k in range(cap // 48000 + 1))[:cap].decode()
        self.assertEqual(result["body"], expected)

    def test_a_corrupt_gzip_body_is_an_upstream_error(self):
        with self.assertRaises(UpstreamError):
            self.client([self.target(1000)]).read("t", "/bad", query={"__bad_gzip": "1"})


class Finding2ReadsOnWritePaths(EchoTestCase):
    """Finding 2: the read tool sent GET with the credential to declared write paths."""

    def tamarind(self):
        # The shape of WP-05's TamarindBioAgent: GET /** beside action paths.
        return Target(
            name="tamarind", base_url=self.origin("a") + "/api",
            credential=Credential(env="A_TOKEN", scheme="header", header_name="x-api-key"),
            get_routes=("/**",), read_post_routes=("/result", "/jobs/search"),
            write_routes=(WriteRoute("POST", "/stop-job"), WriteRoute("DELETE", "/delete-job"), WriteRoute("POST", "/jobs/*/rerun")),
        )

    def rest(self):
        # The shape of a REST service: GET lists and reads, POST creates, PATCH updates.
        return Target(
            name="bench", base_url=self.origin("b") + "/api/v2",
            credential=Credential(env="B_KEY", scheme="basic"),
            get_routes=("/**", "/entries", "/entries/*"),
            write_routes=(WriteRoute("POST", "/entries"), WriteRoute("PATCH", "/entries/*"), WriteRoute("POST", "/entries/*/archive")),
        )

    def test_get_on_a_write_path_is_refused_and_nothing_is_sent(self):
        http = self.client([self.tamarind()])
        for path in ("/stop-job", "/delete-job", "/jobs/42/rerun"):
            with self.subTest(path=path):
                error = self.assertRefused(http.read, "tamarind", path, query={"jobName": "victim-job"})
                self.assertIsInstance(error, RequestRefused)
                self.assertIn("declared write", str(error))

    def test_get_on_a_write_path_as_a_server_may_read_it_is_refused(self):
        http = self.client([self.tamarind()])
        for path in (
            "/stop-job/", "/stop%2Djob", "/stop%252Djob", "/Stop-Job", "/STOP-JOB", "/stop-job;jsessionid=1",
            "/stop-job.json", "/stop-job.a.b", "/%5Cstop-job", "/ｓｔｏｐ-job", "/jobs/42/rerun;x",
            "/jobs/42/Rerun", f"{self.origin('a')}/api/stop-job",
        ):
            with self.subTest(path=path):
                self.assertRefused(http.read, "tamarind", path)

    def test_other_reads_on_the_same_target_still_work(self):
        http = self.client([self.tamarind()])
        http.read("tamarind", "/jobs")
        http.read("tamarind", "/stop-job-history")
        http.read("tamarind", "/result", json_body={"jobName": "x"})
        self.assertEqual([(r.method, r.path) for r in self.sent("a")],
                         [("GET", "/api/jobs"), ("GET", "/api/stop-job-history"), ("POST", "/api/result")])

    def test_a_rest_service_names_its_reads_on_write_paths(self):
        http = self.client([self.rest()])
        http.read("bench", "/entries", query={"pageSize": "5"})
        http.read("bench", "/entries/etr_1")
        self.assertEqual([(r.method, r.path) for r in self.sent("b")], [("GET", "/api/v2/entries"), ("GET", "/api/v2/entries/etr_1")])
        # An action under a named read is still a write path that no read route lies within.
        self.assertRefused(http.read, "bench", "/entries/etr_1/archive")

    def test_a_catch_all_never_names_a_write_path(self):
        target = Target(name="t", base_url=self.origin("a"), credential=Credential(env="A_TOKEN", scheme="bearer"),
                        get_routes=("/**", "/jobs/**"), write_routes=(WriteRoute("POST", "/jobs/*/stop"),))
        self.assertRefused(self.client([target]).read, "t", "/jobs/7/stop")

    def test_writes_still_go_through_the_write_tool(self):
        http = self.client([self.tamarind()])
        http.write("tamarind", "POST", "/stop-job", json_body={"jobName": "mine"})
        self.assertEqual([(r.method, r.path) for r in self.sent("a")], [("POST", "/api/stop-job")])

    def test_a_read_post_never_reaches_a_post_write_even_past_the_declaration(self):
        # Defence in depth: a target that slipped past its own declaration check.
        target = Target(name="t", base_url=self.origin("a"), credential=Credential(env="A_TOKEN", scheme="bearer"),
                        get_routes=(), read_post_routes=("/search",), write_routes=(WriteRoute("POST", "/make"),))
        object.__setattr__(target, "read_post_routes", ("/**", "/make"))
        http = self.client([target])
        self.assertRefused(http.read, "t", "/make", json_body={"name": "made without approval"})
        self.assertRefused(http.read, "t", "/MAKE;x", json_body={})
        http.read("t", "/search", json_body={})
        self.assertEqual([r.method for r in self.sent("a")], ["POST"])


class Finding2Declarations(unittest.TestCase):
    def target(self, read_post, writes):
        return Target(name="t", base_url=BASE, credential=Credential(env="K", scheme="bearer"),
                      get_routes=(), read_post_routes=tuple(read_post), write_routes=tuple(WriteRoute(m, p) for m, p in writes))

    def test_a_read_post_route_that_can_reach_a_write_route_is_refused(self):
        for read_post, writes in (
            (["/items/**"], [("POST", "/items")]),  # the reviewer's P05
            (["/items"], [("POST", "/items")]),
            (["/*/describe"], [("POST", "/project-1/*")]),
            (["/system/find*"], [("POST", "/system/*Things")]),
            (["/items/**"], [("PATCH", "/items/*")]),
            (["/a/*/c"], [("DELETE", "/a/b/*")]),
            (["/**"], [("PUT", "/x")]),
        ):
            with self.subTest(read_post=read_post, writes=writes), self.assertRaises(SpecError):
                self.target(read_post, writes)

    def test_read_post_routes_that_cannot_reach_a_write_route_are_accepted(self):
        # DNAnexus, TamarindBio and AdaptyvBio as WP-05 declares them.
        self.target(
            ["/system/find*", "/system/describe*", "/system/resolveDataObjects", "/system/whoami", "/*/describe",
             "/*/*/describe", "/*/listFolder", "/*/getDetails", "/*/listProjects"],
            [("POST", "/project-new"), ("POST", "/*/newFolder"), ("POST", "/*/move"), ("POST", "/*/clone"),
             ("POST", "/*/rename"), ("POST", "/*/addTags"), ("POST", "/*/removeTags"), ("POST", "/*/setProperties"),
             ("POST", "/*/setDetails"), ("POST", "/*/run"), ("POST", "/*/*/run"), ("POST", "/*/terminate")],
        )
        self.target(["/result", "/validate-job", "/jobs/search", "/pipelines/validate"],
                    [("POST", "/submit-job"), ("POST", "/pipelines/submit"), ("POST", "/stop-job"), ("DELETE", "/delete-job")])
        # A literal read POST inside a write route of another method: the author named it.
        self.target(["/experiments/cost-estimate"], [("POST", "/experiments"), ("PATCH", "/experiments/*")])


class PatternAlgebra(unittest.TestCase):
    """The overlap test must never miss an overlap, and the containment test must never claim one that is false."""

    def test_known_cases(self):
        overlap = safe_http._patterns_overlap
        within = safe_http._pattern_within
        self.assertTrue(overlap("/items/**", "/items"))
        self.assertTrue(overlap("/*/describe", "/project-1/*"))
        self.assertTrue(overlap("/find*", "/*Tags"))
        self.assertFalse(overlap("/system/describe*", "/*/move"))
        self.assertFalse(overlap("/*/describe", "/*/run"))
        self.assertFalse(overlap("/a/b", "/a"))
        self.assertTrue(within("/entries", "/entries"))
        self.assertTrue(within("/entries/*", "/entries/*"))
        self.assertTrue(within("/experiments/cost-estimate", "/experiments/*"))
        self.assertFalse(within("/**", "/stop-job"))
        self.assertFalse(within("/*", "/stop-job"))
        self.assertFalse(within("/a/**", "/a/*"))
        self.assertFalse(within("/entries/**", "/entries/*"))

    def test_against_brute_force(self):
        rng = random.Random(20261122)
        segments = ["a", "b", "ab", "*", "a*", "*b", "a*b", "[ab]", "**"]
        names = ["a", "b", "ab", "ba", "aab", "abb", "", "c", "abab"]

        def pattern():
            parts = [rng.choice(segments) for _ in range(rng.randint(0, 3))]
            while sum(p == "**" for p in parts) > 2:
                parts.remove("**")
            return "/" + "/".join(parts)

        paths = ["/" + "/".join(rng.choice(names) for _ in range(n)) for n in range(5) for _ in range(60)]
        for _ in range(2500):
            a, b = pattern(), pattern()
            both = [p for p in paths if route_matches(a, p) and route_matches(b, p)]
            if both:
                self.assertTrue(safe_http._patterns_overlap(a, b), f"{a} and {b} both match {both[0]}")
            if safe_http._pattern_within(a, b):
                stray = [p for p in paths if route_matches(a, p) and not route_matches(b, p)]
                self.assertFalse(stray, f"{a} was taken as within {b}, but {stray[:1]} matches only {a}")


class Finding3DotSegments(unittest.TestCase):
    def test_dot_segments_with_a_parameter_or_in_fullwidth_are_refused(self):
        target = Target(name="t", base_url=BASE, credential=Credential(env="K", scheme="bearer"), get_routes=("/projects/**",))
        http = SafeHttpClient([target], user_agent="t/1", environ={"K": "secret-value-123"})
        base = http._resolve(target, {"K": "secret-value-123"})
        for path in ("/projects/..;/..;/admin/users", "/projects/%2e%2e;/admin", "/projects/.%2e;/admin",
                     "/projects/.;x/admin", "/projects/．．/admin", "/projects/%u002e%u002e/admin"):
            with self.subTest(path=path), self.assertRaises(RequestRefused):
                http._prepare(target, base, path, None)
        self.assertEqual(http._prepare(target, base, "/projects/a;b=1", None).rel_path, "/projects/a;b=1")


class Finding4Redaction(unittest.TestCase):
    def test_a_credential_echoed_as_a_number_is_redacted(self):
        forms = ["8675309123"]
        self.assertEqual(safe_http._redact_obj({"account": 8675309123, "n": 5, "ok": True}, forms),
                         {"account": REDACTED, "n": 5, "ok": True})
        self.assertEqual(safe_http._redact_obj([8675309123.0], forms), [REDACTED])

    def test_a_credential_too_short_to_redact_is_never_sent(self):
        target = Target(name="t", base_url=CLOSED, credential=Credential(env="K", scheme="header", header_name="X-Key"))
        http = SafeHttpClient([target], user_agent="t/1", environ={"K": "k3y9z"}, trust_env=False)
        with self.assertRaises(NotConfigured) as caught:
            http.read("t", "/x")
        self.assertNotIn("k3y9z", str(caught.exception))
        self.assertEqual(MIN_REDACTION_LENGTH, 6)


class Finding6TimeLimits(EchoTestCase):
    def test_a_caller_cannot_lengthen_the_targets_limit(self):
        target = Target(name="t", base_url=self.origin("a"), timeout_s=1.0)
        started = time.monotonic()
        with self.assertRaises(UpstreamError):
            self.client([target]).read("t", "/slow", query={"__sleep": "3"}, timeout_s=60)
        self.assertLess(time.monotonic() - started, 2.5)


class Finding6And8Tools(EchoTestCase):
    def build(self, target):
        spec = spec_module.AdapterSpec(
            id="reviewagent", display_name="ReviewAgent", service="review", service_title="Review",
            kind="adapter", instructions="", targets=(target,), notice=None, log_level_env=None,
        )
        return tools.build_server(spec, version="0.2.0", client=self.client([target]))

    def call(self, server, name, arguments):
        import asyncio
        from fastmcp import Client

        async def go():
            async with Client(server, mode="legacy") as client:
                result = await client.call_tool(name, arguments, raise_on_error=False)
                return bool(result.is_error), "\n".join(getattr(b, "text", "") for b in result.content)

        return asyncio.run(go())

    def test_the_read_tool_keeps_the_targets_own_time_limit(self):
        server = self.build(Target(name="a", base_url=self.origin("a"), timeout_s=1.0))
        started = time.monotonic()
        is_error, text = self.call(server, "review_get", {"target": "a", "path": "/slow", "query": {"__sleep": "3"}})
        self.assertTrue(is_error, text)
        self.assertLess(time.monotonic() - started, 2.5)

    def test_a_status_failure_is_a_redacted_tool_error(self):
        from unittest import mock

        target = Target(name="a", base_url=self.origin("a"), credential=Credential(env="A_TOKEN", scheme="bearer"))
        server = self.build(target)
        with mock.patch.object(SafeHttpClient, "describe", side_effect=RuntimeError(f"boom {SECRETS['A_TOKEN']}")):
            is_error, text = self.call(server, "review_status", {})
        self.assertTrue(is_error)
        self.assertNotIn(SECRETS["A_TOKEN"], text)
        self.assertNotIn("boom", text)
        self.assertIn("internal error", text)


class Finding7Accept(unittest.TestCase):
    def test_accept_admits_spaces_and_tabs_only_in_ascii(self):
        target = Target(name="t", base_url=CLOSED)
        http = SafeHttpClient([target], user_agent="t/1", environ={}, trust_env=False)
        for accept in ("application/json\r\n;x=1", "application/json\r\n\r\n;x=1", "application/json\r\n\t;x=1",
                       "application/json ;x=1", "application/json\x85;x=1", "application/json\x0b;x=1",
                       "application/json" + ";a=b" * 70):
            with self.subTest(accept=accept[:30]), self.assertRaises(RequestRefused):
                http.read("t", "/x", accept=accept)


class Finding8NonKitErrors(unittest.TestCase):
    def test_a_setting_that_is_not_a_url_is_not_configured_and_spares_other_targets(self):
        targets = [
            Target(name="api", base_url="https://api.example.org", credential=Credential(env="API_KEY", scheme="bearer")),
            Target(name="omero", base_url="", base_url_env="OMERO_BASE_URL", credential=Credential(env="OMERO_KEY", scheme="bearer")),
        ]
        env = {"API_KEY": "api-key-123456", "OMERO_KEY": "omero-key-123456", "OMERO_BASE_URL": "https://omero.lab／example.org"}
        http = SafeHttpClient(targets, user_agent="t/1", environ=env, trust_env=False)
        with self.assertRaises(NotConfigured):
            http._resolve(targets[1], env)
        self.assertEqual(http._resolve(targets[0], env).url, "https://api.example.org")
        rows = {row["name"]: row for row in http.describe()}
        self.assertFalse(rows["omero"]["configured"])
        self.assertTrue(rows["api"]["configured"])

    def test_a_full_url_that_cannot_be_read_is_refused(self):
        target = Target(name="t", base_url="https://127.0.0.1:8443/api")
        http = SafeHttpClient([target], user_agent="t/1", environ={}, trust_env=False)
        with self.assertRaises(RequestRefused):
            http.read("t", "https://127.0.0.1：8443/api/x")

    def test_header_values_must_be_ascii(self):
        target = Target(name="t", base_url=CLOSED, credential=Credential(env="K", scheme="bearer"))
        http = SafeHttpClient([target], user_agent="t/1", environ={"K": "sécret-value-1"}, trust_env=False)
        with self.assertRaises(NotConfigured):
            http.read("t", "/x")
        with self.assertRaises(SpecError):
            Target(name="t", base_url=BASE, headers=(("X-Lab", "café"),))


class Finding9IdnHosts(unittest.TestCase):
    def test_the_origin_is_the_host_the_connection_uses(self):
        import httpx

        self.assertEqual(safe_http._origin_of(httpx.URL("https://xn--bcher-kva.example/api")).host, "xn--bcher-kva.example")
        self.assertEqual(safe_http._origin_of(httpx.URL("https://bücher.example/api")).host, "xn--bcher-kva.example")


if __name__ == "__main__":
    unittest.main()
