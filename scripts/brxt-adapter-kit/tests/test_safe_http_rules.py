"""Declaration and path rules of safe_http that need no network."""

import unittest

import support  # noqa: F401  (sets sys.path)
from brxt_kit.safe_http import (
    Credential,
    NotConfigured,
    RequestRefused,
    SafeHttpClient,
    SpecError,
    Target,
    WriteRoute,
    route_matches,
)

BASE = "https://api.example.org/v2"


def client(*targets: Target, env=None) -> SafeHttpClient:
    return SafeHttpClient(list(targets) or [Target(name="t", base_url=BASE)], user_agent="t/1", environ=env or {})


class DeclarationRules(unittest.TestCase):
    def test_base_url_must_be_https(self):
        for base in ("http://api.example.org", "ftp://api.example.org", "api.example.org", "//api.example.org"):
            with self.subTest(base=base), self.assertRaises(SpecError):
                Target(name="t", base_url=base)

    def test_base_url_carries_no_user_query_or_fragment(self):
        for base in (
            "https://user:pw@api.example.org",
            "https://api.example.org/v1?key=1",
            "https://api.example.org/v1#frag",
            "https://api.example.org/a/../b",
            "https://api.example.org/a\\b",
            "https://api.example.org/ a",
            "https://[::1]/v1",
        ):
            with self.subTest(base=base), self.assertRaises(SpecError):
                Target(name="t", base_url=base)

    def test_there_is_no_custom_target(self):
        with self.assertRaises(SpecError):
            Target(name="custom", base_url=BASE)

    def test_credential_schemes_are_closed(self):
        with self.assertRaises(SpecError):
            Credential(env="K", scheme="cookie")
        with self.assertRaises(SpecError):
            Credential(env="K", scheme="header")  # no header_name
        with self.assertRaises(SpecError):
            Credential(env="K", scheme="query")  # no query_param
        with self.assertRaises(SpecError):
            Credential(env="K", scheme="bearer", header_name="X-Key")
        with self.assertRaises(SpecError):
            Credential(env="K", scheme="header", header_name="Host")
        with self.assertRaises(SpecError):
            Credential(env="lower", scheme="bearer")
        with self.assertRaises(SpecError):
            Credential(env="K", scheme="bearer", username_env="U")

    def test_static_headers_cannot_carry_a_credential(self):
        for name in ("Authorization", "Cookie", "Proxy-Authorization", "Host", "User-Agent"):
            with self.subTest(name=name), self.assertRaises(SpecError):
                Target(name="t", base_url=BASE, headers=((name, "x"),))
        with self.assertRaises(SpecError):
            Target(
                name="t",
                base_url=BASE,
                credential=Credential(env="K", scheme="header", header_name="X-Key"),
                headers=(("X-Key", "static"),),
            )

    def test_env_query_cannot_send_the_credential(self):
        credential = Credential(env="K", scheme="query", query_param="api_key")
        with self.assertRaises(SpecError):
            Target(name="t", base_url=BASE, credential=credential, env_query=(("api_key", "OTHER"),))
        with self.assertRaises(SpecError):
            Target(name="t", base_url=BASE, credential=credential, env_query=(("mailto", "K"),))

    def test_routes_are_paths(self):
        for pattern in ("projects", "/a/../b", "/a?b", "/a b", "/a/**x", "/**/**/**/x"):
            with self.subTest(pattern=pattern), self.assertRaises(SpecError):
                Target(name="t", base_url=BASE, get_routes=(pattern,))
        with self.assertRaises(SpecError):
            WriteRoute(method="GET", pattern="/x")

    def test_targets_sharing_an_origin_cannot_carry_different_credentials(self):
        a = Target(name="a", base_url="https://api.example.org/a", credential=Credential(env="KA", scheme="bearer"))
        b = Target(name="b", base_url="https://api.example.org/b", credential=Credential(env="KB", scheme="bearer"))
        with self.assertRaises(SpecError):
            SafeHttpClient([a, b], user_agent="t/1", environ={})
        c = Target(name="c", base_url="https://api.example.org/c")
        SafeHttpClient([a, c], user_agent="t/1", environ={})  # a target without a credential may share
        d = Target(name="d", base_url="https://api.example.org/d", credential=Credential(env="KA", scheme="bearer"))
        SafeHttpClient([a, d], user_agent="t/1", environ={})  # the same credential may share


class RouteMatching(unittest.TestCase):
    def test_single_segment_glob(self):
        self.assertTrue(route_matches("/system/find*", "/system/findProjects"))
        self.assertFalse(route_matches("/system/find*", "/system/findProjects/extra"))
        self.assertTrue(route_matches("/*/describe", "/project-1/describe"))
        self.assertFalse(route_matches("/*/describe", "/a/b/describe"))

    def test_any_depth(self):
        self.assertTrue(route_matches("/**", "/"))
        self.assertTrue(route_matches("/**", "/a/b/c"))
        self.assertTrue(route_matches("/projects/**", "/projects"))
        self.assertTrue(route_matches("/projects/**", "/projects/x/y"))
        self.assertFalse(route_matches("/projects/**", "/entries"))
        self.assertTrue(route_matches("/**/describe", "/a/b/describe"))

    def test_case_sensitive(self):
        self.assertFalse(route_matches("/Entries", "/entries"))


