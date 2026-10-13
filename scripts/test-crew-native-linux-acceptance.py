#!/usr/bin/env python3
import importlib.util
import io
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import textwrap
import unittest
from unittest.mock import Mock, patch
import zipfile

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


class ChildFailureTests(unittest.TestCase):
    def test_failure_receipt_redacts_long_arguments_and_opaque_values(self):
        secret = "synthetic-long-credential-value-000000000000000000"
        result = Mock(returncode=1, stdout=("opaque " + "a" * 64).encode(),
                      stderr=("Permission denied " + secret).encode())
        output = io.StringIO()
        with patch.object(acceptance.subprocess, "run", return_value=result), \
                patch.object(acceptance, "RUN_DIAGNOSTICS", None), patch.object(acceptance.sys, "stderr", output):
            with self.assertRaisesRegex(RuntimeError, "safe receipt"):
                acceptance.run(["/usr/bin/broker", "start", "--bootstrap-key", secret])
        receipt = output.getvalue()
        self.assertIn("Permission denied", receipt)
        self.assertIn('"exitCode": 1', receipt)
        self.assertIn('"method": "start"', receipt)
        self.assertNotIn(secret, receipt)
        self.assertNotIn("a" * 64, receipt)

    def test_success_keeps_captured_output_without_a_failure_receipt(self):
        result = Mock(returncode=0, stdout=b"ready", stderr=b"")
        with patch.object(acceptance.subprocess, "run", return_value=result), \
                patch.object(acceptance.sys, "stderr", io.StringIO()) as output:
            self.assertIs(acceptance.run(["/broker", "start"]), result)
        self.assertEqual(output.getvalue(), "")


class ArtifactRedirectTests(unittest.TestCase):
    def redirect(self, target):
        request = acceptance.urllib.request.Request(
            "https://api.github.com/repos/example/actions/artifacts/1/zip",
            headers={"Authorization": "Bearer synthetic-test-only", "Accept": "application/vnd.github+json"})
        result = acceptance.ArtifactRedirectHandler().redirect_request(
            request, None, 302, "Found", {}, target)
        return request, result

    def test_github_bearer_is_not_forwarded_to_signed_blob(self):
        original, redirected = self.redirect("https://example.blob.core.windows.net/artifact?sig=synthetic")
        self.assertFalse(redirected.has_header("Authorization"))
        self.assertEqual(original.get_header("Authorization"), "Bearer synthetic-test-only")
        self.assertEqual(redirected.get_header("Accept"), "application/vnd.github+json")

    def test_same_origin_redirect_keeps_api_authorization(self):
        _, redirected = self.redirect("https://api.github.com:443/repos/example/actions/artifacts/2/zip")
        self.assertEqual(redirected.get_header("Authorization"), "Bearer synthetic-test-only")

    def test_different_origin_port_removes_authorization(self):
        _, redirected = self.redirect("https://api.github.com:444/artifact")
        self.assertFalse(redirected.has_header("Authorization"))

    def test_artifact_redirect_cannot_downgrade_https(self):
        with self.assertRaisesRegex(RuntimeError, "remain HTTPS"):
            self.redirect("http://example.blob.core.windows.net/artifact")


class ReleaseVersionTests(unittest.TestCase):
    def test_only_stable_semver_is_accepted(self):
        for version in ("1.93.0", "2.0.0", "0.0.1"):
            self.assertEqual(acceptance.stable_version(version), version)
        for version in ("", "v1.93.0", "1.93", "01.93.0", "1.93.00", "1.93.0-rc.1",
                        "1.93.0+build", "1x93x0", "1.93.0\n", "$(false)"):
            with self.subTest(version=version), self.assertRaises(argparse.ArgumentTypeError):
                acceptance.stable_version(version)

    def test_cli_requires_valid_version_before_creating_native_state(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary) / "fixture"
            command = [sys.executable, str(Path(__file__).with_name("crew-native-linux-acceptance.py")),
                       "--broker", str(Path(temporary) / "broker"), "--root", str(root),
                       "--report", str(Path(temporary) / "report.json"),
                       "--source-sha", "a" * 40, "--release-run-id", "123"]
            for arguments in ([], ["--version", "v1.93.0"], ["--version", "1x93x0"]):
                with self.subTest(arguments=arguments):
                    result = subprocess.run(command + arguments, capture_output=True, text=True, timeout=5)
                    self.assertEqual(result.returncode, 2)
                    self.assertIn("--version", result.stderr)
                    self.assertFalse(root.exists())

    def test_broker_must_match_the_requested_version_before_creating_accounts(self):
        for actual in ("1.93.0", "1.92.0"):
            with self.subTest(actual=actual), tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                binary = root / "broker"
                binary.write_bytes(b"synthetic broker; never executed")
                with patch.object(acceptance.os, "geteuid", return_value=0), \
                        patch.object(acceptance.os, "uname", return_value=Mock(machine="x86_64")), \
                        patch.object(acceptance, "run", return_value=Mock(stdout=("biorouter-crew " + actual).encode())), \
                        patch.object(acceptance, "create_person", side_effect=RuntimeError("accounts boundary")) as create:
                    expected = "accounts boundary" if actual == "1.93.0" else "Broker version mismatch"
                    with self.assertRaisesRegex(RuntimeError, expected):
                        acceptance.accept(binary, root / "fixture", root / "report", "a" * 40, "123", "1.93.0")
                    self.assertEqual(create.call_count, int(actual == "1.93.0"))


