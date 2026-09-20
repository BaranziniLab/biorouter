#!/usr/bin/env python3
"""Assert the real installed CLI resolves its bundled native payload, without capture."""
import argparse
from contextlib import contextmanager, nullcontext
import hashlib
import json
import os
from pathlib import Path
import queue
import signal
import subprocess
import sys
import tempfile
import threading
import time


def digest(path):
    value = hashlib.sha256()
    with path.open('rb') as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b''):
            value.update(block)
    return value.hexdigest()


def helper_processes(executable):
    result = subprocess.run(['/bin/ps', '-axo', 'pid=,lstart=,comm='], capture_output=True,
                            text=True, check=True, timeout=10)
    processes = {}
    for line in result.stdout.splitlines():
        fields = line.strip().split(maxsplit=6)
        if len(fields) == 7 and fields[6] == str(executable):
            processes[int(fields[0])] = ' '.join(fields[1:6])
    return processes


@contextmanager
def owned_app_agents(executable):
    before = helper_processes(executable)
    try:
        yield
    finally:
        after = helper_processes(executable)
        for pid, started in after.items():
            if pid in before:
                continue
            # Recheck both executable and start time immediately before each signal.
            if helper_processes(executable).get(pid) != started:
                continue
            try:
                os.kill(pid, signal.SIGTERM)
            except ProcessLookupError:
                continue
            deadline = time.monotonic() + 5
            while time.monotonic() < deadline and helper_processes(executable).get(pid) == started:
                time.sleep(0.1)
            if helper_processes(executable).get(pid) == started:
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                deadline = time.monotonic() + 2
                while time.monotonic() < deadline and helper_processes(executable).get(pid) == started:
                    time.sleep(0.1)
                if helper_processes(executable).get(pid) == started:
                    raise RuntimeError(f'Owned helper {pid} did not exit after cleanup')


def validate_doctor(document, helper, target, expected_status):
    report = document['computer_use']
    manifest = json.loads((helper / 'manifest.json').read_text(encoding='utf-8'))
    expected = (helper / manifest['executable']).resolve()
    if report.get('integrity') != 'verified' or report.get('development_override') is not False:
        raise ValueError('Installed runtime integrity/override assertion failed')
    # Compare filesystem IDENTITY, not path spelling. Two paths can name one
    # file and still differ as strings: Windows' verbatim prefix (`\\?\C:\…`,
    # which `Path.resolve()` preserves when it is already present), 8.3 short
    # names, and case. The product now strips the verbatim prefix, but
    # `dunce::simplified` deliberately declines to strip it when the plain form
    # is not equivalent — a path over 260 characters, a reserved DOS name, a
    # component ending in a dot or space — so a deep install directory would
    # reintroduce exactly this failure. `os.path.samefile` compares file IDs and
    # is immune to all of it.
    reported = Path(report.get('executable', ''))
    # ⚠ Both sides must be probed BEFORE `samefile`, which raises `OSError` when
    # either path is absent. Guarding only the reported side means a missing
    # INSTALLED payload -- precisely the broken install this check exists to
    # catch -- surfaces as a bare `FileNotFoundError` naming one path, instead of
    # the sentence naming both. A check that cannot explain itself is barely a
    # check.
    missing = [str(p) for p in (reported, expected) if not p.exists()]
    if missing or not os.path.samefile(reported, expected):
        detail = f'; does not exist: {", ".join(missing)}' if missing else ''
        raise ValueError(
            'Doctor resolved outside the installed payload '
            f'(reported {reported!s}, expected {expected!s}{detail})'
        )
    if report.get('target') != target or report.get('runtime_version') != manifest['upstream_version']:
        raise ValueError('Installed runtime target/version mismatch')
    if report.get('status') not in expected_status:
        raise ValueError(f"Unexpected installed readiness: {report.get('status')}")
    permissions = report.get('permissions')
    if not isinstance(permissions, dict) or any(type(permissions.get(key)) is not bool for key in ['accessibility', 'screen_recording']):
        raise ValueError('Native permission facts are absent or malformed')
    if report['status'] == 'ready' and not all([
        report.get('desktop_available'), report.get('capture_available'),
        permissions['accessibility'], permissions['screen_recording'],
    ]):
        raise ValueError('Ready state contradicts native permission/capture facts')
    if report['status'] == 'desktop_unavailable' and report.get('desktop_available') is not False:
        raise ValueError('No-desktop state contradicts desktop availability')
    return report