class PathPreparation(unittest.TestCase):
    def setUp(self):
        self.target = Target(name="t", base_url=BASE, credential=Credential(env="K", scheme="query", query_param="api_key"))
        self.http = client(self.target, env={"K": "secret-value-123"})
        self.base = self.http._resolve(self.target, {"K": "secret-value-123"})

    def prepare(self, path, query=None):
        return self.http._prepare(self.target, self.base, path, query)

    def test_relative_paths(self):
        self.assertEqual(self.prepare("projects").rel_path, "/projects")
        self.assertEqual(self.prepare("/projects/1").rel_path, "/projects/1")
        self.assertEqual(self.prepare("").rel_path, "/")
        prepared = self.prepare("/search?q=tp53&limit=5", {"page": 2})
        self.assertEqual(prepared.query, (("q", "tp53"), ("limit", "5"), ("page", "2")))

    def test_full_url_on_own_origin_is_reduced_to_a_path(self):
        self.assertEqual(self.prepare("https://api.example.org/v2/projects?x=1").rel_path, "/projects")
        self.assertEqual(self.prepare("HTTPS://API.EXAMPLE.ORG/v2/projects").rel_path, "/projects")
        self.assertEqual(self.prepare("https://api.example.org:443/v2").rel_path, "/")

    def test_full_url_elsewhere_is_refused(self):
        for url in (
            "https://evil.example/v2/projects",
            "https://api.example.org.evil.example/v2/x",
            "https://api.example.org:8443/v2/x",
            "http://api.example.org/v2/x",
            "https://api.example.org/v1/x",
            "https://api.example.org/v2x/y",
            "https://api.example.org@evil.example/v2/x",
            "https://evil.example#@api.example.org/v2/x",
            "https://user:pw@api.example.org/v2/x",
            "wss://api.example.org/v2/x",
            "file:///etc/passwd",
        ):
            with self.subTest(url=url), self.assertRaises(RequestRefused):
                self.prepare(url)

    def test_hostile_relative_paths_are_refused(self):
        for path in (
            "//evil.example/x",
            "/a/../../b",
            "/a/%2e%2e/b",
            "/a/%2E%2E%2fb",
            "/a/%252e%252e/b",
            "/a\\b",
            "/a b",
            "/a\r\nX: y",
            "/a%0d%0aX:%20y",
            "/a%00",
            "/a#fragment",
            "x" * 5000,
        ):
            with self.subTest(path=path[:40]), self.assertRaises(RequestRefused):
                self.prepare(path)

    def test_the_credential_parameter_is_reserved(self):
        with self.assertRaises(RequestRefused):
            self.prepare("/search", {"api_key": "attacker"})
        with self.assertRaises(RequestRefused):
            self.prepare("/search?API_KEY=attacker")

    def test_query_values_are_plain(self):
        with self.assertRaises(RequestRefused):
            self.prepare("/search", {"q": {"nested": 1}})
        with self.assertRaises(RequestRefused):
            self.prepare("/search", {"q": "a\nb"})
        with self.assertRaises(RequestRefused):
            self.prepare("/search", ["not", "a", "mapping"])
        self.assertEqual(self.prepare("/s", {"a": True, "b": [1, 2], "c": None}).query, (("a", "true"), ("b", "1"), ("b", "2")))


class BaseResolution(unittest.TestCase):
    def test_placeholder_must_be_one_label(self):
        target = Target(name="bench", base_url="https://{TENANT}.bench.example/api/v2", host_pattern=r"[a-z0-9-]+\.bench\.example")
        http = client(target)
        self.assertEqual(http._resolve(target, {"TENANT": "MyLab"}).url, "https://mylab.bench.example/api/v2")
        for value in ("evil.example/x", "mylab.bench.example", "a@b", "", "-x", "a b", "x" * 70):
            with self.subTest(value=value), self.assertRaises(NotConfigured):
                http._resolve(target, {"TENANT": value})

    def test_base_from_environment_obeys_the_host_pattern(self):
        target = Target(name="omero", base_url="", base_url_env="OMERO_BASE_URL", host_pattern=r"omero\.[a-z0-9.-]+")
        http = client(target)
        self.assertEqual(http._resolve(target, {"OMERO_BASE_URL": "https://omero.lab.example/api/"}).url, "https://omero.lab.example/api")
        for value in ("http://omero.lab.example", "https://evil.example", "https://u:p@omero.lab.example", "https://omero.lab.example?x=1"):
            with self.subTest(value=value), self.assertRaises(NotConfigured):
                http._resolve(target, {"OMERO_BASE_URL": value})
        with self.assertRaises(NotConfigured):
            http._resolve(target, {})

    def test_resolved_targets_that_collide_with_different_credentials_are_refused(self):
        a = Target(name="a", base_url="https://{HOST_A}.example.org", credential=Credential(env="KA", scheme="bearer"))
        b = Target(name="b", base_url="https://{HOST_B}.example.org", credential=Credential(env="KB", scheme="bearer"))
        http = client(a, b)
        env = {"HOST_A": "same", "HOST_B": "same", "KA": "aaaaaaaa", "KB": "bbbbbbbb"}
        with self.assertRaises(NotConfigured):
            http._resolve(a, env)

    def test_status_reports_names_never_values(self):
        target = Target(name="t", base_url=BASE, credential=Credential(env="K", scheme="bearer"))
        http = client(target, env={"K": "super-secret-value"})
        described = http.describe()
        self.assertNotIn("super-secret-value", repr(described))
        self.assertEqual(described[0]["credential"], {"env": "K", "scheme": "bearer", "required": True, "present": True})
        missing = client(target, env={}).describe()[0]
        self.assertFalse(missing["configured"])
        self.assertIn("Set K", missing["problem"])

    def test_unknown_target_is_refused(self):
        http = client()
        for name in ("custom", "other", ""):
            with self.subTest(name=name), self.assertRaises(RequestRefused):
                http.target(name)


if __name__ == "__main__":
    unittest.main()
