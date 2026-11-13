#!/usr/bin/env python3
"""Exercise the hidden CLI evidence contract with synthetic traces and owned process fixtures."""

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


FIXTURE_RESPONSE = 'CREW_STANDALONE_NO_INVOCATION_OK'
FIXTURE_PROMPT = ('Reply with exactly CREW_STANDALONE_NO_INVOCATION_OK. Do not call any tools. '
                  'Do not make Crew requests, discover connections, use grants, read files '
                  'or channel messages, or add explanations. This asks only for a fixed '
                  'synthetic response; it does not ask you to report which tools you have.')


def message(content, role='assistant'):
    return {'type': 'message', 'message': {'id': 'synthetic-message', 'role': role,
            'created': 0, 'content': content,
            'metadata': {'userVisible': True, 'agentVisible': True, 'pinned': False}}}


def events(*chunks):
    return [message([{'type': 'text', 'text': FIXTURE_PROMPT}], 'user')] + [
        message([{'type': 'text', 'text': chunk}])
            for chunk in (chunks or (FIXTURE_RESPONSE,))] + [
                {'type': 'complete', 'total_tokens': 1}]


def stale_worker_events(nested=False):
    arguments = {'method': 'messages.history', 'params': {
        'channel_id': '00000000-0000-4000-8000-0000000002e5', 'limit': 1}}
    denial = "This chat isn't connected to a Crew channel. To connect it, type /crew in this chat."
    name = 'crew__request'
    value = {'content': [{'type': 'text', 'text': denial}], 'isError': False}
    if nested:
        name = 'code_execution__execute_code'
        actual_arguments = arguments
        arguments = {'code': 'synthetic legacy transport', 'tool_graph': [
            {'tool': 'crew/request', 'description': 'synthetic legacy request', 'depends_on': []}]}
        value['_meta'] = {'biorouter/tool-calls': [
            {'tool': 'crew__request', 'args': json.dumps(actual_arguments),
             'status': 'ok', 'result_bytes': len(denial)}]}
        value['content'][0]['text'] = '<tool-output>Result: ' + json.dumps(denial) + '</tool-output>'
    request = {'type': 'toolRequest', 'id': 'synthetic-call',
               'toolCall': {'status': 'success', 'value': {'name': name, 'arguments': arguments}}}
    response = {'type': 'toolResponse', 'id': 'synthetic-call',
                'toolResult': {'status': 'success', 'value': value}}
    data = events()
    return data[:1] + [message([request]), message([response], 'user')] + data[1:]


def trace(data):
    return '\n'.join(json.dumps(event) for event in data)


