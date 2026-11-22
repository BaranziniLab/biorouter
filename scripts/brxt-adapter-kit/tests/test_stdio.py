"""Rendered adapters run as real stdio subprocesses, the way Biorouter starts them.

The sample adapter is rendered by bin/new_adapter.py with its targets pointed at
loopback echo servers, started as a child process that reads its credentials from
its environment, and driven over MCP. The child logs at DEBUG to a file, and the
file is searched for every credential afterwards. The sunset and tombstone
releases are rendered and started the same way.
"""

import shutil
import sys
import tempfile
import unittest
from pathlib import Path

from support import load_source
import new_adapter
from brxt_kit_testing import leak_suite, stdio_probe
from brxt_kit_testing.echo_server import EchoFleet


def render(source, out: Path):
    spec, files = new_adapter.plan(source)
    new_adapter.write(out, files, force=False)
    return spec


def child_env(out: Path, extra=None):
    return stdio_probe.clean_env({"PYTHONPATH": str(out / "src"), **(extra or {})})


class SampleAdapterOverStdio(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = Path(tempfile.mkdtemp(prefix="brxt-stdio-"))
        source = load_source("sample-adapter.json")
        adapter = dict(source["adapter"], id=source["manifest"]["name"], display_name=source["manifest"]["display_name"])
        cls.fleet = EchoFleet(cls.tmp / "ca", leak_suite.echo_server_names(adapter), hosts=leak_suite.echo_hosts(adapter))
        source["adapter"] = leak_suite.rewrite_spec_for_echo(adapter, cls.fleet)
        cls.out = cls.tmp / "sampleagent"
        cls.spec = render(source, cls.out)
        cls.values = leak_suite.make_secrets(cls.spec)
        cls.env = child_env(cls.out, {**cls.values, "SSL_CERT_FILE": str(cls.fleet.ca.ca_file), "SAMPLE_LOG_LEVEL": "DEBUG"})
        cls.command = [sys.executable, "-m", "sampleagent"]

    @classmethod
    def tearDownClass(cls):
        cls.fleet.stop()
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def secret_forms(self):
        forms = set()
        for target in self.spec.targets:
            if target.credential:
                user = self.values.get(target.credential.username_env) if target.credential.username_env else None
                forms |= leak_suite.secret_forms(self.values[target.credential.env], target.credential.scheme, user)
        return forms

    def test_start_probe_lists_the_declared_tools(self):
        log = self.tmp / "probe.log"
        found = stdio_probe.probe(self.command, env=self.env, cwd=str(self.out), log_file=log)
        self.assertEqual(found["tools"], ["sample_get", "sample_status", "sample_write"])
        self.assertEqual(found["server"], "SampleAgent")
        self.assertEqual(found["version"], "0.2.0")
        self.assertIn("Start with sample_status", found["instructions"])
        self.assertTrue(found["annotations"]["sample_get"]["readOnlyHint"])
        self.assertTrue(found["annotations"]["sample_write"]["destructiveHint"])
        for form in self.secret_forms():
            self.assertNotIn(form, found["status_text"])

    def test_leak_battery_over_stdio_and_a_clean_log(self):
        log = self.tmp / "battery.log"
        report = leak_suite.run_over_stdio(
            leak_suite.rewrite_spec_for_echo(
                dict(load_source("sample-adapter.json")["adapter"], id="sampleagent", display_name="SampleAgent"), self.fleet
            ),
            self.fleet,
            self.command[0],
            self.command[1:],
            kit_package="brxt_kit",
            env=self.env,
            values=self.values,
            cwd=str(self.out),
            log_file=log,
        )
        report.assert_clean()
        self.assertGreater(report.checks_run(), 2000)
        text = log.read_text(encoding="utf-8", errors="replace")
        self.assertTrue(text, "the child wrote no log at DEBUG level, so the log check proved nothing")
        for form in self.secret_forms():
            self.assertNotIn(form, text)


    def test_the_log_check_can_fail(self):
        """With the kit's log silencing removed, a query credential does reach the log."""

        import asyncio

        mutant = self.tmp / "mutant"
        shutil.copytree(self.out, mutant)
        vendored = mutant / "src" / "sampleagent" / "_kit" / "safe_http.py"
        text = vendored.read_text(encoding="utf-8")
        silencing = 'for _logger_name in ("httpx", "httpcore"):\n    logging.getLogger(_logger_name).setLevel(logging.WARNING)\n'
        self.assertIn(silencing, text)
        vendored.write_text(text.replace(silencing, ""), encoding="utf-8")
        log = self.tmp / "mutant.log"
        env = dict(self.env, PYTHONPATH=str(mutant / "src"))

        async def one_call():
            async with leak_suite.stdio_session(self.command[0], self.command[1:], env=env, cwd=str(mutant), log_file=log) as session:
                return await session.call("sample_get", {"target": "delta", "path": "/search"})

        call = asyncio.run(one_call())
        self.assertFalse(call.is_error, call.text)
        self.assertIn(self.values["DELTA_KEY"], log.read_text(encoding="utf-8", errors="replace"))


class SunsetAndTombstoneStart(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="brxt-stdio-"))

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def test_sunset_starts_and_says_what_replaces_it(self):
        out = self.tmp / "oldsearchagent"
        render(load_source("sunset-adapter.json"), out)
        found = stdio_probe.probe([sys.executable, "-m", "oldsearchagent"], env=child_env(out), cwd=str(out))
        self.assertEqual(found["tools"], ["oldsearch_get", "oldsearch_status"])
        self.assertIn("is now part of LiteratureAgent", found["instructions"])
        self.assertIn("literatureagent", found["status_text"])
        skill = (out / "skills" / "old-search-evidence" / "SKILL.md").read_text(encoding="utf-8")
        self.assertIn("has moved", skill)
        self.assertIn("literatureagent", skill)

    def test_tombstone_starts_with_one_tool_and_reads_no_credential(self):
        out = self.tmp / "retiredagent"
        render(load_source("tombstone-adapter.json"), out)
        planted = {"RETIRED_API_KEY": "planted-retired-key-0123456789", "ALPHA_TOKEN": "planted-alpha-0123456789"}
        log = self.tmp / "tombstone.log"
        found = stdio_probe.probe([sys.executable, "-m", "retiredagent"], env=child_env(out, planted), cwd=str(out), log_file=log)
        self.assertEqual(found["tools"], ["retired_status"])
        self.assertIn("was retired on 2026-11-21", found["instructions"])
        self.assertIn("laboratory-automation-planning", found["status_text"])
        for value in planted.values():
            self.assertNotIn(value, found["status_text"])
            self.assertNotIn(value, log.read_text(encoding="utf-8", errors="replace"))
        for name in ("retired-lab-runs", "retired-protocols"):
            self.assertIn("has moved", (out / "skills" / name / "SKILL.md").read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
