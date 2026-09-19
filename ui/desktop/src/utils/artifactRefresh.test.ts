import { describe, expect, it } from 'vitest';
import type { Message } from '../api';
import {
  artifactRefreshEvents,
  artifactRefreshTarget,
  refreshEventMatches,
} from './artifactRefresh';

function exchange(
  id: string,
  name = 'developer__text_editor',
  args: unknown = { command: 'write', path: '/tmp/result.txt' },
  result: unknown = { content: [] }
): Message[] {
  return [
    {
      role: 'assistant',
      created: 0,
      metadata: { agentVisible: true, userVisible: true },
      content: [
        {
          type: 'toolRequest',
          id,
          toolCall: { status: 'success', value: { name, arguments: args } },
        },
        { type: 'toolResponse', id, toolResult: { status: 'success', value: result } },
      ],
    },
  ] as Message[];
}

describe('successful artifact invalidation hints', () => {
  it('refreshes an already-open file after a successful editor undo', () => {
    const [event] = artifactRefreshEvents(
      exchange('undo', 'developer__text_editor', { command: 'undo_edit', path: '/tmp/result.txt' }),
      'a'
    );
    expect(refreshEventMatches(event, 'file:/tmp/result.txt')).toBe(true);
    expect(refreshEventMatches(event, 'file:/tmp/unrelated.txt')).toBe(false);
  });
  it('matches a completed write by exact resolved path, never its source line', () => {
    const [event] = artifactRefreshEvents(
      exchange('w', 'developer__text_editor', { command: 'write', path: 'result.txt' }),
      'a',
      '/tmp'
    );
    expect(event.paths).toEqual(['/tmp/result.txt']);
    expect(refreshEventMatches(event, 'file:/tmp/result.txt')).toBe(true);
    expect(refreshEventMatches(event, 'file:/tmp/result.txt.bak')).toBe(false);
    expect(
      artifactRefreshTarget({ kind: 'file', path: '/tmp/result.txt', title: 'Result', line: 42 })
    ).toBe('file:/tmp/result.txt');
  });
  it.each([
    ['computercontroller__docx_tool', 'update_doc', '/tmp/report.docx'],
    ['computercontroller__xlsx_tool', 'save', '/tmp/budget.xlsx'],
  ])('refreshes a matching office file after %s', (name, operation, path) => {
    const [event] = artifactRefreshEvents(exchange('office', name, { operation, path }), 'a');
    expect(event?.paths).toEqual([path]);
    expect(refreshEventMatches(event, `file:${path}`)).toBe(true);
  });
  it.each([{ isError: true }, { is_error: true }])('ignores unsuccessful results %j', (error) => {
    expect(artifactRefreshEvents(exchange('w', undefined, undefined, error), 'a')).toEqual([]);
  });
  it('ignores proposed/failed requests, pending/failed responses, and read-only tools', () => {
    const messages = exchange('w');
    messages[0].content.pop();
    expect(artifactRefreshEvents(messages, 'a')).toEqual([]);
    const failed = exchange('w');
    (failed[0].content[1] as { toolResult: unknown }).toolResult = {
      status: 'error',
      error: 'synthetic',
    };
    expect(artifactRefreshEvents(failed, 'a')).toEqual([]);
    const proposed = exchange('w');
    (proposed[0].content[0] as { toolCall: unknown }).toolCall = {
      status: 'error',
      error: 'incomplete',
    };
    expect(artifactRefreshEvents(proposed, 'a')).toEqual([]);
    expect(
      artifactRefreshEvents(
        exchange('r', 'developer__text_editor', { command: 'view', path: '/tmp/result.txt' }),
        'a'
      )
    ).toEqual([]);
  });
  it('ignores foreign-session provenance and deduplicates mirrored results', () => {
    const foreign = exchange('w');
    foreign[0].metadata.provenance = { fromSessionId: 'other' } as NonNullable<
      Message['metadata']['provenance']
    >;
    expect(artifactRefreshEvents(foreign, 'a')).toEqual([]);
    expect(artifactRefreshEvents([...exchange('w'), ...exchange('w')], 'a')).toHaveLength(1);
  });
  it.each([
    ['code_execution__execute_code', { code: 'if (false) write_file("/tmp/never.txt")' }],
    ['computercontroller__press_key', { app: 'TextEdit', key: 'cmd+s' }],
    ['computercontroller__click', { app: 'TextEdit', element_index: 4 }],
  ])(
    'treats successful opaque %s as an active-file check, not invented path evidence',
    (name, args) => {
      const events = artifactRefreshEvents(exchange('w', name, args), 'a');
      expect(events).toEqual([{ id: 'w', paths: [], checkActiveFile: true }]);
      expect(refreshEventMatches(events[0], 'file:/tmp/result.txt')).toBe(true);
      expect(refreshEventMatches(events[0], 'app:qa')).toBe(false);
    }
  );
  it.each([{ isError: true }, { is_error: true }])(
    'does not refresh after a failed native action: %j',
    (error) => {
      expect(
        artifactRefreshEvents(
          exchange(
            'failed-script',
            'computercontroller__click',
            { app: 'TextEdit', element_index: 4 },
            { content: [], ...error }
          ),
          'a'
        )
      ).toEqual([]);
    }
  );
  it('keeps native action completion hints local, deduplicated and completion-only', () => {
    const completed = exchange('native', 'computercontroller__press_key', {
      app: 'TextEdit',
      key: 'cmd+s',
    });
    const pending = structuredClone(completed);
    pending[0].content.pop();
    expect(artifactRefreshEvents(pending, 'a')).toEqual([]);
    const failedTransport = structuredClone(completed);
    (failedTransport[0].content[1] as { toolResult: unknown }).toolResult = {
      status: 'error',
      error: 'synthetic failure',
    };
    expect(artifactRefreshEvents(failedTransport, 'a')).toEqual([]);
    const foreign = structuredClone(completed);
    foreign[0].metadata.provenance = { fromSessionId: 'other' } as NonNullable<
      Message['metadata']['provenance']
    >;
    expect(artifactRefreshEvents(foreign, 'a')).toEqual([]);
    expect(artifactRefreshEvents([...completed, ...completed], 'a')).toEqual([
      { id: 'native', paths: [], checkActiveFile: true },
    ]);
  });
  it('does not refresh files for desktop observations or unrelated tools with action-like names', () => {
    for (const tool of [
      'computercontroller__list_apps',
      'computercontroller__get_app_state',
      'computercontroller__screen_capture',
      'unrelated__click',
    ]) {
      expect(artifactRefreshEvents(exchange('observe', tool, { app: 'TextEdit' }), 'a')).toEqual(
        []
      );
    }
  });
  it('refreshes the matching built app, not unrelated apps or unbundled source edits', () => {
    const [build] = artifactRefreshEvents(
      exchange('b', 'agent_drafter__build_app', { id: 'qa' }),
      'a'
    );
    expect(refreshEventMatches(build, 'app:qa')).toBe(true);
    expect(refreshEventMatches(build, 'app:other')).toBe(false);
    expect(
      artifactRefreshEvents(
        exchange('src', 'agent_drafter__update_app', { id: 'qa', path: 'src/main.ts' }),
        'a'
      )
    ).toEqual([]);
    for (const path of [
      undefined,
      'index.html',
      'manifest.json',
      'dist/app.js',
      'assets/icon.svg',
    ]) {
      expect(
        artifactRefreshEvents(
          exchange('visible', 'agent_drafter__update_app', { id: 'qa', path }),
          'a'
        )[0]?.appId
      ).toBe('qa');
    }
  });
  it('never identifies a remote webpage as a managed refresh target', () => {
    expect(
      artifactRefreshTarget({
        kind: 'externalUrl',
        title: 'App',
        url: 'http://127.0.0.1:1234/apps/qa/',
      })
    ).toBe('app:qa');
    for (const url of [
      'https://example.org/apps/qa/',
      'http://localhost:1234/apps/qa/',
      'http://127.0.0.1:1234/config',
      'http://127.0.0.1:1234/apps/qa/dist/app.js',
    ]) {
      expect(artifactRefreshTarget({ kind: 'externalUrl', title: 'Web', url })).toBeNull();
    }
  });
});