class HiddenCliContract(unittest.TestCase):
    def test_fixed_response_and_completion_cover_only_the_named_contract(self):
        verdict = SMOKE.validate_trace(trace(events()))
        self.assertEqual(verdict['contract'], 'hidden_cli_no_invocation_v1')
        self.assertEqual(verdict['tool_request_count'], 0)
        self.assertEqual(verdict['tool_response_count'], 0)
        self.assertTrue(verdict['fixed_response'])
        self.assertTrue(verdict['completed'])
        self.assertFalse(verdict['channel_content_returned'])
        self.assertEqual(verdict['limitations'], {
            'effective_model_roster': 'NOT OBSERVABLE',
            'forged_call_denial': 'NOT OBSERVABLE',
            'separate_rust_evidence_required': True})
        for obsolete in ('exact_arguments', 'response_id_matches', 'exact_denial',
                         'local_planning_calls', 'request_count'):
            self.assertNotIn(obsolete, verdict)

    def test_streamed_text_fragments_and_known_null_text_defaults(self):
        data = events('CREW_STANDALONE_', 'NO_INVOCATION_OK\n')
        data[1]['message']['content'][0].update(annotations=None, _meta=None)
        self.assertTrue(SMOKE.validate_trace(trace(data))['fixed_response'])

    def test_real_shaped_user_prelude_and_null_total_tokens(self):
        data = events()
        data[0]['message']['id'] = '00000000-0000-4000-8000-000000000001'
        data[0]['message']['metadata'].update(
            provenance=None, steerOutcome=None, decisionId=None, decisionSummary=None)
        data[-1]['total_tokens'] = None
        verdict = SMOKE.validate_trace(trace(data))
        self.assertTrue(verdict['fixed_response'])
        self.assertTrue(verdict['completed'])

    def test_no_user_echo_is_valid_for_headless_stream_json(self):
        verdict = SMOKE.validate_trace(trace(events()[1:]))
        self.assertTrue(verdict['fixed_response'])
        self.assertTrue(verdict['completed'])
        self.assertEqual(verdict['tool_request_count'], 0)
        self.assertEqual(verdict['tool_response_count'], 0)
        self.assertEqual(verdict['limitations']['effective_model_roster'], 'NOT OBSERVABLE')
        self.assertEqual(verdict['limitations']['forged_call_denial'], 'NOT OBSERVABLE')

    def test_eleven_assistant_fragments_need_no_user_echo(self):
        chunks = ('CREW', '_', 'STAND', 'ALONE', '_', 'NO', '_', 'IN', 'VO', 'CATION', '_OK')
        self.assertEqual(''.join(chunks), FIXTURE_RESPONSE)
        data = events(*chunks)[1:]
        self.assertEqual(len(data), 12)
        for event in data[:-1]:
            event['message']['metadata'].update(
                provenance=None, steerOutcome=None, decisionId=None, decisionSummary=None)
            event['message']['content'][0].update(annotations=None, _meta=None)
        data[-1]['total_tokens'] = None
        verdict = SMOKE.validate_trace(trace(data))
        self.assertTrue(verdict['fixed_response'])
        self.assertTrue(verdict['completed'])

    def test_no_echo_keeps_the_complete_zero_tool_response_contract(self):
        mutants = {
            'tool-request': lambda d: d[0]['message']['content'].append(
                {'type': 'toolRequest', 'id': 'synthetic-call', 'toolCall': {
                    'status': 'success', 'value': {'name': 'crew__request', 'arguments': {}}}}),
            'tool-response': lambda d: d[0]['message']['content'].append(
                {'type': 'toolResponse', 'id': 'synthetic-call', 'toolResult': {
                    'status': 'success', 'value': []}}),
            'additional-text': lambda d: d[0]['message']['content'].append(
                {'type': 'text', 'text': 'synthetic additional content'}),
            'text-metadata': lambda d: d[0]['message']['content'][0].update(
                _meta={'synthetic': 'additional content'}),
            'message-extra': lambda d: d[0]['message'].update(tools=[]),
            'message-metadata': lambda d: d[0]['message']['metadata'].update(
                decisionSummary='synthetic additional content'),
            'event-extra': lambda d: d[0].update(messages=[]),
            'missing-completion': lambda d: d.pop(),
            'duplicate-completion': lambda d: d.append(copy.deepcopy(d[-1])),
            'completion-extra': lambda d: d[-1].update(messages=[]),
            'completion-boolean-token-count': lambda d: d[-1].update(total_tokens=True),
            'event-after-completion': lambda d: d.append(copy.deepcopy(d[0])),
            'missing-assistant': lambda d: d.pop(0),
            'empty-response': lambda d: d[0]['message']['content'][0].update(text=''),
            'prefix-response': lambda d: d[0]['message']['content'][0].update(
                text='synthetic prefix ' + FIXTURE_RESPONSE),
            'suffix-response': lambda d: d[0]['message']['content'][0].update(
                text=FIXTURE_RESPONSE + ' synthetic suffix'),
            'wrong-initial-user': lambda d: d.insert(0, message([
                {'type': 'text', 'text': 'different prompt'}], 'user')),
            'late-matching-user': lambda d: d.insert(1, message([
                {'type': 'text', 'text': FIXTURE_PROMPT}], 'user')),
        }
        for label, mutate in mutants.items():
            with self.subTest(no_echo=label):
                data = events()[1:]
                mutate(data)
                with self.assertRaises(SMOKE.EvidenceError):
                    SMOKE.validate_trace(trace(data))
        late_after_empty = [message([{'type': 'text', 'text': ''}]),
                            message([{'type': 'text', 'text': FIXTURE_PROMPT}], 'user')]
        with self.assertRaises(SMOKE.EvidenceError):
            SMOKE.validate_trace(trace(late_after_empty + events()[1:]))
        for kind in ('error', 'aborted', 'notification', 'model_change', 'system'):
            with self.subTest(event=kind):
                data = events()[1:]
                data.insert(0, {'type': kind, 'message': 'synthetic only'})
                with self.assertRaises(SMOKE.EvidenceError):
                    SMOKE.validate_trace(trace(data))

    def test_an_optional_user_echo_must_be_exact_unique_and_initial(self):
        mutants = {
            'different': lambda d: d[0]['message']['content'][0].update(text='different prompt'),
            'trailing-whitespace': lambda d: d[0]['message']['content'][0].update(
                text=FIXTURE_PROMPT + '\n'),
            'leading-whitespace': lambda d: d[0]['message']['content'][0].update(
                text=' ' + FIXTURE_PROMPT),
            'wrong-initial-role': lambda d: d[0]['message'].update(role='assistant'),
            'duplicate': lambda d: d.insert(1, copy.deepcopy(d[0])),
            'later': lambda d: d.insert(2, copy.deepcopy(d[0])),
            'extra-content': lambda d: d[0]['message']['content'].append(
                {'type': 'text', 'text': ''}),
            'nontext': lambda d: d[0]['message']['content'].append({'type': 'toolRequest'}),
        }
        for label, mutate in mutants.items():
            with self.subTest(prelude=label):
                data = events()
                mutate(data)
                with self.assertRaises(SMOKE.EvidenceError):
                    SMOKE.validate_trace(trace(data))

    def test_the_legacy_one_worker_request_contract_is_rejected(self):
        for nested in (False, True):
            with self.subTest(nested=nested):
                with self.assertRaises(SMOKE.EvidenceError):
                    SMOKE.validate_trace(trace(stale_worker_events(nested)))

    def test_no_tool_or_other_content_is_permitted(self):
        for name in ('crew__request', 'crew__read', 'todo__todo_write',
                     'developer__shell', 'code_execution__execute_code'):
            with self.subTest(tool=name):
                content = {'type': 'toolRequest', 'id': 'extra',
                           'toolCall': {'status': 'success',
                                        'value': {'name': name, 'arguments': {}}}}
                data = events()
                data.insert(1, message([content]))
                with self.assertRaises(SMOKE.EvidenceError):
                    SMOKE.validate_trace(trace(data))
        for content_type in ('toolResponse', 'frontendToolRequest', 'toolConfirmationRequest',
                             'actionRequired', 'image', 'resource', 'thinking',
                             'redactedThinking', 'systemNotification'):
            with self.subTest(content_type=content_type):
                data = events()
                data.insert(1, message([{'type': content_type}]))
                with self.assertRaises(SMOKE.EvidenceError):
                    SMOKE.validate_trace(trace(data))

    def test_extra_text_and_payload_fields_are_rejected(self):
        mutants = {
            'additional-text': lambda d: d[1]['message']['content'].append(
                {'type': 'text', 'text': 'synthetic channel content'}),
            'extra-text-field': lambda d: d[1]['message']['content'][0].update(messages=['synthetic']),
            'text-metadata': lambda d: d[1]['message']['content'][0].update(_meta={'messages': ['synthetic']}),
            'annotations': lambda d: d[1]['message']['content'][0].update(annotations={'priority': 1}),
            'message-metadata': lambda d: d[1]['message']['metadata'].update(decisionSummary={'calls': 1}),
            'message-extra-field': lambda d: d[1]['message'].update(tools=['crew__request']),
            'event-extra-field': lambda d: d[1].update(messages=['synthetic']),
            'user-message': lambda d: d[1]['message'].update(role='user'),
            'hidden-message': lambda d: d[1]['message']['metadata'].update(userVisible=False),
            'completion-payload': lambda d: d[-1].update(messages=['synthetic']),
            'completion-boolean': lambda d: d[-1].update(total_tokens=True),
        }
        for label, mutate in mutants.items():
            with self.subTest(boundary=label):
                data = events()
                mutate(data)
                with self.assertRaises(SMOKE.EvidenceError):
                    SMOKE.validate_trace(trace(data))

    def test_completion_and_fixed_content_cannot_be_replaced_by_model_prose(self):
        mutants = [events()[:-1], [], [{'type': 'complete', 'total_tokens': 1}],
                   events() + [events()[-1]], events() + events(),
                   events('No Crew tools are available.'),
                   events('The forged request was refused.'),
                   events('prefix ' + FIXTURE_RESPONSE), events(FIXTURE_RESPONSE + ' suffix')]
        for data in mutants:
            with self.subTest(event_count=len(data)):
                with self.assertRaises(SMOKE.EvidenceError):
                    SMOKE.validate_trace(trace(data))
        for event_type in ('error', 'aborted', 'notification', 'model_change', 'system'):
            with self.subTest(event_type=event_type):
                with self.assertRaises(SMOKE.EvidenceError):
                    SMOKE.validate_trace(trace([{'type': event_type}] + events()))

    def test_duplicate_fields_cannot_hide_a_tool_payload(self):
        hidden = (json.dumps(events()[0]) + '\n'
                  '{"type":"message","message":{"role":"assistant",'
                  '"content":[{"type":"toolRequest"}],'
                  '"content":[{"type":"text","text":"' + FIXTURE_RESPONSE + '"}]}}\n'
                  '{"type":"complete","total_tokens":1}')
        with self.assertRaises(SMOKE.EvidenceError):
            SMOKE.validate_trace(hidden)

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
                if prompt != FIXTURE_PROMPT.encode():
                    raise AssertionError('stdin was not the supplied synthetic prompt')
                return trace(events()[1:]).encode(), b''

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
                output, samples = SMOKE.invoke(['exact-native-cli'], FIXTURE_PROMPT,
                                               {'HOME': str(root)}, root, root, root,
                                               'exact-observer', 10)
                self.assertEqual(samples, 1)
                self.assertTrue(SMOKE.validate_trace(output)['fixed_response'])
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
