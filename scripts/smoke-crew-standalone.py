#!/usr/bin/env python3
"""Run the actual standalone Crew refusal with an externally prepared provider profile."""

import argparse
import json
import os
import re
from pathlib import Path
import shutil
import signal
import socket
import stat
import subprocess
import sys
import threading
import time
import xml.etree.ElementTree as ET


REQUEST = {'method': 'messages.history', 'params': {
    'channel_id': '00000000-0000-4000-8000-0000000002e5', 'limit': 1}}
DENIAL = "This chat isn't connected to a Crew channel. To connect it, type /crew in this chat."


class EvidenceError(Exception):
    def __init__(self, reason, schema=None):
        super().__init__(reason)
        self.schema = schema


def require(condition, reason):
    if not condition:
        raise EvidenceError(reason)


def exact_arguments(value):
    return json.loads(value) if isinstance(value, str) else value


def protocol_schema(value):
    def kind(item):
        if item is None:
            return 'null'
        return {dict: 'object', list: 'array', str: 'string', bool: 'boolean',
                int: 'number', float: 'number'}.get(type(item), 'other')

    known = {'content', 'isError', 'structuredContent', '_meta'}
    schema = {'known_fields': {key: kind(value[key]) for key in sorted(known & value.keys())},
              'unknown_field_count': len(value.keys() - known)}
    if isinstance(value.get('content'), list):
        schema['content_item_fields'] = [
            sorted({'type', 'text', 'resource', 'data', 'mimeType', 'annotations', '_meta'} & item.keys())
            for item in value['content'][:4] if isinstance(item, dict)]
    if isinstance(value.get('content'), list):
        schema['content_item_types'] = [
            item.get('type') if item.get('type') in {'text', 'image', 'audio', 'resource', 'resource_link'}
            else 'other' for item in value['content'][:4] if isinstance(item, dict)]
    if isinstance(value.get('isError'), bool) or value.get('isError') is None:
        schema['isError'] = value.get('isError')
    return schema


def tool_text(value):
    require(set(value) <= {'content', 'isError', '_meta', 'structuredContent'},
            'unexpected additional tool payload')
    require(value.get('isError') is False or value.get('isError') is None, 'tool result is an error')
    require(value.get('structuredContent') is None or value.get('structuredContent') == {},
            'tool result contains structured payload')
    content = value.get('content')
    require(isinstance(content, list) and len(content) == 1 and content[0].get('type') == 'text'
            and set(content[0]) == {'type', 'text'},
            'extra or nontext tool content')
    text = content[0]['text'].strip()
    if text.startswith('<tool-output'):
        wrapper = ET.fromstring(text)
        require(wrapper.tag == 'tool-output' and len(wrapper) == 0 and not wrapper.tail,
                'unexpected tool-output payload')
        text = (wrapper.text or '').strip()
    return text


