#!/usr/bin/env python3
"""Audit a hidden standalone CLI run that completes without invoking any tool."""

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


CONTRACT = 'hidden_cli_no_invocation_v1'
FIXED_RESPONSE = 'CREW_STANDALONE_NO_INVOCATION_OK'
PROMPT = ('Reply with exactly CREW_STANDALONE_NO_INVOCATION_OK. Do not call any tools. '
          'Do not make Crew requests, discover connections, use grants, read files '
          'or channel messages, or add explanations. This asks only for a fixed '
          'synthetic response; it does not ask you to report which tools you have.')
LIMITATIONS = {'effective_model_roster': 'NOT OBSERVABLE',
               'forged_call_denial': 'NOT OBSERVABLE',
               'separate_rust_evidence_required': True}


class EvidenceError(Exception):
    pass


def require(condition, reason):
    if not condition:
        raise EvidenceError(reason)


def unique_fields(pairs):
    result = {}
    for name, value in pairs:
        require(name not in result, 'duplicate structured-output field')
        result[name] = value
    return result


def validate_trace(output):
    chunks = []
    seen_user = False
    completed = False
    for line in output.splitlines():
        if not line.strip():
            continue
        require(not completed, 'event after completion')
        event = json.loads(line, object_pairs_hook=unique_fields)
        require(isinstance(event, dict), 'invalid stream event')
        if event.get('type') == 'complete':
            require(set(event) == {'type', 'total_tokens'}, 'unexpected completion payload')
            tokens = event['total_tokens']
            require(tokens is None or (type(tokens) is int and tokens >= 0),
                    'invalid completion token count')
            completed = True
            continue
        require(event.get('type') == 'message' and set(event) == {'type', 'message'},
                'unexpected stream event')
        message = event['message']
        require(isinstance(message, dict)
                and {'role', 'content'} <= set(message)
                and set(message) <= {'id', 'role', 'created', 'content', 'metadata'}
                and message['role'] in ('user', 'assistant'), 'unexpected message payload')
        require(message.get('id') is None or isinstance(message['id'], str), 'invalid message ID')
        require('created' not in message or type(message['created']) is int,
                'invalid message timestamp')
        metadata = message.get('metadata', {})
        require(isinstance(metadata, dict)
                and set(metadata) <= {'userVisible', 'agentVisible', 'pinned', 'provenance',
                                      'steerOutcome', 'decisionId', 'decisionSummary'},
                'unexpected message metadata')
        require(all(metadata.get(key, True) is True for key in ('userVisible', 'agentVisible'))
                and metadata.get('pinned', False) is False
                and all(metadata.get(key) is None for key in
                        ('provenance', 'steerOutcome', 'decisionId', 'decisionSummary')),
                'unexpected message metadata content')
        content = message['content']
        require(isinstance(content, list) and content, 'missing message content')
        texts = []
        for item in content:
            require(isinstance(item, dict)
                    and {'type', 'text'} <= set(item)
                    and set(item) <= {'type', 'text', 'annotations', '_meta'}
                    and item['type'] == 'text' and isinstance(item['text'], str),
                    'tool call or nontext content is outside this contract')
            require(item.get('annotations') in (None, {}) and item.get('_meta') in (None, {}),
                    'unexpected text payload')
            texts.append(item['text'])
        if message['role'] == 'user':
            require(not seen_user and not chunks and len(texts) == 1 and texts[0] == PROMPT,
                    'unexpected user prompt')
            seen_user = True
        else:
            chunks.extend(texts)
    require(completed, 'missing completion event')
    require(chunks and ''.join(chunks).strip() == FIXED_RESPONSE,
            'missing or additional synthetic response content')
    return {'contract': CONTRACT, 'tool_request_count': 0, 'tool_response_count': 0,
            'fixed_response': True, 'completed': True, 'channel_content_returned': False,
            'limitations': dict(LIMITATIONS)}


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
           'BIOROUTER_TELEMETRY_ENABLED': 'false', 'OLLAMA_HOST': 'http://127.0.0.1:1'}
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
        prompt = PROMPT
        outputs[label], samples[label] = invoke(command, prompt, env, cwd, profile, workdir,
                                               lsof, args.timeout)
    validate_trace(outputs['ordinary'])
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
        print(json.dumps(failure, sort_keys=True))
        return 1
    except Exception:
        # Native output and exception values can contain profile or provider details.
        print(json.dumps({'result': 'FAIL', 'reason': 'standalone evidence or ownership check failed'}))
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
