#!/usr/bin/env python3
"""Negative proofs for installed-runtime acceptance; no fake package builds."""
import copy
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('installed', Path(__file__).with_name('verify-installed-computer-use.py'))
installed = importlib.util.module_from_spec(spec)
spec.loader.exec_module(installed)
package_spec = importlib.util.spec_from_file_location('packages', Path(__file__).with_name('computer-use-package-acceptance.py'))
packages = importlib.util.module_from_spec(package_spec)
package_spec.loader.exec_module(packages)


class InstalledDoctorTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.helper = Path(self.temp.name)
        (self.helper / 'manifest.json').write_text(json.dumps({'executable': 'ocu', 'upstream_version': '0.3.5'}))
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
                executable.write_text('#!/bin/sh\nexit 0\n')
                executable.chmod(0o755)
                self.assertNotEqual(subprocess.run(command, env=env, capture_output=True).returncode, 0)
                executable.unlink()


if __name__ == '__main__':
    unittest.main()