def validate_trace(output):
    requests, responses = [], []
    for line in output.splitlines():
        if not line.strip():
            continue
        event = json.loads(line)
        for item in event.get('message', {}).get('content', []):
            if item.get('type') == 'toolRequest':
                requests.append(item)
            elif item.get('type') == 'toolResponse':
                responses.append(item)
    require(len(requests) == len(responses) and 1 <= len(requests) <= 2,
            'unexpected request or response count')
    response_ids = [item['id'] for item in responses]
    require(len(set(response_ids)) == len(response_ids), 'duplicate response IDs')
    request_ids = [item['id'] for item in requests]
    require(len(set(request_ids)) == len(request_ids) and set(request_ids) == set(response_ids),
            'tool response ID mismatch')
    pairs = {item['id']: item for item in responses}
    planning = 0
    crew_value, crew_name, crew_arguments = None, None, None
    for request in requests:
        response = pairs[request['id']]
        call, result = request['toolCall'], response['toolResult']
        require(call['status'] == result['status'] == 'success', 'tool transport failed')
        name, arguments = call['value']['name'], call['value']['arguments']
        value = result['value']
        if name == 'todo__todo_write':
            require(planning == 0 and crew_value is None, 'unexpected local planning call')
            require(isinstance(arguments, dict) and set(arguments) == {'content'} and
                    isinstance(arguments['content'], str) and len(arguments['content']) <= 2048,
                    'unexpected planning arguments')
            require(not (value.get('_meta') or {}).get('biorouter/tool-calls'),
                    'planning response contains nested calls')
            summary = re.fullmatch(r'Todo list set: ([0-9]+) item\(s\)', tool_text(value))
            require(summary is not None and int(summary[1]) <= 200,
                    'unexpected local planning result')
            planning += 1
            continue
        require(crew_value is None, 'multiple Crew calls')
        crew_value, crew_name, crew_arguments = value, name, arguments
    require(crew_value is not None, 'missing Crew request')
    value, name, arguments = crew_value, crew_name, crew_arguments
    nested = (value.get('_meta') or {}).get('biorouter/tool-calls')
    if name == 'crew__request':
        require(exact_arguments(arguments) == REQUEST, 'request arguments mismatch')
    else:
        require(name == 'code_execution__execute_code', 'unexpected outer tool')
        graph = arguments.get('tool_graph')
        require(isinstance(graph, list) and len(graph) == 1, 'unexpected tool graph')
        require(graph[0].get('tool') == 'crew/request' and not graph[0].get('depends_on'),
                'unexpected tool graph entry')
        require(nested is not None, 'missing nested request evidence')
    if nested is not None:
        require(isinstance(nested, list) and len(nested) == 1, 'unexpected nested call count')
        require(nested[0].get('tool') == 'crew__request' and nested[0].get('status') == 'ok',
                'unexpected nested tool')
        require(exact_arguments(nested[0].get('args')) == REQUEST, 'nested arguments mismatch')
    text = tool_text(value)
    if name == 'code_execution__execute_code' and text.startswith('Result: '):
        try:
            text = json.loads(text[len('Result: '):])
        except json.JSONDecodeError as error:
            raise EvidenceError('invalid runtime result scalar') from error
        if isinstance(text, dict):
            if not set(text) <= {'content', 'isError', '_meta', 'structuredContent'}:
                raise EvidenceError('unexpected returned MCP result object', protocol_schema(text))
            require(not text.get('_meta'), 'returned MCP result has metadata payload')
            text = tool_text(text)
        else:
            require(isinstance(text, str), 'runtime result is not a scalar string')
    require(text == DENIAL, 'tool returned more than the exact no-channel denial')
    return {'request_count': 1, 'local_planning_calls': planning,
            'exact_arguments': True, 'response_id_matches': True,
            'exact_denial': True, 'channel_content_returned': False}


def owned(path, directory=False, mode=None):
    entry = path.lstat()
    predicate = stat.S_ISDIR if directory else stat.S_ISREG
    require(predicate(entry.st_mode) and entry.st_uid == os.getuid(), 'invalid owned fixture')
    require(directory or entry.st_nlink == 1, 'fixture file has multiple links')
    require(mode is None or stat.S_IMODE(entry.st_mode) == mode, 'invalid fixture permissions')
    return (entry.st_dev, entry.st_ino, entry.st_size, entry.st_mtime_ns, entry.st_ctime_ns,
            stat.S_IMODE(entry.st_mode))


def runtime_paths(root):
    found = set()
    for directory, directories, files in os.walk(root, followlinks=False):
        for name in directories:
            if name == '.ssh':
                found.add(str((Path(directory) / name).relative_to(root)))
        for name in files:
            path = Path(directory) / name
            if name in ('runtime.json', 'owner.lock', 'broker.sock', 'user-action-key.json'):
                found.add(str(path.relative_to(root)))
            elif stat.S_ISSOCK(path.lstat().st_mode):
                found.add(str(path.relative_to(root)))
    return found


def processes():
    result = subprocess.run(['/bin/ps', '-axo', 'pid=,ppid=,comm='],
                            capture_output=True, text=True, check=True, timeout=5)
    entries = {}
    for line in result.stdout.splitlines():
        parts = line.strip().split(None, 2)
        if len(parts) == 3:
            entries[int(parts[0])] = (int(parts[1]), Path(parts[2]).name)
    return entries


def observe_listeners(lsof, pids):
    observation = subprocess.run([lsof, '-w', '-nP', '-a', '-p',
                                  ','.join(map(str, sorted(pids))), '-iTCP', '-sTCP:LISTEN',
                                  '-F', 'pfnT'], capture_output=True, text=True, timeout=5)
    require(observation.returncode in (0, 1) and not observation.stderr.strip(),
            'listener observation unavailable')
    return observation.stdout


def verify_listener_observer(lsof):
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as control:
        control.bind(('127.0.0.1', 0))
        control.listen(1)
        fields = observe_listeners(lsof, {os.getpid()}).splitlines()
        expected = 'n127.0.0.1:' + str(control.getsockname()[1])
        require('p' + str(os.getpid()) in fields and expected in fields and 'TST=LISTEN' in fields,
                'owned listener positive control not observed')


