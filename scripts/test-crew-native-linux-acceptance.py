#!/usr/bin/env python3
import importlib.util
import io
from pathlib import Path
import unittest
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location("acceptance", Path(__file__).with_name("crew-native-linux-acceptance.py"))
acceptance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(acceptance)


class BridgeTests(unittest.TestCase):
    def person(self, response):
        person = acceptance.Person.__new__(acceptance.Person)
        person.bridge = type("Bridge", (), {"stdin": io.BytesIO(), "stdout": io.BytesIO(response)})()
        return person

    def test_signing_payload_canonicalizes_nested_fields(self):
        self.assertEqual(acceptance.encoded(["w", 1001, "n", "run.create", {"z": {"b": 2, "a": 1}, "a": 0}]),
                         b'["w",1001,"n","run.create",{"a":0,"z":{"a":1,"b":2}}]')

    def test_matching_response_is_accepted(self):
        person = self.person(b'{"id":"expected","result":{"ok":true}}\n')
        with patch.object(acceptance.uuid, "uuid4", return_value="expected"), patch.object(acceptance.select, "select", return_value=([1], [], [])):
            self.assertEqual(person.call("hello", {}), {"ok": True})

    def test_response_from_another_request_is_rejected(self):
        person = self.person(b'{"id":"other","result":{}}\n')
        with patch.object(acceptance.uuid, "uuid4", return_value="expected"), patch.object(acceptance.select, "select", return_value=([1], [], [])):
            with self.assertRaisesRegex(RuntimeError, "identity mismatch"):
                person.call("hello", {})

    def test_protocol_error_is_not_success(self):
        person = self.person(b'{"id":"expected","error":{"code":"forbidden"}}\n')
        with patch.object(acceptance.uuid, "uuid4", return_value="expected"), patch.object(acceptance.select, "select", return_value=([1], [], [])):
            with self.assertRaisesRegex(RuntimeError, "refused"):
                person.call("run.create", {})

    def test_timeout_is_not_an_empty_result(self):
        person = self.person(b'')
        with patch.object(acceptance.select, "select", return_value=([], [], [])):
            with self.assertRaisesRegex(RuntimeError, "timed out"):
                person.call("hello", {})

    def test_bob_and_exact_stop_are_attempted_after_alice_close_fails(self):
        alice = Mock()
        bob = Mock()
        alice.close.side_effect = RuntimeError("Alice bridge close failed")
        alice.command.return_value.stdout = b'{"stopped":true,"pid":123}'
        with self.assertRaisesRegex(RuntimeError, "Alice bridge close failed"):
            acceptance.cleanup(alice, bob, Path("/broker"), Path("/state"), {"pid": 123})
        bob.close.assert_called_once_with()
        alice.command.assert_called_once_with([Path("/broker"), "stop", "--state-dir", Path("/state")])

    def test_cleanup_rejects_a_different_broker_pid(self):
        alice = Mock()
        bob = Mock()
        alice.command.return_value.stdout = b'{"stopped":true,"pid":999}'
        with self.assertRaisesRegex(RuntimeError, "Exact broker cleanup failed"):
            acceptance.cleanup(alice, bob, Path("/broker"), Path("/state"), {"pid": 123})


if __name__ == "__main__":
    unittest.main()
