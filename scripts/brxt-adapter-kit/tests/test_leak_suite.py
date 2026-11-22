"""The leak suite passes on the kit, and fails on every broken kit it is meant to catch.

A check that cannot fail proves nothing. Each mutant below reintroduces one way a
credential has leaked or could leak (the old adapter template's behaviour among
them), and the test asserts the suite reports it.
"""

import unittest
from unittest import mock

import httpx

from support import adapter_data
from brxt_kit import safe_http
from brxt_kit_testing import leak_suite


def run(name: str) -> leak_suite.LeakReport:
    return leak_suite.run_in_process(adapter_data(name), kit_package="brxt_kit")


def failed_checks(report: leak_suite.LeakReport) -> set[str]:
    return {f.check for f in report.failures()}


class TheKitPasses(unittest.TestCase):
    def test_sample_adapter_with_every_credential_scheme(self):
        report = run("sample-adapter.json")
        report.assert_clean()
        self.assertGreater(report.checks_run(), 2000)
        covered = {f.target for f in report.findings}
        self.assertTrue({"alpha", "beta", "gamma", "delta", "epsilon", "open"} <= covered)

    def test_single_target_read_only_adapter(self):
        report = run("single-adapter.json")
        report.assert_clean()
        self.assertIn("tool_shapes:no_write_tool_without_write_routes", {f.check for f in report.findings})

    def test_sunset_release(self):
        run("sunset-adapter.json").assert_clean()

    def test_an_empty_report_is_not_a_pass(self):
        with self.assertRaises(AssertionError):
            leak_suite.LeakReport().assert_clean()


