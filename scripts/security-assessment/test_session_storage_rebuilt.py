#!/usr/bin/env python3
"""Offline protocol checks for the rebuilt-daemon acceptance harness."""

import ast
import importlib.util
import json
from pathlib import Path
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch


SPEC = importlib.util.spec_from_file_location(
    "session_storage_rebuilt", Path(__file__).with_name("session_storage_rebuilt.py"))
HARNESS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HARNESS)


class OwnerResponseTests(unittest.TestCase):
    def api(self, body):
        response = Mock()
        response.read.return_value = body
        context = Mock()
        context.__enter__ = Mock(return_value=response)
        context.__exit__ = Mock(return_value=False)
        request = SimpleNamespace(Request=Mock(), urlopen=Mock(return_value=context))
        namespace = {"json": json, "key": "SYNTHETIC_DAEMON_KEY",
                     "urllib": SimpleNamespace(request=request)}
        function = next(node for node in ast.parse(HARNESS.OWNER_PROBE).body
                        if isinstance(node, ast.FunctionDef) and node.name == "api")
        exec(compile(ast.Module(body=[function], type_ignores=[]), "<owner-api>", "exec"), namespace)
        return namespace["api"]

    def test_session_rename_accepts_the_actual_empty_success_response(self):
        self.assertIsNone(self.api(b"")(
            "PUT", "/sessions/synthetic/name", {"name": "SYNTHETIC"}, expect_json=False))

    def test_session_creation_and_read_require_json(self):
        for method in ("POST", "GET"):
            with self.subTest(method=method):
                with self.assertRaises(json.JSONDecodeError):
                    self.api(b"")(method, "/synthetic")
                self.assertEqual(self.api(b'{"name":"SYNTHETIC"}')(method, "/synthetic"),
                                 {"name": "SYNTHETIC"})

    def test_recorded_failed_assertion_remains_in_case_evidence(self):
        result = {"cases": []}
        with self.assertRaises(RuntimeError):
            HARNESS.record_case(result, "synthetic", {"read": True}, {"expected_denial": False})
        self.assertFalse(result["cases"][0]["passed"])
        self.assertEqual(result["cases"][0]["observed"], {"read": True})

    def test_daemon_secret_is_forwarded_by_environment_name(self):
        secret = "SYNTHETIC_PRIVATE_KEY"
        with patch.object(HARNESS, "command") as command:
            HARNESS.inside("synthetic-container", HARNESS.OWNER, ["python3", "-c", "pass"], secret=secret)
        args = command.call_args.args[0]
        self.assertIn("BIOROUTER_SERVER__SECRET_KEY", args)
        self.assertNotIn(secret, " ".join(args))
        self.assertEqual(command.call_args.kwargs["env"]["BIOROUTER_SERVER__SECRET_KEY"], secret)


if __name__ == "__main__":
    unittest.main()