def ordinary_bridge_listeners(output, cli_pid):
    records, current, pid = [], None, None
    for line in output.splitlines():
        require(bool(line), 'invalid listener field')
        if line.startswith('p'):
            if current is not None:
                records.append(current)
                current = None
            pid = int(line[1:])
        elif line.startswith('f'):
            if current is not None:
                records.append(current)
            current = {'pid': pid}
        elif line.startswith('n'):
            require(current is not None, 'listener owner missing')
            current['endpoint'] = line[1:]
        elif line.startswith('TST='):
            require(current is not None, 'listener state owner missing')
            current['state'] = line[4:]
        else:
            require(line.startswith('T'), 'unexpected listener field')
    if current is not None:
        records.append(current)
    require(len(records) <= 1, 'multiple runtime listeners')
    endpoints = set()
    for record in records:
        require(record['pid'] == cli_pid and record.get('state') == 'LISTEN',
                'unexpected listener owner or state')
        address, separator, port = record.get('endpoint', '').rpartition(':')
        require(separator and address == '127.0.0.1' and port.isdigit() and 0 < int(port) <= 65535,
                'unexpected nonloopback listener')
        endpoints.add(record['endpoint'])
    return endpoints


def write_private(path, payload):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, 'wb') as output:
        output.write(payload)


def live_group_members(group, baseline):
    snapshot = subprocess.run(['/bin/ps', '-axo', 'pid=,pgid=,uid=,stat='],
                              capture_output=True, text=True, check=True, timeout=5)
    members = set()
    for line in snapshot.stdout.splitlines():
        parts = line.split()
        if len(parts) == 4 and int(parts[1]) == group and not parts[3].startswith('Z'):
            pid, uid = int(parts[0]), int(parts[2])
            require(pid not in baseline and uid == os.getuid(), 'process group ownership changed')
            members.add(pid)
    return members


def stop_invocation_group(group, baseline):
    def send(sig):
        members = live_group_members(group, baseline)
        if not members:
            return
        try:
            os.killpg(group, sig)
        except ProcessLookupError:
            pass
        except PermissionError:
            # An orphaned group can reject a group signal; each live member is still verified.
            for pid in live_group_members(group, baseline):
                try:
                    os.kill(pid, sig)
                except ProcessLookupError:
                    pass

    send(signal.SIGTERM)
    deadline = time.monotonic() + 3
    while live_group_members(group, baseline) and time.monotonic() < deadline:
        time.sleep(0.05)
    send(signal.SIGKILL)
    deadline = time.monotonic() + 3
    while live_group_members(group, baseline) and time.monotonic() < deadline:
        time.sleep(0.05)
    require(not live_group_members(group, baseline), 'owned process group survived cleanup')


def invoke(command, prompt, env, workdir, profile, phase_root, lsof, timeout):
    baseline = processes()
    child = subprocess.Popen(command, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                             stderr=subprocess.PIPE, cwd=workdir, env=env, start_new_session=True)
    captured = {}

    def collect():
        captured['output'] = child.communicate(prompt.encode())

    reader = threading.Thread(target=collect)
    reader.start()
    tracked = {child.pid}
    samples = 0
    bridge_endpoints = set()
    violation = None
    try:
        deadline = time.monotonic() + timeout
        while child.poll() is None:
            require(time.monotonic() < deadline, 'standalone invocation timed out')
            require(not runtime_paths(profile) and not runtime_paths(phase_root),
                    'phase runtime or SSH state created')
            snapshot = processes()
            changed = True
            while changed:
                new = {pid for pid, (parent, _) in snapshot.items() if parent in tracked}
                changed = not new.issubset(tracked)
                tracked.update(new)
            live = tracked & snapshot.keys()
            require(not any(snapshot[pid][1] in ('biorouterd', 'ssh') for pid in live),
                    'phase child started a daemon or SSH')
            if live:
                observation = observe_listeners(lsof, live)
                if child.poll() is None:
                    bridge_endpoints.update(ordinary_bridge_listeners(observation, child.pid))
                    require(len(bridge_endpoints) <= 1, 'ordinary bridge endpoint changed')
                    samples += 1
            time.sleep(0.15)
        reader.join(timeout=5)
        require(not reader.is_alive() and 'output' in captured, 'could not collect native output')
        require(child.returncode == 0, 'native CLI returned nonzero')
        require(samples > 0, 'live listener ownership not observed')
        stdout, _stderr = captured['output']
        require(len(stdout) <= 8 * 1024 * 1024, 'native trace exceeds bound')
        require(len(bridge_endpoints) == 1, 'ordinary CLI bridge was not observed')
        require(not runtime_paths(profile) and not runtime_paths(phase_root), 'phase runtime created')
        require(not ((tracked - {child.pid}) & processes().keys()), 'phase child remains running')
        return stdout.decode('utf-8'), samples
    except Exception:
        violation = True
        raise
    finally:
        if violation:
            stop_invocation_group(child.pid, baseline)
            child.wait(timeout=3)
        reader.join(timeout=5)
        if 'output' in captured:
            write_private(workdir / 'native-trace.jsonl', captured['output'][0])
            write_private(workdir / 'native-diagnostics.log', captured['output'][1])
        # Existing host PIDs are never candidates for cleanup or failure attribution.
        require(child.pid not in baseline, 'unexpected child PID reuse')


