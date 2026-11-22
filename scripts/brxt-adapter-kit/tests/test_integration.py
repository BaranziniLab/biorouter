"""The real path, end to end. Opt in: it runs uv against PyPI and takes a minute or two.

    BRXT_KIT_INTEGRATION=1 uv run python -m unittest discover -s tests -p test_integration.py -v

With BRXT_KIT_BIOROUTER set to a biorouter binary it also installs the bundle with
`biorouter extension install` into a throwaway BIOROUTER_PATH_ROOT (keyring off)
and starts the installed copy with the exact command Biorouter registered.

Steps: render the sample adapter, `uv lock`, `uv lock --check`, `uv sync --locked`,
run the adapter's own contract tests (its CI step), build the bundle, start the
bundle the way Biorouter does (`uv run --directory <dir> <entry_point>`) from a
fresh extraction, and run the stdio leak battery against that process.
"""

import json
import os
import shutil
import subprocess
import tempfile
import unittest
import zipfile
from pathlib import Path

from support import load_source
import new_adapter
from brxt_kit_testing import leak_suite, stdio_probe
from brxt_kit_testing.echo_server import EchoFleet

ENABLED = os.environ.get("BRXT_KIT_INTEGRATION") == "1"
BIOROUTER = os.environ.get("BRXT_KIT_BIOROUTER")


def run(command, cwd, env=None, timeout=900, stdin=None):
    result = subprocess.run(command, cwd=cwd, env=env, input=stdin, capture_output=True, text=True, timeout=timeout)
    if result.returncode != 0:
        raise AssertionError(f"{' '.join(map(str, command))} failed ({result.returncode}):\n{result.stdout[-3000:]}\n{result.stderr[-3000:]}")
    return result


def uv_env(extra=None):
    env = stdio_probe.clean_env(extra)
    env.pop("VIRTUAL_ENV", None)
    return env


@unittest.skipUnless(ENABLED, "set BRXT_KIT_INTEGRATION=1 to run uv against PyPI")
class RealUvPath(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.tmp = Path(tempfile.mkdtemp(prefix="brxt-integration-"))
        source = load_source("sample-adapter.json")
        adapter = dict(source["adapter"], id="sampleagent", display_name="SampleAgent")
        cls.fleet = EchoFleet(cls.tmp / "ca", leak_suite.echo_server_names(adapter), hosts=leak_suite.echo_hosts(adapter))
        cls.spec_data = leak_suite.rewrite_spec_for_echo(adapter, cls.fleet)
        source["adapter"] = cls.spec_data
        cls.repo = cls.tmp / "SampleAgent"
        cls.spec, files = new_adapter.plan(source)
        new_adapter.write(cls.repo, files, force=False)
        run(["uv", "lock"], cls.repo, uv_env())
        run(["uv", "lock", "--check"], cls.repo, uv_env())
        run(["uv", "sync", "--locked", "--python", "3.13"], cls.repo, uv_env())
        cls.values = leak_suite.make_secrets(cls.spec)

    @classmethod
    def tearDownClass(cls):
        cls.fleet.stop()
        shutil.rmtree(cls.tmp, ignore_errors=True)

    def test_1_the_adapters_own_ci_steps_pass(self):
        tests = run(["uv", "run", "python", "-m", "unittest", "discover", "-s", "tests", "-v"], self.repo, uv_env())
        self.assertIn("OK", tests.stderr)
        build = run(["uv", "run", "python", "scripts/build_brxt.py"], self.repo, uv_env())
        self.assertIn("sampleagent.brxt", build.stdout)
        sums = (self.repo / "dist" / "SHA256SUMS").read_text(encoding="utf-8").split()
        self.assertEqual(sums[1], "sampleagent.brxt")

    def bundle(self):
        path = self.repo / "dist" / "sampleagent.brxt"
        if not path.is_file():
            run(["uv", "run", "python", "scripts/build_brxt.py"], self.repo, uv_env())
        return path

    def battery(self, install_dir: Path, label: str):
        log = self.tmp / f"{label}.log"
        env = uv_env({**self.values, "SSL_CERT_FILE": str(self.fleet.ca.ca_file), "SAMPLE_LOG_LEVEL": "DEBUG"})
        found = stdio_probe.probe(["uv", "run", "--directory", str(install_dir), "sampleagent"], env=env, log_file=log)
        self.assertEqual(found["tools"], ["sample_get", "sample_status", "sample_write"])
        report = leak_suite.run_over_stdio(
            self.spec_data, self.fleet, "uv", ["run", "--directory", str(install_dir), "sampleagent"],
            kit_package="brxt_kit", env=env, values=self.values, log_file=log,
        )
        report.assert_clean()
        text = log.read_text(encoding="utf-8", errors="replace")
        for value in self.values.values():
            if value.startswith("leak-") and "@" not in value:
                self.assertNotIn(value, text)
        return found

    def test_2_a_fresh_extraction_starts_the_way_biorouter_starts_it(self):
        extracted = self.tmp / "extracted"
        with zipfile.ZipFile(self.bundle()) as archive:
            archive.extractall(extracted)
        run(["uv", "sync"], extracted, uv_env())  # what Biorouter's installer runs
        self.battery(extracted, "extracted")

    @unittest.skipUnless(BIOROUTER, "set BRXT_KIT_BIOROUTER to a biorouter binary")
    def test_3_biorouter_installs_and_registers_it(self):
        root = Path(os.environ.get("BRXT_KIT_BIOROUTER_ROOT") or self.tmp / "biorouter-root")
        root.mkdir(parents=True, exist_ok=True)
        env = uv_env({"BIOROUTER_PATH_ROOT": str(root), "BIOROUTER_DISABLE_KEYRING": "true", "NO_COLOR": "1"})
        secrets = "".join(f"{k}={v}\n" for k, v in self.values.items() if k in {"ALPHA_TOKEN", "BETA_KEY", "GAMMA_KEY", "DELTA_KEY", "EPSILON_PASSWORD"})
        run(
            [BIOROUTER, "extension", "install", str(self.bundle()), "--secret-stdin",
             "--env", "BETA_TENANT=mylab", "--env", "EPSILON_USER=user-epsilon", "--env", "SAMPLE_CONTACT_EMAIL=leak-contact@example.org"],
            self.tmp, env, stdin=secrets,
        )
        listed = json.loads(run([BIOROUTER, "extension", "list", "--format", "json"], self.tmp, env).stdout)
        entries = listed if isinstance(listed, list) else listed.get("extensions", listed)
        text = json.dumps(entries)
        self.assertIn("sampleagent", text)
        for value in self.values.values():
            if value.startswith("leak-") and "@" not in value:
                self.assertNotIn(value, text, "a secret value appeared in the extension listing")
        config_text = "".join(p.read_text(encoding="utf-8", errors="replace") for p in root.rglob("config.yaml"))
        self.assertIn("sampleagent", config_text)
        for value in self.values.values():
            if value.startswith("leak-") and "@" not in value:
                self.assertNotIn(value, config_text, "a secret value was written to config.yaml")
        install_dirs = [p.parent for p in root.rglob("manifest.json") if p.parent.name == "sampleagent"]
        self.assertTrue(install_dirs, "no installed copy found under the isolated root")
        found = self.battery(install_dirs[0], "installed")
        self.assertEqual(found["server"], "SampleAgent")


if __name__ == "__main__":
    unittest.main()
