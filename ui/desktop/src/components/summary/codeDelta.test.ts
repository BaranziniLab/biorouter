import { describe, expect, it } from 'vitest';
import type { Message } from '../../api';
import { collectCodeDelta, editorCallsDelta, lineDelta } from './codeDelta';
import cases from './codeDeltaCases.json';

type Content = Message['content'][number];

const message = (role: Message['role'], content: Content[]): Message => ({
  id: crypto.randomUUID(),
  role,
  created: 1,
  metadata: { userVisible: true, agentVisible: true },
  content,
});

const request = (id: string, name: string, args: Record<string, unknown>): Content => ({
  type: 'toolRequest',
  id,
  toolCall: { status: 'success', value: { name, arguments: args } },
});

const response = (
  id: string,
  { isError = false, meta, text = 'ok' }: { isError?: boolean; meta?: unknown; text?: string } = {}
): Content => ({
  type: 'toolResponse',
  id,
  toolResult: {
    status: 'success',
    value: {
      content: [{ type: 'text', text }],
      isError,
      ...(meta === undefined ? {} : { _meta: meta }),
    },
  },
});

const text = (role: Message['role'], value: string) =>
  message(role, [{ type: 'text', text: value }]);

/** One tool call and its answer, as two transcript rows. */
const call = (
  id: string,
  name: string,
  args: Record<string, unknown>,
  answer?: Parameters<typeof response>[1]
): Message[] => [
  message('assistant', [request(id, name, args)]),
  message('user', [response(id, answer)]),
];

// The file the final check's chat wrote from a Code Execution script
// (session 20261012_6): ten lines, one of them blank.
const NOTES =
  '# Final check notes\n\n| Finding | Value |\n|---|---:|\n| Working directory | `/tmp/home` |\n' +
  '| Shell command used | `ls -la` |\n| Entries listed | 2 |\n| Hidden entries | 2 |\n' +
  '| Visible entries | 0 |\n| First entries seen | ., .. |\n';

