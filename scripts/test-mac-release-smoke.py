#!/usr/bin/env python3
"""Exercise the Mac smoke shell with fixture commands, never native artifacts."""
import os
from pathlib import Path
import shlex
import subprocess
import sys
import tempfile
import unittest


SOURCE = Path(__file__).with_name('smoke-test-release-artifacts.sh').read_text()
# From the microphone check (its sentence and `mac_microphone_problems`, which
# `smoke_mac` calls) to the end of `smoke_mac`.
MAC_SMOKE = SOURCE[SOURCE.index('MAC_MICROPHONE_USAGE='):SOURCE.index('LINUX_DESKTOP_SMOKE=')]

STUBS = r'''
trace() { printf '%s\n' "$*" >>"$CASE_ROOT/trace"; }
log() { printf '%s\n' "$*"; }
die() { printf '%s\n' "$*" >&2; exit 1; }
require_file() { [ -f "$1" ] || die "missing artifact: $1"; }
mktemp() {
  local dir
  dir="$(command mktemp -d "$CASE_ROOT/$(basename "$2")")"
  trace "temp $dir"
  printf '%s\n' "$dir"
}
security() { trace "UNEXPECTED security $*"; return 91; }
hdiutil() {
  trace "hdiutil $*"
  if [ "$1" = attach ]; then
    local mount="$5"
    mkdir -p "$mount/Biorouter.app/Contents/Resources/bin" "$mount/Biorouter.app/Contents/MacOS"
    touch "$mount/Biorouter.app/Contents/Resources/bin/biorouter" \
      "$mount/Biorouter.app/Contents/Resources/bin/biorouterd" "$mount/.fixture-attached"
  elif [ "$MODE" = detach-failure ]; then
    return 37
  elif [ "$MODE" = detach-retry ] && [ "$2" != -force ]; then
    return 37
  else
    local mount="${@: -1}"
    rm -f "$mount/.fixture-attached"
  fi
}
codesign() {
  trace "codesign $*"
  if [ "$1" = -d ]; then
    # The signed entitlements, as `codesign -d --entitlements - --xml` prints them.
    # Mode `entitlement` drops audio input from the helper alone, so the check
    # has to find and name one file of the four.
    printf '<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict>'
    [ "$MODE" = entitlement ] && [[ "${@: -1}" == *'Biorouter Helper.app' ]] \
      || printf '<key>com.apple.security.device.audio-input</key><true/>'
    printf '</dict></plist>\n'
    return 0
  fi
  [ "$MODE" != codesign ] || return 37
}
plutil() {
  trace "plutil $*"
  if [ "$MODE" = usage ]; then
    printf 'This app needs access to the microphone\n'
  else
    printf '%s\n' "$MAC_MICROPHONE_USAGE"
  fi
}
spctl() { trace "spctl $*"; [ "$MODE" != spctl ] || return 37; }
xcrun() { trace "xcrun $*"; [ "$MODE" != stapler ] || return 37; }
arch() {
  trace "arch $*"
  [ "$MODE" != rosetta ] || return 37
  [[ "$*" != *--version* ]] || /usr/bin/env "$@"
}
file() { trace "file $*"; [ "$MODE" != architecture ] && printf 'arm64 x86_64\n'; }
# The dictation guards (PR-11) live outside the slice above: the size ceiling and the
# model scan are traced, and the self test is answered for the fixture's absent repo.
check_dictation_size() { trace "size $*"; }
scan_ggml() { trace "scan $*"; }
sw_vers() { printf '15.0\n'; }
bash() {
  if [ "$1" = "$ROOT/scripts/dictation-self-test.sh" ]; then
    trace "dictation self test ${*:2}"
    return 0
  fi
  command bash "$@"
}
python3() {
  if [ "$1" = "$ROOT/scripts/computer-use-runtime.py" ]; then
    trace "helper $*"
    [ "$MODE" != helper ] || return 37
  elif [ "$1" = - ]; then
    trace 'validate private runtime key'
    local code
    code="$(cat)"
    printf '%s\n' 'import os; os.kill = lambda pid, signal: None' "$code" | "$REAL_PYTHON" "$@"
  elif [ "$MODE" = runtime-read-failure ] && [[ "$*" == *runtime.json* ]]; then
    return 37
  else
    "$REAL_PYTHON" "$@"
  fi
}
function /usr/bin/env() {
  if [[ "$*" == *--version* ]]; then
    trace "version $*"
    if [[ "$*" == *biorouterd* ]] && [ "$MODE" = daemon-version ]; then return 0; fi
    if [[ "$*" != *biorouterd* ]] && [ "$MODE" = cli-version ]; then return 0; fi
    printf 'biorouter %s\n' "$VERSION"
    return
  fi
  trace "launch HOME=$HOME STATE=$XDG_STATE_HOME KEYRING=$BIOROUTER_DISABLE_KEYRING ARGS=$*"
  local userdata="" arg
  for arg in "$@"; do
    [[ "$arg" != --user-data-dir=* ]] || userdata="${arg#--user-data-dir=}"
  done
  [ -d "$userdata" ] && [ ! -L "$userdata" ] || return 38
  "$REAL_PYTHON" -c 'import os, stat, sys; metadata = os.stat(sys.argv[1]); assert metadata.st_uid == os.getuid() and stat.S_IMODE(metadata.st_mode) == 0o700' "$userdata" || return 38
  trace 'private userData exists before launch'
  mkdir -p "$userdata/logs" "$XDG_STATE_HOME/biorouter/daemon"
  case "$MODE" in
    not-ready) printf 'startup pending\n' >"$userdata/logs/main.log" ;;
    fatal) printf 'Fatal error during startup\n' >"$userdata/logs/main.log" ;;
    *) printf 'React ready event received\n' >"$userdata/logs/main.log" ;;
  esac
  "$REAL_PYTHON" - "$XDG_STATE_HOME/biorouter/daemon" "$MODE" <<'PYDATA'
import json, os, pathlib, sys
directory = pathlib.Path(sys.argv[1])
runtime = dict(pid=425002, profile_id='fixture-profile', instance_id='fixture-instance')
key = dict(runtime)
if sys.argv[2] == 'wrong-key':
    key['instance_id'] = 'other-fixture'
directory.chmod(0o700)
(directory / 'runtime.json').write_text(json.dumps(runtime))
key_path = directory / 'user-action-key.json'
key_path.write_text(json.dumps(key))
key_path.chmod(0o644 if sys.argv[2] == 'key-mode' else 0o600)
PYDATA
}
pgrep() {
  if [ "${4:-}" = osascript ]; then
    [ "$MODE" != prompt ] || printf '425003\n'
  elif [ "$2" != 425001 ] && [ "$2" != 425002 ]; then
    printf '425001\n425002\n'
  fi
}
kill() {
  trace "kill $*"
  if [ "$1" != -0 ]; then touch "$CASE_ROOT/stopped-$2"; fi
  [ "$MODE" != exited ] || [ "$1" != -0 ]
}
ps() {
  local pid="${@: -1}"
  trace "ps $*"
  if [ "$2" = comm= ]; then
    [ "$pid" != 425002 ] || printf '/fixture/biorouterd\n'
  elif [ "$pid" = 425002 ]; then
    [ "$MODE" != daemon-hangs ] || printf 'S\n'
  elif [ "$MODE" = app-hangs ]; then
    printf 'S\n'
  fi
}
sleep() { command sleep 0.002; }
'''


