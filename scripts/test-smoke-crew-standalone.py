#!/usr/bin/env python3
"""Exercise the refusal boundary without invoking a provider or reading credentials."""

import copy
import importlib.util
import json
import os
import signal
import subprocess
import sys
import time
from unittest.mock import patch
from pathlib import Path
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location('crew_smoke', Path(__file__).with_name('smoke-crew-standalone.py'))
SMOKE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SMOKE)


def events(nested=False):
    arguments = copy.deepcopy(SMOKE.REQUEST)
    name = 'crew__request'
    value = {'content': [{'type': 'text', 'text': SMOKE.DENIAL}], 'isError': False}
    if nested:
        name = 'code_execution__execute_code'
        arguments = {'code': 'request through the advertised runtime transport',
                     'tool_graph': [{'tool': 'crew/request', 'description': 'synthetic refusal', 'depends_on': []}]}
        value['_meta'] = {'biorouter/tool-calls': [{'tool': 'crew__request', 'args': json.dumps(SMOKE.REQUEST),
                                                  'status': 'ok', 'result_bytes': len(SMOKE.DENIAL)}]}
        value['content'][0]['text'] = '<tool-output source="crew">\nResult: ' + json.dumps(SMOKE.DENIAL) + '\n</tool-output>'
    request = {'type': 'toolRequest', 'id': 'synthetic-call',
               'toolCall': {'status': 'success', 'value': {'name': name, 'arguments': arguments}}}
    response = {'type': 'toolResponse', 'id': 'synthetic-call',
                'toolResult': {'status': 'success', 'value': value}}
    return [{'message': {'role': 'assistant', 'content': [request]}},
            {'message': {'role': 'user', 'content': [response]}}]


def trace(data):
    return '\n'.join(json.dumps(event) for event in data)


