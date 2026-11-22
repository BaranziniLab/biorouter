"""Tool shapes, annotations, the status report, and the sunset and tombstone servers."""

import asyncio
import json
import unittest
from unittest import mock

from support import SECRETS, EchoTestCase, adapter_data
from brxt_kit import spec as spec_module
from brxt_kit import tools
from brxt_kit.safe_http import Credential, SafeHttpClient, SpecError, Target


async def _list_and_call(server, calls=()):
    from fastmcp import Client

    async with Client(server, mode="legacy") as client:
        listed = {t.name: t for t in await client.list_tools()}
        results = []
        for name, arguments in calls:
            result = await client.call_tool(name, arguments, raise_on_error=False)
            results.append((bool(result.is_error), "\n".join(getattr(b, "text", "") for b in result.content)))
        instructions = client.instructions or ""
        return listed, results, instructions


def run(server, calls=()):
    return asyncio.run(_list_and_call(server, calls))


class ToolShapes(unittest.TestCase):
    def test_adapter_with_writes_has_three_tools_with_honest_annotations(self):
        spec = spec_module.load_adapter_spec(adapter_data("sample-adapter.json"))
        listed, _, instructions = run(tools.build_server(spec, version="0.2.0"))
        self.assertEqual(sorted(listed), ["sample_get", "sample_status", "sample_write"])
        get, status, write = listed["sample_get"], listed["sample_status"], listed["sample_write"]
        self.assertTrue(get.annotations.read_only_hint)
        self.assertFalse(get.annotations.destructive_hint)
        self.assertTrue(status.annotations.read_only_hint)
        self.assertFalse(write.annotations.read_only_hint)
        self.assertTrue(write.annotations.destructive_hint)
        schema = json.dumps(get.input_schema)
        self.assertNotIn("allow_mutation", schema)
        target_enum = get.input_schema["properties"]["target"]["enum"]
        self.assertEqual(target_enum, ["alpha", "beta", "gamma", "delta", "epsilon", "open"])
        write_schema = write.input_schema
        # A single choice is written as "const" in JSON Schema.
        self.assertEqual(write_schema["properties"]["target"].get("enum", [write_schema["properties"]["target"].get("const")]), ["alpha"])
        self.assertEqual(write_schema["properties"]["method"]["enum"], ["POST", "DELETE"])
        self.assertIn("asks the person before every write", instructions)

    def test_read_only_adapter_has_no_write_tool(self):
        spec = spec_module.load_adapter_spec(adapter_data("single-adapter.json"))
        listed, _, instructions = run(tools.build_server(spec, version="0.2.0"))
        self.assertEqual(sorted(listed), ["single_get", "single_status"])
        self.assertNotIn("_write", instructions)

    def test_no_command_runner_exists(self):
        spec = spec_module.load_adapter_spec(adapter_data("sample-adapter.json"))
        listed, _, _ = run(tools.build_server(spec, version="0.2.0"))
        self.assertFalse([n for n in listed if "command" in n or n.startswith(("plan_", "run_"))])

    def test_status_names_settings_and_shows_no_value(self):
        spec = spec_module.load_adapter_spec(adapter_data("single-adapter.json"))
        client = SafeHttpClient(spec.targets, user_agent="t/1", environ={"SINGLE_TOKEN": "very-secret-token-1"})
        _, results, _ = run(tools.build_server(spec, version="0.2.0", client=client), [("single_status", {})])
        is_error, text = results[0]
        self.assertFalse(is_error)
        self.assertNotIn("very-secret-token-1", text)
        payload = json.loads(text)
        self.assertEqual(payload["targets"][0]["credential"]["present"], True)
        self.assertEqual(payload["kit_version"], tools.KIT_VERSION)
        self.assertEqual(payload["version"], "0.2.0")