describe('collectCodeDelta', () => {
  it('counts a file a Code Execution script wrote (the final check: Code stayed +0 −0)', () => {
    const messages = call(
      'script',
      'code_execution__execute_code',
      { code: 'const md = "…";\ntext_editor({ command: "write", path, file_text: md });' },
      {
        text: 'Result: {"writeResult":"Successfully wrote to /tmp/home/final-check-notes.md"}',
        meta: {
          'biorouter/tool-calls': [
            {
              tool: 'developer__shell',
              args: JSON.stringify({ command: 'ls -la', working_directory: '/tmp/home' }),
              status: 'ok',
              result_bytes: 101,
            },
            {
              tool: 'developer__text_editor',
              args: JSON.stringify({
                command: 'write',
                path: '/tmp/home/final-check-notes.md',
                file_text: NOTES,
              }),
              status: 'ok',
              result_bytes: 84,
            },
            {
              tool: 'developer__text_editor',
              args: JSON.stringify({ command: 'view', path: '/tmp/home/final-check-notes.md' }),
              status: 'ok',
              result_bytes: 300,
            },
          ],
        },
      }
    );

    expect(collectCodeDelta(messages)).toEqual({ added: 10, removed: 0 });
  });

  it('takes the script total the daemon measured over the records it truncated or dropped', () => {
    // A record's `args` is cut at 2 KB, so a real file's write is not JSON any
    // more; the daemon measures every edit from the full arguments instead.
    const truncated =
      JSON.stringify({ command: 'write', path: '/p/big.ts', file_text: 'x\n'.repeat(4000) }).slice(
        0,
        2048
      ) + '…';
    const messages = call(
      'script',
      'code_execution__execute_code',
      { code: '…' },
      {
        meta: {
          'biorouter/tool-calls': [
            { tool: 'developer__text_editor', args: truncated, status: 'ok' },
          ],
          'biorouter/tool-calls-dropped': 7,
          'biorouter/code-delta': { added: 4012, removed: 3 },
        },
      }
    );

    expect(collectCodeDelta(messages)).toEqual({ added: 4012, removed: 3 });
  });

  it('still counts the edits a script made before it failed', () => {
    const messages = call(
      'script',
      'code_execution__execute_code',
      { code: '…' },
      {
        isError: true,
        text: 'Error: boom',
        meta: { 'biorouter/code-delta': { added: 2, removed: 1 } },
      }
    );

    expect(collectCodeDelta(messages)).toEqual({ added: 2, removed: 1 });
  });

  it('counts nothing from text that only looks like a diff (the /bug chat read +0 −9)', () => {
    const messages = [
      text('user', '/bug the Code count stayed at +0 -0 even though this chat wrote a file'),
      ...call(
        'grep',
        'developer__shell',
        { command: 'git diff --stat && git show HEAD' },
        {
          text:
            ' 2 files changed, 3 insertions(+), 9 deletions(-)\n' +
            'diff --git a/x.ts b/x.ts\n--- a/x.ts\n+++ b/x.ts\n@@ -1,2 +1,2 @@\n-old\n+new\n' +
            '*** Begin Patch\n+added\n-removed\n',
        }
      ),
      ...call(
        'view',
        'developer__text_editor',
        { command: 'view', path: '/p/BaseChat.tsx' },
        {
          text: '```ts\nconst compactDiffRe = /\\+([0-9])\\s+[-−]([0-9])/g;\n```',
        }
      ),
      text(
        'assistant',
        'The rail showed +0 −9 although nothing changed.\n\n```ts\nconst a = 1;\nconst b = 2;\n```\n'
      ),
    ];

    expect(collectCodeDelta(messages)).toEqual({ added: 0, removed: 0 });
  });

  it('counts a direct edit only once its call has succeeded', () => {
    const write = { command: 'write', path: '/p/a.md', file_text: 'one\ntwo\n' };
    const messages = [
      ...call('failed', 'developer__text_editor', write, { isError: true }),
      // A call still running has no answer yet.
      message('assistant', [request('running', 'developer__text_editor', write)]),
      ...call('landed', 'developer__text_editor', write),
    ];

    expect(collectCodeDelta(messages)).toEqual({ added: 2, removed: 0 });
  });

  it('does not count a failed sub-call inside a script', () => {
    const messages = call(
      'script',
      'code_execution__execute_code',
      { code: '…' },
      {
        meta: {
          'biorouter/tool-calls': [
            {
              tool: 'developer__text_editor',
              args: JSON.stringify({
                command: 'insert',
                path: '/p/a',
                insert_line: 0,
                new_str: 'x\ny',
              }),
              status: 'error',
              error: 'no such file',
            },
          ],
        },
      }
    );

    expect(collectCodeDelta(messages)).toEqual({ added: 0, removed: 0 });
  });

  it('pairs an undo with the last edit of the same file, across direct calls', () => {
    const messages = [
      ...call('w', 'developer__text_editor', {
        command: 'write',
        path: '/p/a',
        file_text: 'a\nb\nc\n',
      }),
      ...call('e', 'developer__text_editor', {
        command: 'str_replace',
        path: '/p/b',
        old_str: 'x',
        new_str: 'y',
      }),
      ...call('u', 'developer__text_editor', { command: 'undo_edit', path: '/p/a' }),
    ];

    expect(collectCodeDelta(messages)).toEqual({ added: 4, removed: 4 });
  });
});

describe('lineDelta', () => {
  it('counts a huge replacement whole instead of diffing it (the daemon does the same)', () => {
    const before = Array.from({ length: 2000 }, (_, i) => `a${i}\n`).join('');
    const after = Array.from({ length: 2000 }, (_, i) => `b${i}\n`).join('');
    expect(lineDelta(before, after)).toEqual({ added: 2000, removed: 2000 });
  });
});

describe('editorCallsDelta (the cases the daemon’s tally must agree with)', () => {
  for (const testCase of cases as Array<{
    name: string;
    calls: Array<Record<string, unknown>>;
    added: number;
    removed: number;
  }>) {
    it(testCase.name, () => {
      expect(editorCallsDelta(testCase.calls)).toEqual({
        added: testCase.added,
        removed: testCase.removed,
      });
    });
  }
});
