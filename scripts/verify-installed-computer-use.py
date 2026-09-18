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
    manifest = json.loads((helper / 'manifest.json').read_text())
    expected = (helper / manifest['executable']).resolve()
    if report.get('integrity') != 'verified' or report.get('development_override') is not False:
        raise ValueError('Installed runtime integrity/override assertion failed')
    if Path(report.get('executable', '')).resolve() != expected:
        raise ValueError('Doctor resolved outside the installed payload')
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


def check(cli, helper, target, expected_status, expected_backends, report_path):
    helper = helper.resolve()
    suffix = '.exe' if target.startswith('win32') else ''
    expected = json.loads(expected_backends.read_text())
    for name in ['biorouter', 'biorouterd']:
        binary = cli.parent / (name + suffix)
        if name == 'biorouterd' and target.startswith('win32') and not binary.exists():
            binary = helper.parent / 'bin' / (name + suffix)
        if digest(binary) != expected['backends'][name + suffix]:
            raise ValueError(f'Installed {name} differs from production build')
    manifest = json.loads((helper / 'manifest.json').read_text())
    agents = owned_app_agents(helper / manifest['executable']) if sys.platform == 'darwin' else nullcontext()
    with agents, tempfile.TemporaryDirectory(prefix='biorouter-installed-check-') as isolated:
        env = dict(os.environ, BIOROUTER_PATH_ROOT=isolated, BIOROUTER_DISABLE_KEYRING='true')
        for key in ['BIOROUTER_COMPUTER_USE_DIR', 'OPEN_COMPUTER_USE_DISABLE_APP_AGENT_PROXY']:
            env.pop(key, None)
        result = subprocess.run([str(cli), 'doctor', '--format', 'json', '--no-update'],
                                capture_output=True, text=True, env=env, timeout=60, check=True)
        document = json.loads(result.stdout)
        report = validate_doctor(document, helper, target, expected_status)
        tools = builtin_tools(cli, env)
        names = [tool['name'] for tool in tools]
        required = {'list_apps', 'get_app_state', 'click', 'perform_secondary_action', 'scroll',
                    'drag', 'type_text', 'press_key', 'set_value', 'screen_capture'}
        if set(names) != required or len(names) != 10:
            raise ValueError(f'Installed builtin tool census mismatch: {names}')
    receipt = {'cli': str(cli), 'target': target, 'doctor': report, 'tools': sorted(names),
               'backend_hashes': expected['backends'], 'source_commit': expected['source_commit'],
               'helper_manifest_sha256': digest(helper / 'manifest.json'),
               'desktop_actions_performed': False}
    report_path.parent.mkdir(parents=True, exist_ok=True)
    report_path.write_text(json.dumps(receipt, indent=2) + '\n')
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