class MacReleaseSmoke(unittest.TestCase):
    def run_fixture(self, mode='normal', arch='arm64'):
        with tempfile.TemporaryDirectory(prefix='biorouter-mac-smoke-fixture-') as temporary:
            root = Path(temporary)
            baseline = root / 'preexisting'
            baseline.mkdir()
            for name, contents in [('default-keychain', 'owner-login'),
                                   ('search-list', 'historical-arm64\nhistorical-x64\n'),
                                   ('mount', 'baseline mount'), ('pid', 'baseline pid')]:
                (baseline / name).write_text(contents)
            before = {file.name: file.read_bytes() for file in baseline.iterdir()}
            artifact = root / 'fixture.dmg'
            artifact.write_text('fixture bytes, not a DMG')
            script = root / 'fixture.sh'
            script.write_text(
                'set -euo pipefail\n'
                + 'CASE_ROOT=' + shlex.quote(str(root)) + '\n'
                + 'ROOT=' + shlex.quote(str(root / 'repo')) + '\n'
                + 'REAL_PYTHON=' + shlex.quote(sys.executable) + '\n'
                + 'VERSION=1.93.0\nMODE=' + shlex.quote(mode) + '\n'
                + MAC_SMOKE + '\n' + STUBS
                + '\nsmoke_mac ' + shlex.quote(arch) + ' ' + shlex.quote(str(artifact)) + '\n'
            )
            result = subprocess.run(['/bin/bash', str(script)], capture_output=True,
                                    text=True, timeout=15, env={**os.environ})
            trace = (root / 'trace').read_text().splitlines()
            allocated = [Path(line[5:]) for line in trace if line.startswith('temp ')]
            self.assertEqual({file.name: file.read_bytes() for file in baseline.iterdir()}, before)
            self.assertFalse(any('security ' in line for line in trace), trace)
            self.assertTrue(all(path.parent == root for path in allocated))
            if mode == 'detach-failure':
                self.assertEqual([path.exists() for path in allocated], [True, False])
            else:
                self.assertTrue(all(not path.exists() for path in allocated), trace)
            detach = [line for line in trace if line.startswith('hdiutil detach')]
            self.assertTrue(detach, trace)
            self.assertTrue(all(str(allocated[0]) in line for line in detach), detach)
            self.assertTrue(all('preexisting' not in line for line in trace))
            signals = [line.split() for line in trace if line.startswith('kill ')]
            if signals:
                # The app's pid: the one `kill -0` polls, or, when a startup prompt ended
                # the wait before any poll, the first one stopped.
                app_pid = next((parts[2] for parts in signals if parts[1] == '-0'),
                               signals[0][2])
                self.assertTrue(all(parts[2] in {app_pid, '425001', '425002'}
                                    for parts in signals), signals)
                launch = next(line for line in trace if line.startswith('launch '))
                self.assertIn('HOME=' + str(allocated[1]), launch)
                self.assertIn('STATE=' + str(allocated[1] / '.local/state'), launch)
                self.assertIn('--user-data-dir=' + str(allocated[1] / 'userData'), launch)
            return result, trace

    def test_early_artifact_failures_clean_only_owned_paths_and_preserve_status(self):
        for mode in ('helper', 'codesign', 'spctl', 'stapler', 'entitlement', 'usage',
                     'rosetta', 'architecture', 'cli-version', 'daemon-version'):
            with self.subTest(mode=mode):
                result, trace = self.run_fixture(mode, 'x64')
                self.assertEqual(result.returncode, 37 if mode in
                                 ('helper', 'codesign', 'spctl', 'stapler') else 1, result.stderr)
                self.assertFalse(any(line.startswith(('launch ', 'kill ')) for line in trace))
                self.assertEqual(sum(line.startswith('hdiutil detach') for line in trace), 1)

    def test_microphone_failures_name_what_is_missing(self):
        result, _ = self.run_fixture('entitlement')
        self.assertIn('arm64 microphone: ', result.stderr)
        self.assertIn('Contents/Frameworks/Biorouter Helper.app', result.stderr)
        self.assertEqual(result.stderr.count('audio-input is not in the signed'), 1, result.stderr)
        result, _ = self.run_fixture('usage')
        self.assertIn('arm64 microphone: NSMicrophoneUsageDescription is '
                      '"This app needs access to the microphone"', result.stderr)

    def test_both_architectures_keep_native_checks_and_disposable_mock_keychain(self):
        for arch in ('arm64', 'x64'):
            with self.subTest(arch=arch):
                result, trace = self.run_fixture(arch=arch)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn('desktop shutdown passed', result.stdout)
                launch = next(line for line in trace if line.startswith('launch '))
                self.assertIn('--use-mock-keychain', launch)
                self.assertNotIn('--disable-gpu', launch)
                self.assertIn('KEYRING=true', launch)
                self.assertIn('--user-data-dir=', launch)
                self.assertIn('-u BIOROUTER_PATH_ROOT -u BIOROUTER_DEV_PROFILE_ROOT', launch)
                for check in ('helper ', 'codesign ', 'spctl ', 'xcrun ', 'plutil ', 'file ',
                              'version ', 'scan ', 'dictation self test '):
                    self.assertTrue(any(line.startswith(check) for line in trace), trace)
                # Both shipped binaries are held to their ceiling, by the platform's name.
                sizes = [line.split()[1:] for line in trace if line.startswith('size ')]
                self.assertEqual([(platform, path.rsplit('/', 1)[1]) for platform, path in sizes],
                                 [('darwin-' + arch, 'biorouter'), ('darwin-' + arch, 'biorouterd')],
                                 sizes)
                backend = 'metal' if arch == 'arm64' else 'cpu'
                self.assertTrue(any(line.startswith('dictation self test ')
                                    and '--expect-backend ' + backend in line
                                    for line in trace), trace)
                entitlements = [line for line in trace
                                if line.startswith('codesign -d --entitlements - --xml ')]
                self.assertEqual([line.rsplit('/Biorouter.app', 1)[1] for line in entitlements],
                                 ['', '/Contents/Frameworks/Biorouter Helper.app',
                                  '/Contents/Resources/bin/biorouter',
                                  '/Contents/Resources/bin/biorouterd'], entitlements)
                self.assertIn('validate private runtime key', trace)
                self.assertIn('private userData exists before launch', trace)
                self.assertEqual(sum(line.startswith('hdiutil detach') for line in trace), 1)
                self.assertIn('kill -KILL 425001', trace)
                self.assertIn('kill -TERM 425002', trace)
                self.assertNotIn('kill -KILL 425002', trace)

    def test_private_runtime_key_checks_still_block_success(self):
        for mode in ('wrong-key', 'key-mode'):
            with self.subTest(mode=mode):
                result, trace = self.run_fixture(mode)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn('did not save a private matching daemon key', result.stderr)
                self.assertIn('validate private runtime key', trace)
                self.assertIn('kill -TERM 425002', trace)
                self.assertNotIn('desktop shutdown passed', result.stdout)

    def test_readiness_and_shutdown_failures_remain_failures(self):
        for mode, message in [('not-ready', 'did not become ready within 30 s'),
                              ('fatal', 'failed during startup'),
                              ('prompt', 'opened a startup prompt'),
                              ('exited', 'exited during startup'),
                              ('app-hangs', 'did not exit within 20 s'),
                              ('daemon-hangs', 'did not stop within 15 s')]:
            with self.subTest(mode=mode):
                result, trace = self.run_fixture(mode)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn(message, result.stderr)
                self.assertNotIn('desktop shutdown passed', result.stdout)
                self.assertTrue(any(line.startswith('kill -TERM ') for line in trace))

    def test_unexpected_runtime_read_failure_still_cleans_launched_descendants(self):
        result, trace = self.run_fixture('runtime-read-failure')
        self.assertEqual(result.returncode, 37, result.stderr)
        self.assertIn('kill -KILL 425001', trace)
        self.assertIn('kill -KILL 425002', trace)

    def test_detach_retry_and_failure_do_not_mask_the_verdict(self):
        result, trace = self.run_fixture('detach-retry')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(sum(line.startswith('hdiutil detach') for line in trace), 2)
        result, trace = self.run_fixture('detach-failure')
        self.assertEqual(result.returncode, 1, result.stderr)
        self.assertIn('smoke mount did not detach', result.stderr)
        self.assertNotIn('desktop shutdown passed', result.stdout)


if __name__ == '__main__':
    unittest.main()