class Refusals(EchoTestCase):
    def build(self, env=None):
        target = Target(name="a", base_url=self.origin("a"), credential=Credential(env="A_TOKEN", scheme="query", query_param="key"))
        spec = spec_module.AdapterSpec(
            id="refusalagent", display_name="RefusalAgent", service="refusal", service_title="Refusal",
            kind="adapter", instructions="", targets=(target,), notice=None, log_level_env=None,
        )
        return tools.build_server(spec, version="0.2.0", client=self.client([target], env=env))

    def test_refusals_are_tool_errors_without_secrets(self):
        _, results, _ = run(self.build(), [
            ("refusal_get", {"target": "a", "path": f"{self.origin('out')}/x"}),
            ("refusal_get", {"target": "custom", "path": "/x"}),
            ("refusal_get", {"target": "a", "path": "/x", "query": {"key": "mine"}}),
        ])
        for is_error, text in results:
            self.assertTrue(is_error, text)
            self.assertNotIn(SECRETS["A_TOKEN"], text)
        self.assertNothingSent()

    def test_an_internal_error_is_redacted_and_says_nothing_more(self):
        server = self.build()
        with mock.patch.object(SafeHttpClient, "read", side_effect=RuntimeError(f"boom {SECRETS['A_TOKEN']}")):
            _, results, _ = run(server, [("refusal_get", {"target": "a", "path": "/x"})])
        is_error, text = results[0]
        self.assertTrue(is_error)
        self.assertNotIn(SECRETS["A_TOKEN"], text)
        self.assertNotIn("boom", text)
        self.assertIn("internal error", text)


class SunsetAndTombstone(unittest.TestCase):
    def test_sunset_keeps_its_tools_and_says_why_everywhere(self):
        spec = spec_module.load_adapter_spec(adapter_data("sunset-adapter.json"))
        listed, results, instructions = run(tools.build_server(spec, version="0.2.0"), [("oldsearch_status", {})])
        self.assertEqual(sorted(listed), ["oldsearch_get", "oldsearch_status"])
        self.assertIn("is now part of LiteratureAgent", instructions)
        self.assertIn("gets no further updates", instructions)
        for tool in listed.values():
            self.assertIn("Sunset notice", tool.description)
        status = json.loads(results[0][1])
        self.assertEqual(status["notice"]["replacement_id"], "literatureagent")
        self.assertIn("targets", status)

    def test_tombstone_has_one_status_tool_and_builds_no_client(self):
        spec = spec_module.load_adapter_spec(adapter_data("tombstone-adapter.json"))
        with mock.patch.object(tools, "SafeHttpClient", side_effect=AssertionError("a tombstone built an HTTP client")):
            listed, results, instructions = run(tools.build_server(spec, version="0.2.0"), [("retired_status", {})])
        self.assertEqual(sorted(listed), ["retired_status"])
        self.assertTrue(listed["retired_status"].annotations.read_only_hint)
        self.assertFalse(listed["retired_status"].annotations.open_world_hint)
        self.assertIn("was retired on 2026-11-21", instructions)
        self.assertIn("holds no credentials", instructions)
        status = json.loads(results[0][1])
        self.assertNotIn("targets", status)
        self.assertEqual(status["notice"]["use_instead"], "The laboratory-automation-planning skill in BAAM.")

    def test_spec_kinds_are_enforced(self):
        data = adapter_data("tombstone-adapter.json")
        data["targets"] = adapter_data("single-adapter.json")["targets"]
        with self.assertRaises(SpecError):
            spec_module.load_adapter_spec(data)
        data = adapter_data("sunset-adapter.json")
        data.pop("notice")
        with self.assertRaises(SpecError):
            spec_module.load_adapter_spec(data)
        data = adapter_data("single-adapter.json")
        data["notice"] = adapter_data("sunset-adapter.json")["notice"]
        with self.assertRaises(SpecError):
            spec_module.load_adapter_spec(data)

    def test_unknown_keys_are_refused_at_every_level(self):
        for mutate in (
            lambda d: d.update(extra=1),
            lambda d: d["targets"][0].update(credentail={"env": "X"}),
            lambda d: d["targets"][0]["credential"].update(host="evil.example"),
            lambda d: d["targets"][0].update(allow_custom=True),
        ):
            data = adapter_data("single-adapter.json")
            mutate(data)
            with self.assertRaises(SpecError):
                spec_module.load_adapter_spec(data)


if __name__ == "__main__":
    unittest.main()