class RefusalBoundary(unittest.TestCase):
    def test_direct_and_nested_runtime_transports(self):
        for nested in (False, True):
            with self.subTest(nested=nested):
                verdict = SMOKE.validate_trace(trace(events(nested)))
                self.assertTrue(verdict['exact_denial'])
                self.assertFalse(verdict['channel_content_returned'])

    def test_one_local_planning_summary_cannot_carry_channel_content(self):
        plan = [
            {'message': {'role': 'assistant', 'content': [{'type': 'toolRequest', 'id': 'local-plan',
                'toolCall': {'status': 'success', 'value': {'name': 'todo__todo_write',
                    'arguments': {'content': '- [ ] Make one synthetic Crew refusal request'}}}}]}},
            {'message': {'role': 'user', 'content': [{'type': 'toolResponse', 'id': 'local-plan',
                'toolResult': {'status': 'success', 'value': {'isError': False,
                    'content': [{'type': 'text', 'text': '<tool-output>Todo list set: 1 item(s)</tool-output>'}]}}}]}}
        ]
        self.assertEqual(SMOKE.validate_trace(trace(plan + events(True)))['local_planning_calls'], 1)
        mutations = {
            'planning-after-Crew': lambda p: events(True) + p,
            'planning-extra-arguments': lambda p: (p[0]['message']['content'][0]['toolCall']['value']['arguments'].update(connection_id='unexpected') or p + events(True)),
            'planning-content-result': lambda p: (p[1]['message']['content'][0]['toolResult']['value']['content'][0].update(text='private channel message') or p + events(True)),
            'planning-nested-Crew': lambda p: (p[1]['message']['content'][0]['toolResult']['value'].update(_meta={'biorouter/tool-calls': [{'tool': 'crew__request'}]}) or p + events(True)),
            'planning-resource': lambda p: (p[1]['message']['content'][0]['toolResult']['value']['content'].append({'type': 'resource'}) or p + events(True)),
        }
        for label, mutate in mutations.items():
            with self.subTest(boundary=label):
                with self.assertRaises(SMOKE.EvidenceError):
                    SMOKE.validate_trace(trace(mutate(copy.deepcopy(plan))))

    def test_recorded_mcp_object_must_contain_only_the_exact_refusal(self):
        mcp = {'isError': False, 'content': [{'type': 'text', 'text': SMOKE.DENIAL}]}
        data = events(True)
        data[1]['message']['content'][0]['toolResult']['value']['content'][0]['text'] = (
            '<tool-output>Result: ' + json.dumps(mcp) + '</tool-output>')
        self.assertTrue(SMOKE.validate_trace(trace(data))['exact_denial'])
        mutations = {
            'object-resource': lambda m: m['content'].append({'type': 'resource'}),
            'object-messages': lambda m: m.update(messages=['private']),
            'object-nested-metadata': lambda m: m.update(_meta={'content': ['private']}),
            'object-is-error': lambda m: m.update(isError=True),
            'object-hidden-content-key': lambda m: m['content'][0].update(messages=['private']),
        }
        for label, mutate in mutations.items():
            with self.subTest(boundary=label):
                candidate = copy.deepcopy(mcp)
                mutate(candidate)
                data[1]['message']['content'][0]['toolResult']['value']['content'][0]['text'] = (
                    '<tool-output>Result: ' + json.dumps(candidate) + '</tool-output>')
                with self.assertRaises(SMOKE.EvidenceError):
                    SMOKE.validate_trace(trace(data))

    def test_known_optional_mcp_defaults_and_private_schema_diagnostics(self):
        result = {'content': [{'type': 'text', 'text': SMOKE.DENIAL}],
                  'isError': None, 'structuredContent': None, '_meta': None}
        data = events(True)
        data[1]['message']['content'][0]['toolResult']['value']['content'][0]['text'] = (
            '<tool-output>Result: ' + json.dumps(result) + '</tool-output>')
        self.assertTrue(SMOKE.validate_trace(trace(data))['exact_denial'])
        result['structuredContent'] = {'messages': ['private']}
        data[1]['message']['content'][0]['toolResult']['value']['content'][0]['text'] = (
            '<tool-output>Result: ' + json.dumps(result) + '</tool-output>')
        with self.assertRaises(SMOKE.EvidenceError):
            SMOKE.validate_trace(trace(data))
        schema = json.dumps(SMOKE.protocol_schema({'private-dynamic-key': 'private-value',
                'content': [{'type': 'text', 'text': 'private-text'}], 'isError': False}))
        for private in ('private-dynamic-key', 'private-value', 'private-text'):
            self.assertNotIn(private, schema)
        self.assertIn('unknown_field_count', schema)

    def test_runtime_result_must_decode_to_only_the_exact_denial(self):
        for text in ('Result: ' + json.dumps({'message': SMOKE.DENIAL}),
                     'Result: ' + json.dumps(SMOKE.DENIAL) + ' trailing message',
                     'Result: ' + json.dumps(SMOKE.DENIAL + ' private message')):
            with self.subTest(text_shape=text[:12]):
                data = events(True)
                data[1]['message']['content'][0]['toolResult']['value']['content'][0]['text'] = (
                    '<tool-output>' + text + '</tool-output>')
                with self.assertRaises(SMOKE.EvidenceError):
                    SMOKE.validate_trace(trace(data))

    def test_assistant_prose_is_not_tool_content(self):
        data = events(True)
        data.append({'message': {'role': 'assistant', 'content': [
            {'type': 'text', 'text': 'The channel request was refused; no message content was returned.'}]}})
        self.assertTrue(SMOKE.validate_trace(trace(data))['exact_denial'])

    def test_returned_payloads_and_transport_ambiguity_are_rejected(self):
        mutations = {
            'response-id': lambda d: d[1]['message']['content'][0].update(id='another-call'),
            'second-call': lambda d: d[0]['message']['content'].append(copy.deepcopy(d[0]['message']['content'][0])),
            'missing-response': lambda d: d.pop(),
            'different-channel': lambda d: d[1]['message']['content'][0]['toolResult']['value']['_meta']['biorouter/tool-calls'][0].update(args=json.dumps({'method': 'messages.history', 'params': {'channel_id': 'other', 'limit': 1}})),
            'grant-argument': lambda d: d[1]['message']['content'][0]['toolResult']['value']['_meta']['biorouter/tool-calls'][0].update(args=json.dumps({**SMOKE.REQUEST, 'connection_id': 'unexpected'})),
            'extra-nested-call': lambda d: d[1]['message']['content'][0]['toolResult']['value']['_meta']['biorouter/tool-calls'].append({'tool': 'crew__connections', 'args': '{}', 'status': 'ok'}),
            'wrong-graph': lambda d: d[0]['message']['content'][0]['toolCall']['value']['arguments']['tool_graph'][0].update(tool='crew/connections'),
            'extra-graph': lambda d: d[0]['message']['content'][0]['toolCall']['value']['arguments']['tool_graph'].append({'tool': 'crew/request'}),
            'missing-metadata': lambda d: d[1]['message']['content'][0]['toolResult']['value'].pop('_meta'),
            'nested-error': lambda d: d[1]['message']['content'][0]['toolResult']['value']['_meta']['biorouter/tool-calls'][0].update(status='error'),
            'transport-error': lambda d: d[1]['message']['content'][0]['toolResult'].update(status='error'),
            'mcp-error': lambda d: d[1]['message']['content'][0]['toolResult']['value'].update(isError=True),
            'hidden-messages': lambda d: d[1]['message']['content'][0]['toolResult']['value'].update(structuredContent={'messages': ['private']}),
            'extra-text': lambda d: d[1]['message']['content'][0]['toolResult']['value']['content'].append({'type': 'text', 'text': 'private message'}),
            'resource': lambda d: d[1]['message']['content'][0]['toolResult']['value']['content'].append({'type': 'resource', 'resource': {'text': 'private'}}),
            'message-after-denial': lambda d: d[1]['message']['content'][0]['toolResult']['value']['content'][0].update(text=SMOKE.DENIAL + '\nprivate message'),
            'xml-child': lambda d: d[1]['message']['content'][0]['toolResult']['value']['content'][0].update(text='<tool-output>' + SMOKE.DENIAL + '<message>private</message></tool-output>'),
        }
        for label, mutate in mutations.items():
            with self.subTest(boundary=label):
                data = events(True)
                mutate(data)
                with self.assertRaises(SMOKE.EvidenceError):
                    SMOKE.validate_trace(trace(data))

    def test_only_the_single_ordinary_cli_loopback_bridge_is_allowed(self):
        valid = 'p123\nf9\nn127.0.0.1:43210\nTST=LISTEN\nTQR=0\nTQS=0\n'
        self.assertEqual(SMOKE.ordinary_bridge_listeners(valid, 123), {'127.0.0.1:43210'})
        self.assertEqual(SMOKE.ordinary_bridge_listeners('', 123), set())
        invalid = {
            'child-owner': valid.replace('p123', 'p124'),
            'nonloopback': valid.replace('127.0.0.1', '0.0.0.0'),
            'ipv6-unexpected': valid.replace('127.0.0.1', '[::1]'),
            'second-listener': valid + 'f10\nn127.0.0.1:43211\nTST=LISTEN\n',
            'human-header': 'COMMAND PID USER FD TYPE DEVICE SIZE/OFF NODE NAME\n' + valid,
            'missing-state': valid.replace('TST=LISTEN\n', ''),
            'invalid-port': valid.replace('43210', '0'),
            'different-state': valid.replace('LISTEN', 'ESTABLISHED'),
        }
        for boundary, fields in invalid.items():
            with self.subTest(boundary=boundary):
                with self.assertRaises(SMOKE.EvidenceError):
                    SMOKE.ordinary_bridge_listeners(fields, 123)

    def test_invoke_observes_the_exact_cli_and_collects_native_output(self):
        class NativeProcess:
            pid = 987654
            returncode = 0
            polls = 0

            def poll(self):
                self.polls += 1
                return None if self.polls <= 2 else self.returncode

            def communicate(self, prompt):
                if prompt != b'synthetic request':
                    raise AssertionError('stdin was not the supplied synthetic prompt')
                return trace(events(True)).encode(), b''

        process = NativeProcess()
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            protected = root / 'synthetic-secret'
            protected.write_text('synthetic only')
            protected.chmod(0o600)
            before = SMOKE.owned(protected, mode=0o600)
            fields = 'p987654\nf9\nn127.0.0.1:43210\nTST=LISTEN\n'
            with patch.object(SMOKE.subprocess, 'Popen', return_value=process) as launch, \
                    patch.object(SMOKE, 'processes', side_effect=[{}, {process.pid: (123, 'biorouter')}, {}]), \
                    patch.object(SMOKE, 'observe_listeners', return_value=fields) as observe, \
                    patch.object(SMOKE.time, 'sleep'), patch.object(SMOKE.os, 'killpg') as kill:
                output, samples = SMOKE.invoke(['exact-native-cli'], 'synthetic request',
                                               {'HOME': str(root)}, root, root, root,
                                               'exact-observer', 10)
                self.assertEqual(samples, 1)
                self.assertTrue(SMOKE.validate_trace(output)['exact_denial'])
                self.assertEqual(launch.call_args.args[0], ['exact-native-cli'])
                self.assertTrue(launch.call_args.kwargs['start_new_session'])
                self.assertEqual(launch.call_args.kwargs['env'], {'HOME': str(root)})
                observe.assert_called_once_with('exact-observer', {process.pid})
                kill.assert_not_called()
            for log in ('native-trace.jsonl', 'native-diagnostics.log'):
                SMOKE.owned(root / log, mode=0o600)
            self.assertEqual(SMOKE.owned(protected, mode=0o600), before)

    def test_cleanup_group_rejects_existing_or_foreign_live_processes(self):
        uid = os.getuid()
        snapshot = subprocess.CompletedProcess([], 0, stdout=f'100 100 {uid} Z\n101 100 {uid} S\n', stderr='')
        with patch.object(SMOKE.subprocess, 'run', return_value=snapshot):
            self.assertEqual(SMOKE.live_group_members(100, {}), {101})
            with self.assertRaises(SMOKE.EvidenceError):
                SMOKE.live_group_members(100, {101: ('baseline',)})
        snapshot.stdout = f'101 100 {uid + 1} S\n'
        with patch.object(SMOKE.subprocess, 'run', return_value=snapshot):
            with self.assertRaises(SMOKE.EvidenceError):
                SMOKE.live_group_members(100, {})

    def test_failure_cleans_descendant_after_its_cli_parent_exits(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            fixture = root / 'exit-with-child.py'
            fixture.write_text(
                "import os, pathlib, subprocess, sys\n"
                "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'])\n"
                "pathlib.Path('owned-pids').write_text(str(os.getpid()) + ' ' + str(child.pid))\n")
            pids = None
            try:
                with patch.object(SMOKE, 'observe_listeners', return_value=''):
                    with self.assertRaises(SMOKE.EvidenceError):
                        SMOKE.invoke([sys.executable, str(fixture)], '',
                                     {'HOME': str(root), 'PATH': os.defpath}, root, root, root,
                                     'unused-observer', 10)
                pids = [int(value) for value in (root / 'owned-pids').read_text().split()]
                deadline = time.monotonic() + 3
                while time.monotonic() < deadline:
                    status = subprocess.run(['/bin/ps', '-o', 'stat=', '-p', str(pids[1])],
                                            capture_output=True, text=True, check=False)
                    if status.returncode != 0 or status.stdout.strip().startswith('Z'):
                        break
                    time.sleep(0.05)
                else:
                    self.fail('owned descendant survived a failed invocation after parent exit')
            finally:
                if pids is None and (root / 'owned-pids').exists():
                    pids = [int(value) for value in (root / 'owned-pids').read_text().split()]
                if pids:
                    try:
                        status = subprocess.run(['/bin/ps', '-o', 'pgid=,uid=,stat=',
                                                 '-p', str(pids[1])],
                                                capture_output=True, text=True, check=False)
                        fields = status.stdout.split()
                        if fields and int(fields[0]) == pids[0] and int(fields[1]) == os.getuid() and not fields[2].startswith('Z'):
                            os.kill(pids[1], signal.SIGKILL)
                    except ProcessLookupError:
                        pass

    def test_unquiet_banner_is_not_accepted_as_json(self):
        with self.assertRaises(json.JSONDecodeError):
            SMOKE.validate_trace('starting chat\n' + trace(events()))

    def test_fixture_metadata_rejects_symlinks_and_permissive_files(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            regular = root / 'secret'
            regular.write_text('synthetic')
            regular.chmod(0o600)
            SMOKE.owned(regular, mode=0o600)
            regular.chmod(0o644)
            with self.assertRaises(SMOKE.EvidenceError):
                SMOKE.owned(regular, mode=0o600)
            alias = root / 'alias'
            alias.symlink_to(regular)
            with self.assertRaises(SMOKE.EvidenceError):
                SMOKE.owned(alias)

    def test_runtime_inventory_finds_only_actual_fixture_resources(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / 'owner.lock').touch()
            (root / 'ordinary-report').write_text('biorouterd is mentioned in prose')
            self.assertEqual(SMOKE.runtime_paths(root), {'owner.lock'})


if __name__ == '__main__':
    unittest.main()