def builtin_tools(cli, env):
    # rmcp closes on EOF; keep stdin open until each response has actually arrived.
    with tempfile.TemporaryFile(mode='w+') as errors:
        child = subprocess.Popen([str(cli), 'mcp', 'computercontroller'], stdin=subprocess.PIPE,
                                 stdout=subprocess.PIPE, stderr=errors, text=True, env=env)
        lines = queue.Queue()
        def read_lines():
            for line in child.stdout:
                lines.put(line)
            lines.put(None)
        reader = threading.Thread(target=read_lines, daemon=True)
        reader.start()
        def send(message):
            child.stdin.write(json.dumps(message) + '\n')
            child.stdin.flush()
        def receive(identifier):
            deadline = time.monotonic() + 20
            while time.monotonic() < deadline:
                line = lines.get(timeout=max(0.01, deadline - time.monotonic()))
                if line is None:
                    raise ValueError('Installed MCP exited before its response')
                response = json.loads(line)
                if response.get('id') == identifier:
                    if 'error' in response:
                        raise ValueError(f'Installed MCP error: {response["error"]}')
                    return response['result']
            raise TimeoutError('Installed MCP response timed out')
        try:
            send({'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {
                'protocolVersion': '2025-03-26', 'capabilities': {},
                'clientInfo': {'name': 'installed-package-check', 'version': '1'}}})
            receive(1)
            send({'jsonrpc': '2.0', 'method': 'notifications/initialized'})
            send({'jsonrpc': '2.0', 'id': 2, 'method': 'tools/list', 'params': {}})
            return receive(2)['tools']
        finally:
            child.stdin.close()
            try:
                child.wait(timeout=5)
            except subprocess.TimeoutExpired:
                child.kill()
                child.wait(timeout=5)
            reader.join(timeout=1)
            child.stdout.close()


# A budget PER ATTEMPT, summing to the bound the callers size themselves
# against. One shared deadline let a slow cold run squeeze the warm one -- on
# Windows to 14 s -- so the error named the warm attempt when the cold one was
# the problem. The numbers come from measurement, not taste: the slowest
# platform that PASSES needs ~13.3 s per attempt (darwin-arm64), so 40 s cold
# leaves roughly 3x headroom and is TIGHTER than the 60 s a cold run could
# reach before. scripts/computer-use-package-acceptance.py imports the total
# rather than restating it.
ATTEMPT_TIMEOUTS = {'cold': 40, 'warm': 20}
DOCTOR_TIMEOUT = sum(ATTEMPT_TIMEOUTS.values())


# What a timeout report is looking for. A whole-machine snapshot buries the two
# processes that matter in several hundred system daemons.
RELEVANT = ('biorouter', 'ocu', 'OpenComputerUse', 'llama-server', 'powershell', 'node')


def snapshot(fields):
    """Live processes as `pid rest` lines, EXCLUDING the query itself.

    The exclusion is not tidiness. `powershell` is in RELEVANT and the POSIX arm
    shells out to `ps`, so without it the snapshot reports its own helper as a
    surviving process -- which it did, in four receipts of the run that motivated
    this, listing `ps` as a process left behind.
    """
    command = (['powershell', '-NoProfile', '-Command',
                '$me = $PID; Get-CimInstance Win32_Process | Where-Object { $_.ProcessId -ne $me } '
                '| ForEach-Object { "' + fields + '" }']
               if os.name == 'nt' else ['/bin/ps', '-axo', 'pid=,ppid=,comm='])
    # Popen, not run(): the POSIX arm cannot exclude itself from inside `ps`, so
    # the caller needs the child's pid to drop that one row.
    try:
        child = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True)
        out, _ = child.communicate(timeout=20)
    except (OSError, subprocess.SubprocessError) as error:
        return None, f'process snapshot unavailable: {error}'
    rows = {}
    for line in (out or '').splitlines():
        parts = line.split(maxsplit=1)
        if len(parts) == 2 and parts[0].isdigit():
            rows[parts[0]] = parts[1].strip()
    rows.pop(str(child.pid), None)
    return rows, None


def process_tree():
    """Live processes that could plausibly be holding the doctor's stdout."""
    rows, failure = snapshot('$($_.ProcessId) $($_.ParentProcessId) $($_.Name)')
    if rows is None:
        return failure
    named = [f'{pid} {rest}' for pid, rest in rows.items()
             if any(token in rest for token in RELEVANT)]
    return '\n'.join(named[:40] or ['(no Biorouter-related process alive)']) + \
        f'\n  [{len(named)} relevant of {len(rows)} total processes]'


def describe_output(written):
    """Say what the CLI had actually produced, judged by PARSEABILITY.

    Not by byte count: Rust's stdout is line-buffered over a 1 KiB writer, so a
    doctor that hangs partway through has already flushed kilobytes. Calling any
    non-zero length "complete" reported a truncated document as a finished run,
    which points a future debugger at a phantom.
    """
    if not written:
        return 'no output, so the work had not finished'
    try:
        json.loads(written)
    except ValueError:
        return f'{len(written)} bytes of a TRUNCATED document, so the work had not finished'
    return (f'{len(written)} bytes forming a COMPLETE document, so the work finished and '
            'something else was holding its stdout')


