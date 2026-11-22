"""safe_http against loopback HTTPS echo servers: where each credential actually goes."""

import base64
import io
import json
import logging
import socket
import unittest

from support import SECRETS, EchoTestCase
from brxt_kit.safe_http import (
    MAX_REDIRECTS,
    Credential,
    NotConfigured,
    SafeHttpClient,
    Target,
    UpstreamError,
    WriteRoute,
)

REDIRECT_CODES = ("301", "302", "303", "307", "308")


def all_secret_forms():
    forms = set()
    for value in SECRETS.values():
        forms |= {value, base64.b64encode(value.encode()).decode(), base64.b64encode(f"{value}:".encode()).decode()}
    forms.add(base64.b64encode(f"user-e:{SECRETS['E_PASSWORD']}".encode()).decode())
    return forms


class Binding(EchoTestCase):
    def targets(self):
        return [
            Target(name="a", base_url=self.origin("a") + "/api", credential=Credential(env="A_TOKEN", scheme="bearer"),
                   write_routes=(WriteRoute("POST", "/items"), WriteRoute("DELETE", "/items/*"))),
            Target(name="b", base_url=self.origin("b"), credential=Credential(env="B_KEY", scheme="header", header_name="X-API-Key")),
            Target(name="c", base_url=self.origin("c") + "/q", credential=Credential(env="C_KEY", scheme="query", query_param="api_key"),
                   get_routes=("/search",), read_post_routes=("/system/find*",)),
        ]

    def assertOnlyOwnCredential(self, request, own_env):
        text = request.raw_text()
        for env, value in SECRETS.items():
            if env != own_env:
                self.assertNotIn(value, text, f"{env} reached {request.server}")

    def test_each_credential_reaches_only_its_own_target(self):
        http = self.client(self.targets())
        http.read("a", "/things")
        http.read("b", "/v1/genes")
        http.read("c", "/search", query={"q": "tp53"})
        requests = {r.server: r for r in self.sent()}
        self.assertEqual(sorted(requests), ["a", "b", "c"])
        self.assertEqual(requests["a"].header("authorization"), f"Bearer {SECRETS['A_TOKEN']}")
        self.assertIsNone(requests["a"].header("x-api-key"))
        self.assertEqual(requests["b"].header("x-api-key"), SECRETS["B_KEY"])
        self.assertIsNone(requests["b"].header("authorization"))
        self.assertIn(("api_key", SECRETS["C_KEY"]), requests["c"].query)
        self.assertIsNone(requests["c"].header("authorization"))
        self.assertOnlyOwnCredential(requests["a"], "A_TOKEN")
        self.assertOnlyOwnCredential(requests["b"], "B_KEY")
        self.assertOnlyOwnCredential(requests["c"], "C_KEY")
        self.assertEqual(self.sent("out"), [])

    def test_basic_schemes(self):
        targets = [
            Target(name="k", base_url=self.origin("a"), credential=Credential(env="A_TOKEN", scheme="basic")),
            Target(name="u", base_url=self.origin("b"), credential=Credential(env="E_PASSWORD", scheme="basic", username_env="E_USER")),
        ]
        http = self.client(targets, env={**SECRETS, "E_USER": "user-e"})
        http.read("k", "/x")
        http.read("u", "/x")
        a, b = self.sent("a")[0], self.sent("b")[0]
        self.assertEqual(a.header("authorization"), "Basic " + base64.b64encode(f"{SECRETS['A_TOKEN']}:".encode()).decode())
        self.assertEqual(b.header("authorization"), "Basic " + base64.b64encode(f"user-e:{SECRETS['E_PASSWORD']}".encode()).decode())

    def test_header_scheme_with_prefix(self):
        target = Target(name="t", base_url=self.origin("a"),
                        credential=Credential(env="A_TOKEN", scheme="header", header_name="Authorization", value_prefix="Token "))
        self.client([target]).read("t", "/x")
        self.assertEqual(self.sent("a")[0].header("authorization"), f"Token {SECRETS['A_TOKEN']}")

    def test_full_url_to_any_other_host_is_refused_and_nothing_is_sent(self):
        http = self.client(self.targets())
        out = self.fleet["out"]
        for path in (
            f"{out.origin}/exfil",
            f"https://localhost:{out.port}/exfil",
            f"{self.origin('b')}/v1/genes",
            f"{self.origin('c')}/q/search",
            f"http://127.0.0.1:{self.fleet['a'].port}/api/things",
            f"https://127.0.0.1:{out.port}/api/things",
            f"//127.0.0.1:{out.port}/exfil",
            f"https://127.0.0.1:{self.fleet['a'].port}@127.0.0.1:{out.port}/exfil",
        ):
            with self.subTest(path=path):
                self.assertRefused(http.read, "a", path)

    def test_full_url_on_own_origin_is_sent_with_the_credential(self):
        http = self.client(self.targets())
        result = http.read("a", f"{self.origin('a')}/api/things?x=1")
        self.assertEqual(result["status_code"], 200)
        request = self.sent("a")[0]
        self.assertEqual(request.path, "/api/things")
        self.assertEqual(request.query, [("x", "1")])
        self.assertEqual(request.header("authorization"), f"Bearer {SECRETS['A_TOKEN']}")

    def test_custom_and_unknown_targets_are_refused(self):
        http = self.client(self.targets())
        for name in ("custom", "out", "A"):
            with self.subTest(name=name):
                self.assertRefused(http.read, name, f"{self.origin('out')}/exfil")

    def test_cross_host_redirects_are_reported_not_followed(self):
        http = self.client(self.targets())
        for target, path in (("a", "/things"), ("b", "/v1"), ("c", "/search")):
            for code in REDIRECT_CODES:
                for location in (f"{self.origin('out')}/stolen", f"{self.origin('b' if target != 'b' else 'a')}/stolen",
                                 f"http://{self.fleet[target].host}:{self.fleet[target].port}/stolen"):
                    with self.subTest(target=target, code=code, location=location):
                        self.fleet.log.clear()
                        result = http.read(target, path, query={"__redirect_to": location, "__redirect_code": code})
                        self.assertIn("redirect_refused", result)
                        self.assertEqual(result["status_code"], int(code))
                        self.assertEqual({r.server for r in self.sent()}, {target})
                        self.assertEqual(len(self.sent()), 1)
                        self.assertNotIn(SECRETS["A_TOKEN"], json.dumps(result))

    def test_a_redirect_carrying_a_user_name_is_not_followed(self):
        http = self.client(self.targets())
        port = self.fleet["a"].port
        result = http.read("a", "/x", query={"__redirect_to": f"https://someone:pw@127.0.0.1:{port}/api/y", "__redirect_code": "302"})
        self.assertIn("redirect_refused", result)
        self.assertEqual([r.path for r in self.sent()], ["/api/x"])

    def test_same_origin_redirect_is_followed_and_keeps_the_credential_there(self):
        http = self.client(self.targets())
        for code in REDIRECT_CODES:
            with self.subTest(code=code):
                self.fleet.log.clear()
                result = http.read("a", "/x", query={"__redirect_to": f"{self.origin('a')}/api/y", "__redirect_code": code})
                self.assertEqual(result["status_code"], 200)
                self.assertEqual([h["to"] for h in result["redirects"]], ["/api/y"])
                hops = self.sent()
                self.assertEqual([(r.server, r.path) for r in hops], [("a", "/api/x"), ("a", "/api/y")])
                self.assertTrue(all(r.header("authorization") == f"Bearer {SECRETS['A_TOKEN']}" for r in hops))

    def test_query_credential_survives_a_same_origin_redirect_once(self):
        http = self.client(self.targets())
        http.read("c", "/search", query={"__redirect_to": f"{self.origin('c')}/q/search?api_key=planted&page=2", "__redirect_code": "302"})
        second = self.sent("c")[1]
        self.assertEqual([v for k, v in second.query if k == "api_key"], [SECRETS["C_KEY"]])
        self.assertIn(("page", "2"), second.query)

    def test_redirect_chains_stop_after_three(self):
        http = self.client(self.targets())
        location = f"{self.origin('a')}/api/end"
        for _ in range(6):
            location = f"{self.origin('a')}/api/hop?" + __import__("urllib.parse").parse.urlencode({"__redirect_to": location, "__redirect_code": "302"})
        self.fleet.log.clear()
        with self.assertRaises(UpstreamError) as caught:
            http.read("a", location)
        self.assertIn("redirect", str(caught.exception))
        self.assertEqual(len(self.sent()), MAX_REDIRECTS + 1)
        self.assertEqual({r.server for r in self.sent()}, {"a"})

    def test_reads_never_send_a_write_method(self):
        http = self.client(self.targets())
        http.read("a", "/things")
        http.read("c", "/system/findProjects", json_body={"limit": 1})
        methods = [(r.server, r.method, r.path) for r in self.sent()]
        self.assertEqual(methods, [("a", "GET", "/api/things"), ("c", "POST", "/q/system/findProjects")])
        self.assertRefused(http.read, "a", "/things", json_body={"x": 1})
        self.assertRefused(http.read, "c", "/other")
        self.assertRefused(http.read, "c", "/system/delete")

    def test_303_turns_a_read_post_into_get(self):
        http = self.client(self.targets())
        http.read("c", "/system/findProjects", json_body={}, )
        self.fleet.log.clear()
        http.read("c", "/system/findProjects", query={"__redirect_to": f"{self.origin('c')}/q/search", "__redirect_code": "303"})
        self.assertEqual([r.method for r in self.sent()], ["POST", "GET"])

    def test_writes_follow_declared_routes_only(self):
        http = self.client(self.targets())
        result = http.write("a", "POST", "/items", json_body={"name": "x"})
        self.assertEqual(result["method"], "POST")
        request = self.sent("a")[0]
        self.assertEqual((request.method, request.path, json.loads(request.body)), ("POST", "/api/items", {"name": "x"}))
        self.assertEqual(request.header("content-type"), "application/json")
        http.write("a", "delete", "/items/7")
        self.assertEqual(self.sent("a")[-1].method, "DELETE")
        self.assertRefused(http.write, "a", "PUT", "/items")
        self.assertRefused(http.write, "a", "DELETE", "/items")
        self.assertRefused(http.write, "a", "POST", "/other")
        self.assertRefused(http.write, "a", "GET", "/items")
        self.assertRefused(http.write, "b", "POST", "/items")
        self.assertRefused(http.write, "a", "POST", f"{self.origin('out')}/items")

    def test_missing_required_credential_is_refused_before_sending(self):
        http = self.client(self.targets(), env={k: v for k, v in SECRETS.items() if k != "A_TOKEN"})
        error = self.assertRefused(http.read, "a", "/things")
        self.assertIsInstance(error, NotConfigured)

    def test_optional_credential_absent_sends_none(self):
        target = Target(name="t", base_url=self.origin("a"), credential=Credential(env="ABSENT_KEY", scheme="bearer", required=False))
        self.client([target]).read("t", "/x")
        self.assertIsNone(self.sent("a")[0].header("authorization"))

    def test_target_without_a_credential_never_carries_one(self):
        targets = self.targets() + [Target(name="open", base_url=self.origin("out") + "/open")]
        self.client(targets).read("open", "/genes")
        request = self.sent("out")[0]
        for value in SECRETS.values():
            self.assertNotIn(value, request.raw_text())

    def test_shared_credential_reaches_only_the_targets_that_declare_it(self):
        targets = [
            Target(name="eutils", base_url=self.origin("a"), credential=Credential(env="A_TOKEN", scheme="query", query_param="api_key")),
            Target(name="blast", base_url=self.origin("b"), credential=Credential(env="A_TOKEN", scheme="query", query_param="api_key")),
            Target(name="other", base_url=self.origin("c")),
        ]
        http = self.client(targets)
        for name in ("eutils", "blast", "other"):
            http.read(name, "/x")
        self.assertIn(("api_key", SECRETS["A_TOKEN"]), self.sent("a")[0].query)
        self.assertIn(("api_key", SECRETS["A_TOKEN"]), self.sent("b")[0].query)
        self.assertNotIn(SECRETS["A_TOKEN"], self.sent("c")[0].raw_text())
        self.assertRefused(http.read, "eutils", f"{self.origin('c')}/x")

    def test_secrets_echoed_by_a_service_are_redacted(self):
        http = self.client(self.targets())
        result = json.dumps(http.read("a", "/things"))
        result += json.dumps(http.read("b", "/x"))
        result += json.dumps(http.read("c", "/search"))
        for form in all_secret_forms():
            self.assertNotIn(form, result)
        self.assertIn("[REDACTED]", result)
        self.assertIn("api_key=%5BREDACTED%5D", json.dumps(http.read("c", "/search")).replace("[REDACTED]", "%5BREDACTED%5D"))

    def test_response_size_is_capped(self):
        target = Target(name="t", base_url=self.origin("a"), max_response_bytes=10_000)
        http = self.client([target])
        result = http.read("t", "/big", query={"__bytes": "50000"})
        self.assertTrue(result["truncated"])
        self.assertEqual(result["bytes_read"], 10_000)
        small = http.read("t", "/small", query={"__bytes": "500"})
        self.assertFalse(small["truncated"])
        self.assertEqual(small["bytes_read"], 500)

    def test_compressed_replies_are_inflated_within_the_cap(self):
        target = Target(name="t", base_url=self.origin("a"), max_response_bytes=100_000)
        result = self.client([target]).read("t", "/bomb", query={"__gzip_bytes": str(50_000_000)})
        self.assertTrue(result["truncated"])
        self.assertEqual(result["bytes_read"], 100_000)

    def test_time_is_capped(self):
        target = Target(name="t", base_url=self.origin("a"), timeout_s=1.0)
        with self.assertRaises(UpstreamError):
            self.client([target]).read("t", "/slow", query={"__sleep": "3"})

    def test_binary_content_is_not_shown(self):
        target = Target(name="t", base_url=self.origin("a"))
        result = self.client([target]).read("t", "/file", query={"__binary": "1"})
        self.assertIsNone(result["body"])
        self.assertIn("Binary content", result["note"])

    def test_errors_never_carry_a_query_credential(self):
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            closed_port = probe.getsockname()[1]
        target = Target(name="t", base_url=f"https://127.0.0.1:{closed_port}",
                        credential=Credential(env="C_KEY", scheme="query", query_param="api_key"))
        with self.assertRaises(UpstreamError) as caught:
            self.client([target]).read("t", "/x")
        self.assertNotIn(SECRETS["C_KEY"], str(caught.exception))

    def test_untrusted_certificates_are_refused(self):
        target = Target(name="t", base_url=self.origin("a"), credential=Credential(env="A_TOKEN", scheme="bearer"))
        http = SafeHttpClient([target], user_agent="t/1", environ=SECRETS, trust_env=False)
        with self.assertRaises(UpstreamError):
            http.read("t", "/x")
        self.assertEqual(self.sent(), [])

    def test_http_library_logs_never_show_a_query_credential(self):
        stream = io.StringIO()
        handler = logging.StreamHandler(stream)
        root = logging.getLogger()
        old_level = root.level
        root.addHandler(handler)
        root.setLevel(logging.DEBUG)
        try:
            self.client(self.targets()).read("c", "/search", query={"q": "x"})
        finally:
            root.removeHandler(handler)
            root.setLevel(old_level)
        self.assertNotIn(SECRETS["C_KEY"], stream.getvalue())

    def test_request_bodies_and_accept_are_checked(self):
        http = self.client(self.targets())
        self.assertRefused(http.write, "a", "POST", "/items", json_body={"x": float("nan")})
        self.assertRefused(http.write, "a", "POST", "/items", json_body={"x": "a" * 1_100_000})
        self.assertRefused(http.read, "a", "/x", accept="text/html\r\nX-Evil: 1")
        http.read("a", "/x", accept="text/csv")
        self.assertEqual(self.sent("a")[-1].header("accept"), "text/csv")

    def test_control_characters_in_a_credential_are_refused(self):
        target = Target(name="t", base_url=self.origin("a"), credential=Credential(env="A_TOKEN", scheme="bearer"))
        self.assertRefused(self.client([target], env={"A_TOKEN": "abc\r\nX-Injected: 1"}).read, "t", "/x")


if __name__ == "__main__":
    unittest.main()