class TheSuiteCatchesBrokenKits(unittest.TestCase):
    """Each mutant must make the suite fail, on the check named for its defect."""

    def assertCaught(self, report, *expected_prefixes):
        failed = failed_checks(report)
        self.assertTrue(failed, "the leak suite passed a broken kit")
        for prefix in expected_prefixes:
            self.assertTrue(any(check.startswith(prefix) for check in failed), f"no failure for {prefix}: {sorted(failed)[:10]}")

    def test_old_template_sends_full_urls_with_every_key(self):
        """The 0.1.0 adapters: a full URL in path went anywhere, keys went everywhere, redirects followed."""

        def old_read(self, target_name, path, *, query=None, json_body=None, accept=None, timeout_s=None):
            target = self.target(target_name)
            environ = self.env()
            base = self._resolve(target, environ)
            url = path if path.startswith(("http://", "https://")) else base.url + "/" + path.lstrip("/")
            headers = {}
            params = dict(query or {})
            for other in self.targets:
                credential = other.credential
                value = environ.get(credential.env) if credential else None
                if not value:
                    continue
                if credential.scheme == "query":
                    params[credential.query_param] = value
                else:
                    headers.setdefault("Authorization", f"Bearer {value}")
            with httpx.Client(follow_redirects=True, verify=self._verify) as client:
                response = client.request("GET", url, params=params, headers=headers)
            return {"url": str(response.url), "status_code": response.status_code, "body": response.text}

        with mock.patch.object(safe_http.SafeHttpClient, "read", old_read):
            report = run("sample-adapter.json")
        self.assertCaught(report, "full_url:outsider", "read:no_foreign_credential", "redirect:outsider", "read:no_secret_in_result")

    def test_following_redirects_in_the_http_library(self):
        real_client = httpx.Client

        def following(*args, **kwargs):
            kwargs["follow_redirects"] = True
            client = real_client(*args, **kwargs)
            real_send = client.send
            client.send = lambda request, **kw: real_send(request, follow_redirects=True, **kw)
            return client

        with mock.patch.object(safe_http.httpx, "Client", following):
            report = run("sample-adapter.json")
        self.assertCaught(report, "redirect:outsider")

    def test_redirect_origin_check_removed(self):
        def send_follows_anything(self, *args, **kwargs):
            with mock.patch.object(safe_http, "_origin_of", lambda url: args[1].origin):
                return original_send(self, *args, **kwargs)

        original_send = safe_http.SafeHttpClient._send
        with mock.patch.object(safe_http.SafeHttpClient, "_send", send_follows_anything):
            report = run("sample-adapter.json")
        self.assertCaught(report, "redirect:outsider")

    def test_every_credential_attached_to_every_target(self):
        real_auth = safe_http.SafeHttpClient._auth

        def attach_all(self, target, environ):
            headers, params = {}, []
            for other in self.targets:
                if other.credential and environ.get(other.credential.env):
                    h, p = real_auth(self, other, environ)
                    for key, value in h.items():
                        headers[key if key not in headers else f"X-Extra-{other.name}"] = value
                    params += p
            return headers, params

        with mock.patch.object(safe_http.SafeHttpClient, "_auth", attach_all):
            report = run("sample-adapter.json")
        self.assertCaught(report, "read:no_foreign_credential")

    def test_full_url_host_check_removed(self):
        def reduce_anything(self, target, base, url):
            parts = safe_http.urllib.parse.urlsplit(url)
            return (parts.path or "/"), safe_http._parse_query_string(parts.query)

        real_send = safe_http.SafeHttpClient._send
        captured = {}

        def prepare_and_remember(self, target, base, path, query):
            captured["url"] = path
            return original_prepare(self, target, base, path, query)

        def send_to_named_url(self, target, base, method, prepared, *args):
            url = captured.get("url", "")
            if "://" in url and not url.startswith("//"):
                parts = safe_http.urllib.parse.urlsplit(url)
                try:
                    port = parts.port or 443
                except ValueError:
                    port = 443
                if parts.hostname:
                    base = safe_http.ResolvedBase(safe_http.Origin(parts.scheme or "https", parts.hostname, port), "")
            return real_send(self, target, base, method, prepared, *args)

        original_prepare = safe_http.SafeHttpClient._prepare
        with mock.patch.object(safe_http.SafeHttpClient, "_reduce_full_url", reduce_anything), \
                mock.patch.object(safe_http.SafeHttpClient, "_prepare", prepare_and_remember), \
                mock.patch.object(safe_http.SafeHttpClient, "_send", send_to_named_url):
            report = run("sample-adapter.json")
        self.assertCaught(report, "full_url:outsider_by_ip", "full_url:other_target")

    def test_redaction_removed(self):
        with mock.patch.object(safe_http, "_redact_obj", lambda value, forms: value):
            report = run("sample-adapter.json")
        self.assertCaught(report, "read:no_secret_in_result")

    def test_read_sends_a_write_method(self):
        original_send = safe_http.SafeHttpClient._send

        def put_instead(self, target, base, method, *args):
            return original_send(self, target, base, "PUT" if method == "GET" else method, *args)

        with mock.patch.object(safe_http.SafeHttpClient, "_send", put_instead):
            report = run("sample-adapter.json")
        self.assertCaught(report, "read:read_method")

    def test_status_leaks_a_value(self):
        original = safe_http.SafeHttpClient.describe

        def leaky(self):
            rows = original(self)
            for row, target in zip(rows, self.targets, strict=True):
                if target.credential:
                    row["value"] = self.env().get(target.credential.env)
            return rows

        with mock.patch.object(safe_http.SafeHttpClient, "describe", leaky):
            report = run("sample-adapter.json")
        self.assertCaught(report, "status:no_values")

    def test_reserved_query_parameter_accepted(self):
        with mock.patch.object(safe_http, "_reserved_params", lambda target: set()):
            report = run("sample-adapter.json")
        self.assertCaught(report, "query_credential:reserved")

    def test_reads_reach_write_paths(self):
        """Kit 1.0.0: the read tool sent GET with the credential to a declared write's path."""

        with mock.patch.object(safe_http, "_unnamed_write_routes", lambda *args: []):
            report = run("sample-adapter.json")
        self.assertCaught(report, "read:write_path_refused:DELETE /items/*")

    def test_dot_segment_check_removed(self):
        with mock.patch.object(safe_http, "_check_rel_path", lambda rel, refuse=None: None):
            report = run("sample-adapter.json")
        self.assertCaught(report, "path:dot_segments")

    def test_write_route_check_removed(self):
        def any_route(self, target_name, method, path, **kwargs):
            target = self.target(target_name)
            environ = self.env()
            base = self._resolve(target, environ)
            prepared = self._prepare(target, base, path, kwargs.get("query"))
            return self._send(target, base, method.upper(), prepared, kwargs.get("json_body"), None, None, environ)

        with mock.patch.object(safe_http.SafeHttpClient, "write", any_route):
            report = run("sample-adapter.json")
        self.assertCaught(report, "write:undeclared")


if __name__ == "__main__":
    unittest.main()