def live_processes():
    """pid -> command, for every process this user can see. Empty on failure."""
    rows, _ = snapshot('$($_.ProcessId) $($_.Name)')
    return rows or {}


def survivors(before, after):
    """Processes that appeared during the run and are STILL alive afterwards.

    Deliberately not a parent-pid lookup: once the CLI exits, anything it left
    behind is re-parented to init, so the link that would identify it is gone at
    exactly the moment we want to check. A before/after difference survives that.
    """
    return sorted(f'{pid} {name}' for pid, name in after.items() if pid not in before)


def stop_tree(child):
    """Stop a hung doctor and everything it started, without ever raising.

    The informative ValueError this precedes must always win: a reap that
    itself times out (a process wedged in uninterruptible I/O) would otherwise
    replace the diagnosis with a bare TimeoutExpired.
    """
    try:
        if os.name == 'nt':
            subprocess.run(['taskkill', '/F', '/T', '/PID', str(child.pid)],
                           capture_output=True, timeout=30)
        else:
            os.killpg(os.getpgid(child.pid), signal.SIGKILL)
    except (OSError, subprocess.SubprocessError):
        pass
    try:
        child.kill()
    except OSError:
        pass
    try:
        child.wait(timeout=30)
    except subprocess.TimeoutExpired:
        pass


def tail_text(path, limit=4000):
    """The tail of a text file written by a process we just killed.

    Best-effort on purpose: this runs inside the construction of an error
    message, and a diagnostic that can itself raise would replace the real
    failure with its own.
    """
    try:
        text = path.read_text(encoding='utf-8', errors='replace').strip()
    except OSError as error:
        return f'(stderr unreadable: {error})'
    if not text:
        return '(stderr was empty -- doctor produced no phase breadcrumb at all, ' \
               'so it had not reached its first phase)'
    return text[-limit:]


def run_doctor(cli, env, scratch, evidence=None):
    """Run the installed `doctor` twice, observing process exit independently of its pipes.

    Output goes to real files rather than pipes, and exit is observed with
    wait(), because `capture_output=True` blocks until EVERY writer closes the
    pipe -- including a grandchild that inherited the handle. That makes "the
    command is still working" and "the command finished but a descendant holds
    its stdout" the same observation, and they are different defects.

    The call is made twice on purpose: first execution of a freshly extracted
    binary is an operating-system scan cost at near-zero CPU, so a cold/warm
    pair is the cleanest evidence of whether latency is first-run or structural.
    """
    timings = []
    document = None
    for attempt, budget in ATTEMPT_TIMEOUTS.items():
        # ⚠ Write the breadcrumbs somewhere DURABLE. They used to go into the
        # caller's `TemporaryDirectory`, which is deleted on scope exit, so the
        # phase trace for the run that actually finished -- the one that
        # localises where the time went -- was discarded before anything could
        # upload it. Downloading `installed-packages-win32-x64` from the run
        # that measured 34.91 s yields `backends.json` and nothing else.
        target_dir = evidence if evidence is not None else scratch
        out = target_dir / f'doctor-{attempt}.json'
        err = target_dir / f'doctor-{attempt}.err'
        started = time.monotonic()
        before = live_processes()
        with out.open('wb') as stdout, err.open('wb') as stderr:
            # Its own process group / job, so abandoning a hung doctor takes its
            # descendants with it. Otherwise they keep running on the runner and
            # can hold the extracted package files open, which then fails the
            # artifact upload for a reason that looks unrelated.
            grouping = ({'creationflags': subprocess.CREATE_NEW_PROCESS_GROUP}
                        if os.name == 'nt' else {'start_new_session': True})
            child = subprocess.Popen([str(cli), 'doctor', '--format', 'json', '--no-update'],
                                     stdin=subprocess.DEVNULL, stdout=stdout, stderr=stderr,
                                     env=env, **grouping)
            try:
                code = child.wait(timeout=budget)
            except subprocess.TimeoutExpired:
                written = out.read_bytes()
                # ⚠ Capture the tree BEFORE killing anything. This used to call
                # `process_tree()` inside the message below, i.e. AFTER
                # `stop_tree(child)` had already taken the process and its
                # descendants down -- so the one diagnostic that exists to say
                # WHAT was hung reported "(no Biorouter-related process alive)
                # [0 relevant of 136 total processes]" every single time, at
                # exactly the moment it mattered. Measured on run 35474721330.
                hung_tree = process_tree()
                stop_tree(child)
                # ⚠ Read stderr too. stdout carries the JSON and is written only
                # at the END, so on a timeout it is empty BY CONSTRUCTION and
                # says nothing about where the time went -- which left the
                # Windows job reporting "exceeded its 40s budget, stdout held no
                # output" and no way to tell a slow dependency probe from a slow
                # Computer Use probe from a wedged process. `doctor` writes one
                # `[doctor] <phase> (+Ns)` breadcrumb per phase to stderr for
                # exactly this moment; the non-timeout failure path below already
                # reported stderr, and only this path threw it away.
                trace = tail_text(err)
                raise ValueError(
                    f'Installed doctor ({attempt}) exceeded its {budget}s budget. '
                    f'Its stdout held {describe_output(written)}. '
                    # Carry what already finished, so a reader sees the cold cost
                    # instead of inferring it from what the warm run had left.
                    f'Completed attempts: {timings or "none"}. '
                    f'Phase trace (stderr):\n{trace}\n'
                    f'Live processes:\n{hung_tree}') from None
        elapsed = time.monotonic() - started
        # A descendant that outlives the CLI is the OTHER mechanism that can make
        # a doctor call look hung. Observing exit independently of the pipes stops
        # it failing the job -- so it has to be RECORDED, or the fix would simply
        # make a real leak invisible instead of loud.
        left_behind = survivors(before, live_processes())
        timings.append({'attempt': attempt, 'seconds': round(elapsed, 2), 'exit_code': code,
                        'processes_left_behind': left_behind})
        if code != 0:
            raise ValueError(f'Installed doctor ({attempt}) exited {code}: {err.read_text(encoding="utf-8", errors="replace")[:2000]}')
        document = json.loads(out.read_text(encoding='utf-8'))
    return document, timings