def run(args):
    cli, profile, workdir = Path(args.cli), Path(args.profile_root), Path(args.workdir)
    require(cli.is_absolute() and cli.is_file() and os.access(cli, os.X_OK), 'invalid CLI handle')
    require(profile.is_absolute() and workdir.is_absolute(), 'fixture handles must be absolute')
    owned(profile, directory=True, mode=0o700)
    owned(profile / 'config', directory=True, mode=0o700)
    secrets_before = owned(profile / 'config/secrets.yaml', mode=0o600)
    config_before = owned(profile / 'config/config.yaml', mode=0o600)
    require(not workdir.exists() and not workdir.is_symlink(), 'workdir must be fresh')
    workdir.mkdir(mode=0o700)
    owned(workdir, directory=True, mode=0o700)
    lsof = shutil.which('lsof') or ('/usr/sbin/lsof' if Path('/usr/sbin/lsof').is_file() else None)
    require(lsof is not None, 'listener observer unavailable')
    verify_listener_observer(lsof)
    require(not runtime_paths(profile), 'provider-only fixture already has runtime state')
    env = {'PATH': str(cli.parent) + os.pathsep + os.defpath,
           'BIOROUTER_PATH_ROOT': str(profile), 'BIOROUTER_DISABLE_KEYRING': '1',
           'BIOROUTER_TELEMETRY_ENABLED': 'false'}
    common = [str(cli), 'run', '--no-session', '--provider', args.provider,
              '--model', args.model, '--max-turns', '2', '--quiet', '--output-format', 'stream-json',
              '-i', '-']
    outputs, samples = {}, {}
    for label in ('ordinary', 'crew'):
        cwd = workdir / label
        cwd.mkdir(mode=0o700)
        home = cwd / 'home'
        home.mkdir(mode=0o700)
        env['HOME'] = str(home)
        command = common if label == 'ordinary' else common + ['--with-builtin', 'crew']
        prompt = 'Reply exactly OK. Do not call any tools.' if label == 'ordinary' else (
            'Make one simple Crew MCP request; do not use Todo or planning tools. Exact arguments omit connection_id: '
            + json.dumps(REQUEST) + '. Use crew/request through code execution if that is its '
            'advertised form. Do not discover connections, retry, create grants, or use another '
            'tool. If using code execution, pass the returned value directly to record_result; '
            'do not wrap it in an object or add fields or explanations. '
            'After the tool returns, stop. No message content is authorized.')
        outputs[label], samples[label] = invoke(command, prompt, env, cwd, profile, workdir,
                                               lsof, args.timeout)
    for line in outputs['ordinary'].splitlines():
        if line.strip():
            event = json.loads(line)
            require(not any(item.get('type') in ('toolRequest', 'toolResponse')
                            for item in event.get('message', {}).get('content', [])),
                    'ordinary baseline invoked a tool')
    verdict = validate_trace(outputs['crew'])
    require(owned(profile / 'config/secrets.yaml', mode=0o600) == secrets_before,
            'provider secret file mutated')
    require(owned(profile / 'config/config.yaml', mode=0o600) == config_before, 'provider config mutated')
    verdict.update({'cli_exit': 0, 'listener_positive_control': True,
                    'ordinary_run_baseline': True, 'live_listener_samples': samples['crew'],
                    'ordinary_bridge_listeners': 1, 'unexpected_tcp_listeners': 0,
                    'phase_daemon_or_ssh': False, 'phase_runtime_created': False,
                    'provider_files_unchanged': True, 'baseline_processes_untouched': True})
    return verdict


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cli', required=True)
    parser.add_argument('--profile-root', required=True)
    parser.add_argument('--provider', required=True)
    parser.add_argument('--model', required=True)
    parser.add_argument('--workdir', required=True)
    parser.add_argument('--timeout', type=int, default=180)
    args = parser.parse_args()
    try:
        verdict = run(args)
        print(json.dumps({'result': 'PASS', **verdict}, sort_keys=True))
    except EvidenceError as error:
        failure = {'result': 'FAIL', 'reason': str(error)}
        if error.schema is not None:
            failure['protocol_schema'] = error.schema
        print(json.dumps(failure, sort_keys=True))
        return 1
    except Exception:
        # Native output and exception values can contain profile or provider details.
        print(json.dumps({'result': 'FAIL', 'reason': 'standalone evidence or ownership check failed'}))
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