class WorkflowArtifactTests(unittest.TestCase):
    def download(self, kind, version="1.93.0", bad_digest=False, bad_source=False):
        workflow = Path(__file__).parent.parent / ".github/workflows/crew-native-linux-acceptance.yml"
        blocks = workflow.read_text().split("          python3 - <<'PY'\n")
        script = textwrap.dedent(blocks[1 if kind == "backend" else 2].split("          PY\n", 1)[0])
        files = {"biorouter-crew": b"synthetic broker"} if kind == "backend" else {
            f"biorouter_{version}_amd64.deb": b"GUI DEB", f"Biorouter-{version}-1.x86_64.rpm": b"GUI RPM",
            f"biorouter-cli_{version}_amd64.deb": b"CLI DEB", f"biorouter-cli-{version}-1.x86_64.rpm": b"CLI RPM"}
        archive = io.BytesIO()
        with zipfile.ZipFile(archive, "w") as output:
            for name, content in files.items():
                output.writestr(name, content)
        content = archive.getvalue()
        artifact = dict(id=42, name="linux-" + kind + "-" + version, expired=False,
                        size_in_bytes=len(content), digest="sha256:" + ("0" * 64 if bad_digest else hashlib.sha256(content).hexdigest()))
        metadata = dict(head_sha=("b" if bad_source else "a") * 40, status="completed", conclusion="success",
                        path=".github/workflows/linux-gui-packages.yml")
        responses = {"/actions/runs/123": json.dumps(metadata).encode(),
                     "/actions/runs/123/artifacts?per_page=100": json.dumps({"artifacts": [
                         {**artifact, "id": 41, "name": "linux-" + kind + "-1.92.0"}, artifact]}).encode(),
                     "/actions/artifacts/42/zip": content}
        def open_response(request, **options):
            path = request.full_url.removeprefix("https://api.github.com/repos/BaranziniLab/biorouter")
            return io.BytesIO(responses[path])
        opener = Mock(open=Mock(side_effect=open_response))
        with tempfile.TemporaryDirectory() as temporary, \
                patch.dict(os.environ, dict(VERSION=version, SOURCE_SHA="a" * 40, RELEASE_RUN="123",
                                            RUNNER_TEMP=temporary, GH_TOKEN="synthetic-test-only")), \
                patch.object(acceptance.urllib.request, "build_opener", return_value=opener):
            exec(compile(script, str(workflow), "exec"), {})
            root = Path(temporary) if kind == "backend" else Path(temporary) / "native-linux-packages"
            proof = json.loads((root / ("broker-artifact-proof.json" if kind == "backend" else "source-proof.json")).read_text())
            self.assertEqual(proof["version"], version)
            self.assertEqual(proof["sourceSha"], "a" * 40)
            self.assertEqual(proof["artifactId"], 42)
            if kind == "backend":
                self.assertEqual((root / "exact-biorouter-crew").read_bytes(), files["biorouter-crew"])
            else:
                self.assertEqual(set(proof["packages"]), set(files))
                for name, content in files.items():
                    self.assertEqual((root / name).read_bytes(), content)
                    self.assertEqual(proof["packages"][name]["sha256"], hashlib.sha256(content).hexdigest())

    def test_current_and_future_versions_select_exact_artifacts_and_four_packages(self):
        for version in ("1.93.0", "2.0.0"):
            for kind in ("backend", "packages"):
                with self.subTest(version=version, kind=kind):
                    self.download(kind, version)

    def test_version_propagation_preserves_source_and_digest_refusals(self):
        for kind in ("backend", "packages"):
            for invalid in (dict(bad_digest=True), dict(bad_source=True)):
                with self.subTest(kind=kind, invalid=invalid), self.assertRaises(AssertionError):
                    self.download(kind, **invalid)