def check(cli, helper, target, expected_status, expected_backends, report_path):
    helper = helper.resolve()
    suffix = '.exe' if target.startswith('win32') else ''
    expected = json.loads(expected_backends.read_text(encoding='utf-8'))
    for name in ['biorouter', 'biorouterd']:
        binary = cli.parent / (name + suffix)
        if name == 'biorouterd' and target.startswith('win32') and not binary.exists():
            binary = helper.parent / 'bin' / (name + suffix)
        if digest(binary) != expected['backends'][name + suffix]:
            raise ValueError(f'Installed {name} differs from production build')
    manifest = json.loads((helper / 'manifest.json').read_text(encoding='utf-8'))
    agents = owned_app_agents(helper / manifest['executable']) if sys.platform == 'darwin' else nullcontext()
    with agents, tempfile.TemporaryDirectory(prefix='biorouter-installed-check-') as isolated:
        env = dict(os.environ, BIOROUTER_PATH_ROOT=isolated, BIOROUTER_DISABLE_KEYRING='true')
        for key in ['BIOROUTER_COMPUTER_USE_DIR', 'OPEN_COMPUTER_USE_DISABLE_APP_AGENT_PROXY']:
            env.pop(key, None)
        # `report_path.parent` is `target/package-acceptance/`, which the
        # workflow uploads with `if-no-files-found: error`. The isolated dir
        # stays the config root; only the evidence moves somewhere that outlives
        # the failure it documents.
        evidence = report_path.parent
        evidence.mkdir(parents=True, exist_ok=True)
        document, doctor_timings = run_doctor(cli, env, Path(isolated), evidence)
        report = validate_doctor(document, helper, target, expected_status)
        tools = builtin_tools(cli, env)
        names = [tool['name'] for tool in tools]
        required = {'list_apps', 'get_app_state', 'click', 'perform_secondary_action', 'scroll',
                    'drag', 'type_text', 'press_key', 'set_value', 'screen_capture'}
        if set(names) != required or len(names) != 10:
            raise ValueError(f'Installed builtin tool census mismatch: {names}')
    receipt = {'cli': str(cli), 'target': target, 'doctor': report, 'tools': sorted(names),
               'doctor_timings': doctor_timings,
               'backend_hashes': expected['backends'], 'source_commit': expected['source_commit'],
               'helper_manifest_sha256': digest(helper / 'manifest.json'),
               'desktop_actions_performed': False}
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(receipt, indent=2) + '\n', encoding='utf-8')
    print(json.dumps(receipt))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cli', type=Path, required=True)
    parser.add_argument('--helper', type=Path, required=True)
    parser.add_argument('--target', required=True)
    parser.add_argument('--expect-status', required=True, help='Comma-separated permitted factual states')
    parser.add_argument('--backends', type=Path, required=True)
    parser.add_argument('--report', type=Path, required=True)
    args = parser.parse_args()
    check(args.cli.resolve(), args.helper, args.target, args.expect_status.split(','), args.backends, args.report)
