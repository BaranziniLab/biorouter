#!/usr/bin/env python3
"""Offline protocol checks for the rebuilt-daemon acceptance harness."""

import ast
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tempfile
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

    def test_graceful_stop_allows_a_slow_drain_and_confirms_process_exit(self):
        alive = subprocess.CompletedProcess([], 0)
        stopped = subprocess.CompletedProcess([], 1)
        with patch.object(HARNESS, "inside", side_effect=[alive, alive, alive, stopped]) as inside, \
                patch.object(HARNESS.time, "monotonic", side_effect=[0, 1, 15, 29]), \
                patch.object(HARNESS.time, "sleep"):
            HARNESS.stop_daemon("synthetic")
        self.assertEqual(inside.call_count, 4)

    def test_graceful_stop_timeout_never_counts_as_an_exit(self):
        with patch.object(HARNESS, "inside") as inside, \
                patch.object(HARNESS.time, "monotonic", side_effect=[0, 31]):
            with self.assertRaisesRegex(RuntimeError, "did not stop"):
                HARNESS.stop_daemon("synthetic")
        self.assertEqual(inside.call_count, 1)


class UndoFixtureTests(unittest.TestCase):
    def execute_probe(self, home, phase, api):
        def fixture_path(path):
            path = Path(path)
            return path if path.is_relative_to(home) else home / path.relative_to('/home/br')
        namespace = {'Path': fixture_path, 'json': json, 'os': os, 'api': api}
        source = HARNESS.UNDO_PROBE[len(HARNESS.API_PROBE):]
        output = io.StringIO()
        with patch.object(os, 'geteuid', return_value=10001), \
                patch('sys.argv', ['probe', phase]), patch('sys.stdout', output):
            exec(compile(source, '<undo-probe>', 'exec'), namespace)
        return json.loads(output.getvalue())

    def test_editor_snapshot_and_restart_undo_protocol(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            (home / 'fixture.json').write_text(json.dumps({'id': 'synthetic-session'}))
            store = home / 'app-root/config/developer_undo/synthetic.json'
            requests = []

            def api(method, path, body):
                requests.append((method, path, body))
                self.assertEqual(body['session_id'], 'synthetic-session')
                if path == '/agent/resume':
                    self.assertTrue(body['load_model_and_extensions'])
                    return {'session': {'id': 'synthetic-session'}, 'initializing': False,
                            'extension_results': [{'name': 'developer', 'success': True}]}
                self.assertEqual(path, '/agent/call_tool')
                self.assertEqual(body['name'], 'developer__text_editor')
                arguments = body['arguments']
                self.assertEqual(arguments['path'], str(home / 'undo-target.txt'))
                target = home / 'undo-target.txt'
                if arguments['command'] == 'write':
                    store.parent.mkdir(parents=True)
                    store.parent.chmod(0o700)
                    store.write_text(json.dumps({'files': {'bound-identity': [
                        {'content': target.read_text(), 'legacy': False}]}}))
                    store.chmod(0o600)
                    target.write_text(arguments['file_text'])
                else:
                    self.assertEqual(arguments['command'], 'undo_edit')
                    history = json.loads(store.read_text())
                    target.write_text(history['files']['bound-identity'].pop()['content'])
                    store.write_text(json.dumps(history))
                return {'is_error': False, 'content': []}

            written = self.execute_probe(home, 'write', api)
            restored = self.execute_probe(home, 'restore', api)
            self.assertTrue(written['persisted_original_snapshot'])
            self.assertTrue(restored['original_file_restored'])
            self.assertTrue(restored['snapshot_consumed'])
            self.assertEqual([path for _, path, _ in requests],
                             ['/agent/call_tool', '/agent/resume', '/agent/call_tool'])

    def test_editor_error_does_not_count_as_a_successful_snapshot(self):
        with tempfile.TemporaryDirectory() as temporary:
            home = Path(temporary)
            (home / 'fixture.json').write_text(json.dumps({'id': 'synthetic-session'}))
            with self.assertRaisesRegex(AssertionError, 'did not complete'):
                self.execute_probe(home, 'write', Mock(return_value={'is_error': True}))

    def test_directory_privacy_without_traversable_parents_is_inconclusive(self):
        observed = {'observer_euid': 10002, 'public_control_read': True,
                    'ancestors_traversable': False, 'directory_list_denied': True,
                    'snapshot_open_denied': True, 'has_daemon_secret': False}
        result = {'cases': []}
        with self.assertRaisesRegex(RuntimeError, 'fixture assertions failed'):
            HARNESS.record_undo_observer(result, 'observer_fresh_undo', observed)
        self.assertFalse(result['cases'][0]['assertions']['parents_traversable'])


if __name__ == "__main__":
    unittest.main()
