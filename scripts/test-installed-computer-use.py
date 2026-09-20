#!/usr/bin/env python3
"""Negative proofs for installed-runtime acceptance; no fake package builds."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import plistlib
import shutil
import signal
import stat
import subprocess
import sys
import tempfile
import time
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('installed', Path(__file__).with_name('verify-installed-computer-use.py'))
installed = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installed)
package_spec = importlib.util.spec_from_file_location('packages', Path(__file__).with_name('computer-use-package-acceptance.py'))
packages = importlib.util.module_from_spec(package_spec)
package_spec.loader.exec_module(packages)
runtime_spec = importlib.util.spec_from_file_location('cu_runtime', Path(__file__).with_name('computer-use-runtime.py'))
cu_runtime = importlib.util.module_from_spec(runtime_spec)
runtime_spec.loader.exec_module(cu_runtime)


class InstalledDoctorTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.helper = Path(self.temp.name)
        (self.helper / 'manifest.json').write_text(json.dumps({'executable': 'ocu', 'upstream_version': '0.3.5'}), encoding='utf-8')
        # The payload file has to exist on disk. `validate_doctor` compares
        # filesystem IDENTITY (`os.path.samefile`) rather than path spelling, so
        # a manifest naming a file that was never written cannot be matched --
        # and on macOS it fails in a way that reads as a path bug, because
        # `tempfile` hands out `/var/...` while `resolve()` reports the
        # `/private/var/...` the symlink points at.
        (self.helper / 'ocu').write_bytes(b'not a real helper')
        self.report = {'computer_use': {'integrity': 'verified', 'development_override': False,
            'executable': str(self.helper / 'ocu'), 'target': 'linux-x64', 'runtime_version': '0.3.5',
            'status': 'ready', 'desktop_available': True, 'capture_available': True,
            'permissions': {'accessibility': True, 'screen_recording': True}}}

    def validate(self, report, statuses=('ready',)):
        return installed.validate_doctor(report, self.helper, 'linux-x64', statuses)

    def test_actual_readiness_contract(self):
        self.validate(self.report)
        denied = copy.deepcopy(self.report)
        denied['computer_use'].update(status='os_permission_required', permissions={'accessibility': False, 'screen_recording': True})
        self.validate(denied, ('os_permission_required',))

    def test_a_missing_payload_is_named_not_raised_as_an_oserror(self):
        # `os.path.samefile` raises OSError when EITHER side is absent, and the
        # absent side that matters is the installed one: that is the broken
        # install this check exists to catch. Asserting on ValueError is what
        # makes the difference visible -- an OSError escapes as an unhandled
        # traceback naming a single path.
        (self.helper / 'ocu').unlink()
        with self.assertRaises(ValueError) as caught:
            self.validate(self.report)
        self.assertIn('does not exist', str(caught.exception))
        self.assertIn('ocu', str(caught.exception))

    def test_a_symlinked_temp_root_is_still_the_same_payload(self):
        # macOS hands out `/var/folders/...` while `resolve()` reports the
        # `/private/var/...` it points at. Spelling differs, identity does not,
        # and a spelling comparison fails every macOS run.
        report = copy.deepcopy(self.report)
        report['computer_use']['executable'] = str(self.helper / '.' / 'ocu')
        self.validate(report)

    def test_wrong_installation_and_unverified_bytes_fail(self):
        for field, value in [('executable', '/another/install/ocu'), ('integrity', 'verified_on_start'),
                             ('development_override', True), ('target', 'linux-arm64'), ('runtime_version', '0.3.4')]:
            with self.subTest(field=field), self.assertRaises(ValueError):
                report = copy.deepcopy(self.report)
                report['computer_use'][field] = value
                self.validate(report)

    def test_missing_dependency_is_not_accepted_as_headless(self):
        self.report['computer_use']['status'] = 'missing_dependency'
        with self.assertRaises(ValueError):
            self.validate(self.report, ('desktop_unavailable',))

    def test_no_desktop_cannot_be_reported_as_ready(self):
        self.report['computer_use']['desktop_available'] = False
        with self.assertRaises(ValueError):
            self.validate(self.report)
        self.report['computer_use']['status'] = 'desktop_unavailable'
        self.validate(self.report, ('desktop_unavailable',))

    def test_unknown_permission_string_fails_cleanly(self):
        self.report['computer_use']['permissions'] = 'unknown'
        with self.assertRaises(ValueError):
            self.validate(self.report)

    def test_ready_with_denied_capture_fails(self):
        self.report['computer_use']['permissions']['screen_recording'] = False
        with self.assertRaises(ValueError):
            self.validate(self.report)


@unittest.skipUnless(sys.platform == 'darwin', 'Requires native macOS code signing')
class MacCandidateSigningTests(unittest.TestCase):
    def test_unsigned_nested_framework_is_signed_without_changing_runtime(self):
        with tempfile.TemporaryDirectory(prefix='package signing ü ') as directory:
            app = Path(directory) / 'Candidate.app'
            framework = app / 'Contents/Frameworks/Fixture.framework'
            framework_version = framework / 'Versions/A'
            helper = app / 'Contents/Resources/computer-use'
            runtime_app = helper / 'BioRouter Computer Use.app'
            def executable_bundle(bundle, name):
                contents = bundle / 'Contents'
                (contents / 'MacOS').mkdir(parents=True)
                (contents / 'Info.plist').write_bytes(plistlib.dumps({
                    'CFBundleExecutable': name, 'CFBundleIdentifier': 'org.biorouter.fixture.' + name,
                    'CFBundlePackageType': 'APPL', 'CFBundleVersion': '1'}))
                subprocess.run(['cc', '-x', 'c', '-', '-o', contents / 'MacOS' / name],
                    input='int main(void) { return 0; }', text=True, check=True, capture_output=True)
            executable_bundle(app, 'candidate')
            executable_bundle(runtime_app, 'ocu')
            subprocess.run(['codesign', '--force', '--sign', '-', runtime_app], check=True, capture_output=True)
            (helper / 'manifest.json').write_text('{"fixture": "preserve runtime bytes"}', encoding='utf-8')
            runtime_before = {p.relative_to(helper): p.read_bytes() for p in helper.rglob('*') if p.is_file()}
            (framework_version / 'Resources').mkdir(parents=True)
            (framework_version / 'Resources/Info.plist').write_bytes(plistlib.dumps({
                'CFBundleExecutable': 'Fixture', 'CFBundleIdentifier': 'org.biorouter.fixture.framework',
                'CFBundlePackageType': 'FMWK', 'CFBundleVersion': '1'}))
            subprocess.run(['cc', '-dynamiclib', '-x', 'c', '-', '-o', framework_version / 'Fixture'],
                input='int fixture(void) { return 1; }', text=True, check=True, capture_output=True)
            (framework / 'Versions/Current').symlink_to('A')
            (framework / 'Fixture').symlink_to('Versions/Current/Fixture')
            (framework / 'Resources').symlink_to('Versions/Current/Resources')
            subprocess.run(['codesign', '--remove-signature', framework], check=True, capture_output=True)
            old_sign = subprocess.run(['codesign', '--force', '--sign', '-', app], capture_output=True, text=True)
            self.assertNotEqual(old_sign.returncode, 0)
            self.assertIn('code object is not signed at all', old_sign.stderr)
            packages.sign_macos_candidate(app)
            runtime_after = {p.relative_to(helper): p.read_bytes() for p in helper.rglob('*') if p.is_file()}
            self.assertEqual(runtime_after, runtime_before)


@unittest.skipIf(os.name == 'nt', 'Uses a POSIX shell to stage the descendant')
class InstalledDoctorInvocationTests(unittest.TestCase):
    """The doctor call must observe PROCESS EXIT, not pipe EOF.

    `subprocess.run(capture_output=True)` returns only when every writer closes
    the pipe. On Windows a helper spawned by the CLI inherits that handle, so a
    doctor run that finished correctly is indistinguishable from one that hung.
    """

    def fake_cli(self, body):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        cli = Path(directory.name) / 'biorouter'
        cli.write_text('#!/bin/sh\n' + body, encoding='utf-8')
        cli.chmod(0o755)
        return cli, Path(directory.name)

    def test_a_descendant_holding_stdout_does_not_look_like_a_hang(self):
        # Writes its complete answer, leaves a descendant holding the inherited
        # stdout well past the deadline, then exits successfully.
        cli, scratch = self.fake_cli(
            'echo \'{"ok": true}\'\n'
            'sleep 90 &\n'
            'exit 0\n')
        started = time.monotonic()
        document, timings = installed.run_doctor(cli, dict(os.environ), scratch)
        elapsed = time.monotonic() - started
        self.assertEqual(document, {'ok': True})
        self.assertEqual([t['attempt'] for t in timings], ['cold', 'warm'])
        self.assertLess(elapsed, installed.DOCTOR_TIMEOUT,
                        'a descendant holding the pipe must not be read as a hang')

    def test_a_doctor_that_never_finishes_fails_with_what_it_had_written(self):
        cli, scratch = self.fake_cli('sleep 90\n')
        # Patch the PER-ATTEMPT budgets: the loop reads these, not the total.
        with patch.object(installed, 'ATTEMPT_TIMEOUTS', {'cold': 1, 'warm': 1}):
            with self.assertRaises(ValueError) as caught:
                installed.run_doctor(cli, dict(os.environ), scratch)
        message = str(caught.exception)
        self.assertIn('(cold)', message)
        self.assertIn('exceeded its 1s budget', message)
        self.assertIn('no output, so the work had not finished', message)

    def test_a_slow_cold_run_cannot_squeeze_the_warm_one(self):
        # A single shared deadline let a slow cold attempt starve the warm one, so
        # the reported failure named the WRONG attempt. Each gets its own budget.
        self.assertEqual(installed.DOCTOR_TIMEOUT, sum(installed.ATTEMPT_TIMEOUTS.values()))
        self.assertGreaterEqual(installed.ATTEMPT_TIMEOUTS['warm'], 20)

    def test_a_truncated_document_is_not_reported_as_a_finished_run(self):
        # Rust's stdout is line-buffered, so a doctor that hangs partway through
        # has already flushed kilobytes. Judging completeness by byte count
        # reported a truncated document as a finished run.
        self.assertIn('TRUNCATED', installed.describe_output(b'{"computer_use": {"sta'))
        self.assertIn('COMPLETE', installed.describe_output(b'{"computer_use": {}}'))
        self.assertIn('no output', installed.describe_output(b''))

    def test_a_descendant_surviving_the_doctor_is_recorded_not_ignored(self):
        # Observing exit independently of the pipes stops a leaked descendant
        # failing the job, so it has to be RECORDED or the fix would simply make
        # a real leak invisible instead of loud.
        cli, scratch = self.fake_cli(
            'echo \'{"ok": true}\'\n'
            'sleep 30 &\n'
            'exit 0\n')
        _, timings = installed.run_doctor(cli, dict(os.environ), scratch)
        self.assertTrue(
            any(t['processes_left_behind'] for t in timings),
            'a process that outlived the CLI must appear in the receipt')

    def test_a_nonzero_doctor_is_reported_with_its_stderr(self):
        cli, scratch = self.fake_cli('echo boom >&2\nexit 4\n')
        with self.assertRaisesRegex(ValueError, 'exited 4'):
            installed.run_doctor(cli, dict(os.environ), scratch)


class OwnedCleanupTests(unittest.TestCase):
    def test_agent_cleanup_preserves_existing_pid_and_runs_on_failure(self):
        state = {11: 'existing-start'}
        def kill(pid, sig):
            self.assertEqual((pid, sig), (22, signal.SIGTERM))
            del state[pid]
        with patch.object(installed, 'helper_processes', side_effect=lambda _: dict(state)), \
                patch.object(installed.os, 'kill', side_effect=kill) as killed:
            with self.assertRaisesRegex(ValueError, 'probe failed'):
                with installed.owned_app_agents(Path('/only/this/ocu')):
                    state[22] = 'new-start'
                    raise ValueError('probe failed')
            killed.assert_called_once_with(22, signal.SIGTERM)
        self.assertEqual(state, {11: 'existing-start'})

    def test_reused_pid_is_not_signalled(self):
        snapshots = [{11: 'existing'}, {11: 'existing', 22: 'new'}, {11: 'existing', 22: 'reused'}]
        with patch.object(installed, 'helper_processes', side_effect=snapshots), patch.object(installed.os, 'kill') as killed:
            with installed.owned_app_agents(Path('/only/this/ocu')):
                pass
            killed.assert_not_called()

    def test_process_snapshot_matches_exact_executable_not_prefix(self):
        executable = Path('/exact/ocu')
        result = subprocess.CompletedProcess([], 0, stdout=f' 11 Fri Sep 18 10:00:00 2026 {executable}\n 22 Fri Sep 18 10:00:00 2026 {executable}-other\n')
        with patch.object(installed.subprocess, 'run', return_value=result):
            self.assertEqual(installed.helper_processes(executable), {11: 'Fri Sep 18 10:00:00 2026'})

    def test_timed_out_container_removes_only_created_id(self):
        container = 'a' * 64
        calls = []
        def execute(command, **kwargs):
            calls.append(command)
            if command[1] == 'start':
                raise subprocess.TimeoutExpired(command, 600)
        with patch.object(packages.subprocess, 'check_output', return_value=container + '\n'), \
                patch.object(packages, 'run', side_effect=execute):
            with self.assertRaises(subprocess.TimeoutExpired):
                packages.owned_container(['test-image', 'true'])
        self.assertEqual(calls, [['docker', 'start', '--attach', container], ['docker', 'rm', '--force', container]])

    @unittest.skipIf(os.name == 'nt', 'Bash runtime absence gate runs only in Linux installers')
    def test_runtime_toolchain_presence_fails_even_with_errexit(self):
        with tempfile.TemporaryDirectory() as empty:
            env = dict(os.environ, PATH=empty)
            shell = '/bin/bash'
            command = [shell, '-euc', packages.runtime_only_path_guard()]
            self.assertEqual(subprocess.run(command, env=env, capture_output=True).returncode, 0)
            for name in ['node', 'npm']:
                executable = Path(empty) / name
                executable.write_text('#!/bin/sh\nexit 0\n', encoding='utf-8')
                executable.chmod(0o755)
                self.assertNotEqual(subprocess.run(command, env=env, capture_output=True).returncode, 0)
                executable.unlink()


class RemoveTreeTests(unittest.TestCase):
    """`remove_tree` must delete a tree containing READ-ONLY files.

    ⚠ This is a Windows-only defect with a cross-platform test, on purpose. Git
    marks pack files read-only, and Windows refuses to unlink a read-only file,
    so a bare `shutil.rmtree` cannot remove a git clone there:

        PermissionError: [WinError 5] Access is denied:
          '...source.noindex/.git/objects/pack/pack-....idx'

    On Linux and macOS deletion is governed by the DIRECTORY's write bit, so the
    same tree removes fine and the bug is invisible -- which is exactly why it
    survived: `computer-use-runtime.py build` worked once on a Windows machine
    and then failed on every later run, cleaning up the previous one.

    The read-only bit is set explicitly here rather than by cloning a
    repository, so the test needs no network and asserts the property directly.
    """

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)

    def _readonly_tree(self):
        root = Path(self.temp.name) / 'checkout'
        (root / '.git' / 'objects' / 'pack').mkdir(parents=True)
        pack = root / '.git' / 'objects' / 'pack' / 'pack-deadbeef.idx'
        pack.write_bytes(b'not really a pack')
        ordinary = root / 'README.md'
        ordinary.write_text('ordinary', encoding='utf-8')
        os.chmod(pack, stat.S_IREAD)
        return root, pack

    def test_removes_a_tree_with_read_only_files(self):
        root, pack = self._readonly_tree()
        self.assertFalse(os.access(pack, os.W_OK), 'precondition: the pack file must be read-only')
        cu_runtime.remove_tree(root)
        self.assertFalse(root.exists(), 'remove_tree must delete a checkout containing read-only files')

    def test_a_bare_rmtree_is_what_fails_on_windows(self):
        """The control. Without this the test above could pass for the wrong reason.

        On Windows a bare `shutil.rmtree` raises; everywhere else it succeeds.
        Asserting the platform-specific outcome keeps the test honest instead of
        skipping, and it fails loudly if a future Python changes the behaviour.
        """
        root, _ = self._readonly_tree()
        if os.name == 'nt':
            with self.assertRaises(PermissionError):
                shutil.rmtree(root)
            cu_runtime.remove_tree(root)
        else:
            shutil.rmtree(root)
        self.assertFalse(root.exists())

    def test_a_genuine_failure_is_still_raised(self):
        """Clearing the read-only bit must not become "swallow every error"."""
        with self.assertRaises(FileNotFoundError):
            cu_runtime.remove_tree(Path(self.temp.name) / 'was-never-there')


class NonAsciiInstallPathTests(unittest.TestCase):
    """The acceptance harness installs under a directory containing a non-ASCII
    character ON PURPOSE, and this is the class of bug that exists to catch.

    ⚠ The bug it caught was in the HARNESS, not the product. `Path.read_text`
    with no encoding uses `locale.getpreferredencoding()`, which on Windows is
    the ANSI code page (cp1252), not UTF-8. The doctor JSON is written by a Rust
    process as UTF-8 and contains the install path, so reading it back through
    cp1252 turned the directory's `ü` (U+00FC) into `Ã¼` (U+00C3 U+00BC) -- its
    own UTF-8 bytes reinterpreted one byte at a time. The reported executable
    could then never equal the expected one, and the job failed with "Doctor
    resolved outside the installed payload", blaming the product.

    Measured on Windows Server 2025 before the fix:
        expected segment codepoints: [... 0xFC ...]
        reported segment codepoints: [... 0xC3, 0xBC ...]

    JSON is UTF-8 by RFC 8259, so the omission was wrong on every platform and
    merely invisible where the locale encoding is already UTF-8 -- which is why
    it survived CI on Linux and macOS.
    """

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='BioRouter installed ü ')
        self.addCleanup(self.temp.cleanup)
        self.helper = Path(self.temp.name)
        (self.helper / 'manifest.json').write_text(
            json.dumps({'executable': 'ocu', 'upstream_version': '0.3.5'}), encoding='utf-8')
        (self.helper / 'ocu').write_bytes(b'not a real helper')

    def test_the_fixture_really_is_non_ascii(self):
        """Never let this suite pass because the premise quietly disappeared."""
        self.assertIn('ü', str(self.helper),
                      'the temp directory must contain a non-ASCII character for these tests to mean anything')

    def test_a_utf8_doctor_document_round_trips(self):
        """The real shape: Rust writes UTF-8, the harness reads it back."""
        document = {'computer_use': {
            'integrity': 'verified', 'development_override': False,
            'executable': str(self.helper / 'ocu'), 'target': 'linux-x64',
            'runtime_version': '0.3.5', 'status': 'ready',
            'desktop_available': True, 'capture_available': True,
            'permissions': {'accessibility': True, 'screen_recording': True}}}
        written = self.helper / 'doctor.json'
        # ⚠ ensure_ascii=False is load-bearing. Python's default escapes
        # non-ASCII to \uXXXX, which is pure ASCII and decodes identically
        # under every codec -- a fixture that cannot reproduce the bug. Rust's
        # serde_json emits RAW UTF-8, verified against the real doctor output:
        # doctor-cold.json contains the bytes C3 BC and no escapes.
        written.write_bytes(json.dumps(document, ensure_ascii=False).encode('utf-8'))

        loaded = json.loads(written.read_text(encoding='utf-8'))
        report = installed.validate_doctor(loaded, self.helper, 'linux-x64', ('ready',))
        self.assertEqual(report['status'], 'ready')

    def test_reading_utf8_as_the_locale_encoding_is_what_corrupted_it(self):
        """The control, so the test above cannot pass for the wrong reason.

        Decoding the same bytes through cp1252 must produce the mojibake, and
        `validate_doctor` must reject it. On a UTF-8 locale this is a no-op,
        which is exactly why the bug never showed outside Windows.
        """
        raw = json.dumps({'path': str(self.helper / 'ocu')}, ensure_ascii=False).encode('utf-8')
        through_cp1252 = json.loads(raw.decode('cp1252'))['path']
        through_utf8 = json.loads(raw.decode('utf-8'))['path']

        self.assertEqual(through_utf8, str(self.helper / 'ocu'))
        self.assertNotEqual(through_cp1252, through_utf8,
                            'cp1252 must mangle a UTF-8 non-ASCII path; if it does not, this fixture is not exercising the bug')
        self.assertIn('Ã', through_cp1252, 'the corruption signature is the UTF-8 lead byte surfacing as U+00C3')
        self.assertFalse(Path(through_cp1252).exists(), 'the mangled path must not resolve to a real file')

    def test_every_text_io_in_the_acceptance_scripts_names_its_encoding(self):
        """A source census. The behavioural tests above cover the one call that
        broke; this covers the other twelve that had the same latent defect, and
        any new one.
        """
        offenders = []
        reads, writes = 'read_text' + '(', 'write_text' + '('
        for name in ('verify-installed-computer-use.py', 'computer-use-package-acceptance.py',
                     'computer-use-runtime.py', 'test-installed-computer-use.py'):
            source = Path(__file__).with_name(name)
            text = source.read_text(encoding='utf-8')
            # Join continuation lines so a call whose `encoding=` sits on the
            # next physical line is not reported.
            flattened = text.replace(",\n", ", ").replace("(\n", "(")
            for number, line in enumerate(flattened.splitlines(), start=1):
                # ⚠ Build the needles rather than writing them literally. A
                # census whose own detection line matches its own pattern
                # reports itself forever, and the obvious "skip this file" fix
                # would blind it to this file's real call sites. The console
                # census in `no_console_window_census.rs` learned the same
                # lesson from the other direction, where a covering name
                # matched its own definition and the gate passed with the fix
                # deleted.
                if reads in line or writes in line:
                    if 'encoding=' not in line:
                        offenders.append(f'{name}:{number}: {line.strip()}')
        self.assertEqual(offenders, [], "text I/O without an explicit encoding:\n" + "\n".join(offenders))


if __name__ == '__main__':
    unittest.main()