class PackageVersionTests(unittest.TestCase):
    def smoke(self, format, actual="1.93.0", requested="1.93.0"):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            shim = root / "shim"
            shim.write_text("#!" + sys.executable + "\n" + textwrap.dedent('''\
                import hashlib,json,os,sys
                from pathlib import Path
                name=Path(sys.argv[0]).name;args=sys.argv[1:]
                with open(os.environ['FIXTURE_LOG'],'a') as log: log.write(json.dumps([name,*args])+'\\n')
                if name=='uname': print('x86_64')
                elif name=='id':
                    if len(args)>1: sys.exit(1)
                    print('0')
                elif name=='stat':
                    assert args==['-c','%u:%g:%a','/var/lib'];print('0:0:755')
                elif name=='sha256sum':
                    for row in Path('packages.sha256').read_text().splitlines():
                        digest,file=row.split('  ');assert hashlib.sha256(Path(file).read_bytes()).hexdigest()==digest
                elif name=='runuser':
                    assert args[:5]==['-u','biorouter_package_qa','--','env','-i']
                    command=args[5:]
                    while '=' in command[0]: command=command[1:]
                    if command[-1]=='--version':
                        print(Path(command[0]).name+' '+os.environ['FIXTURE_ACTUAL_VERSION'])
                else: assert name in ('useradd','chmod','install','apt-get','dnf','dpkg','rpm')
                '''))
            shim.chmod(0o755)
            bin_dir = root / "bin"
            bin_dir.mkdir()
            for name in ("uname", "id", "stat", "sha256sum", "runuser", "useradd", "chmod", "install", "apt-get", "dnf", "dpkg", "rpm"):
                (bin_dir / name).symlink_to(shim)
            packages = root / "packages"
            packages.mkdir()
            names = [f"biorouter_{requested}_amd64.deb", f"Biorouter-{requested}-1.x86_64.rpm",
                     f"biorouter-cli_{requested}_amd64.deb", f"biorouter-cli-{requested}-1.x86_64.rpm"]
            for name in names:
                (packages / name).write_bytes(name.encode())
            (packages / "packages.sha256").write_text("".join(hashlib.sha256(name.encode()).hexdigest() + "  " + name + "\n" for name in names))
            environment = {**os.environ, "PATH": str(bin_dir) + ":/usr/bin:/bin", "TMPDIR": str(root),
                           "FIXTURE_LOG": str(root / "calls.jsonl"), "FIXTURE_ACTUAL_VERSION": actual}
            script = Path(__file__).with_name("smoke-native-linux-packages.sh")
            result = subprocess.run(["/bin/bash", str(script), format, str(packages), str(root / "desktop.sh"), requested],
                                    capture_output=True, text=True, env=environment, timeout=10)
            calls = [json.loads(line) for line in (root / "calls.jsonl").read_text().splitlines()] if (root / "calls.jsonl").exists() else []
            return result, calls, str(packages)

    def test_installs_requested_deb_and_rpm_names_and_checks_each_runtime(self):
        for format in ("deb", "rpm"):
            with self.subTest(format=format):
                result, calls, directory = self.smoke(format)
                self.assertEqual(result.returncode, 0, result.stderr)
                expected = ("biorouter_1.93.0_amd64.deb", "biorouter-cli_1.93.0_amd64.deb") if format == "deb" else ("Biorouter-1.93.0-1.x86_64.rpm", "biorouter-cli-1.93.0-1.x86_64.rpm")
                installed = [argument for call in calls if call[0] in ("apt-get", "dnf") for argument in call if argument.startswith(directory + "/")]
                self.assertEqual(installed, [directory + "/" + name for name in expected])
                self.assertEqual(sum(call[-1] == "--version" for call in calls), 5)
                self.assertIn("NATIVE_LINUX_PACKAGE_ACCEPTANCE_OK " + format, result.stdout)

    def test_wrong_runtime_version_stops_before_gui_launch(self):
        result, calls, _ = self.smoke("deb", actual="1.92.0")
        self.assertNotEqual(result.returncode, 0)
        self.assertFalse(any("timeout" in call for call in calls))
        self.assertNotIn("NATIVE_LINUX_PACKAGE_ACCEPTANCE_OK", result.stdout)

    def test_invalid_version_is_refused_before_any_native_command(self):
        for version in ("v1.93.0", "1x93x0", "01.93.0", "1.93.0-rc.1"):
            with self.subTest(version=version):
                result, calls, _ = self.smoke("deb", requested=version)
                self.assertEqual(result.returncode, 2)
                self.assertIn("stable major.minor.patch", result.stderr)
                self.assertEqual(calls, [])


if __name__ == "__main__":
    unittest.main()
