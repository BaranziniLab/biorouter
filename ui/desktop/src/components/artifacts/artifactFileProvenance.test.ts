import { describe, expect, it } from 'vitest';
import type { Message } from '../../api';
import { resolveFileLink } from './artifactFileLinks';
import {
  filePathLookupBeforeMessage,
  filePathsBeforeMessage,
  referencedFilePaths,
} from './artifactFileProvenance';

describe('file-link reliability: provenance boundaries', () => {
  it('keeps the indexed basename lookup scoped to earlier messages', () => {
    const messages: Message[] = ['/prior/report.md', '/future/report.md'].map((path, index) => ({
      id: String(index),
      role: 'assistant',
      created: index,
      metadata: { userVisible: true, agentVisible: true },
      content: [{ type: 'text', text: `Created \`${path}\`.` }],
    }));
    expect(filePathLookupBeforeMessage(messages, 0, 'local', '/work')('report.md')).toEqual([]);
    expect(filePathLookupBeforeMessage(messages, 1, 'local', '/work')('report.md')).toEqual([
      '/prior/report.md',
    ]);
    expect(filePathLookupBeforeMessage(messages, 2, 'local', '/work')('report.md')).toEqual([
      '/prior/report.md',
      '/future/report.md',
    ]);
  });

  it('uses the same source and literal-name parsing for prose provenance', () => {
    expect(referencedFilePaths('See /work/source.rs#L42. And /work/literal.rs%23L42.')).toEqual([
      '/work/source.rs',
      '/work/literal.rs#L42',
    ]);
    expect(referencedFilePaths('See /work/source.rs:42:7 and /work/source.rs#L42:7.')).toEqual([]);
  });

  it('anchors qualified relatives to the actual workspace, never a matching prior suffix', () => {
    const known = ['/elsewhere/results/report.md', '/work/results/report.md'];
    expect(resolveFileLink('results/report.md', '/work', [known[0]])).toMatchObject({
      kind: 'resolved',
      path: '/work/results/report.md',
    });
    expect(resolveFileLink('results/report.md', '/work', known)).toMatchObject({
      kind: 'resolved',
      path: '/work/results/report.md',
    });
    expect(resolveFileLink('other/report.md', '/work', known)).toMatchObject({
      kind: 'resolved',
      path: '/work/other/report.md',
    });
    expect(resolveFileLink('report.md', '/work', known).kind).toBe('unresolved');
    expect(resolveFileLink('./report.md', '/work', known)).toMatchObject({
      kind: 'resolved',
      path: '/work/report.md',
    });
  });

  it.each([
    'hidden-request',
    'foreign-request',
    'foreign-result',
    'unexecuted-wrapper',
    'quoted-shell',
  ])('does not promote %s to a known written file', (scenario) => {
    const request: Message = {
      id: 'request',
      role: 'assistant',
      created: 1,
      metadata: { userVisible: true, agentVisible: true },
      content: [
        {
          type: 'toolRequest',
          id: 'write',
          toolCall: {
            status: 'success',
            value: {
              name: 'developer__text_editor',
              arguments: { command: 'write', path: '/elsewhere/report.md' },
            },
          },
        },
      ],
    };
    const response: Message = {
      id: 'response',
      role: 'tool',
      created: 2,
      metadata: { userVisible: false, agentVisible: true },
      content: [
        {
          type: 'toolResponse',
          id: 'write',
          toolResult: { status: 'success', value: { is_error: false, content: [] } },
        },
      ],
    };
    if (scenario === 'hidden-request') request.metadata.userVisible = false;
    if (scenario === 'foreign-request') {
      request.metadata.provenance = { kind: 'agent_injection', fromSessionId: 'foreign' };
    }
    if (scenario === 'foreign-result') {
      response.metadata.provenance = { kind: 'agent_injection', fromSessionId: 'foreign' };
    }
    if (scenario === 'unexecuted-wrapper' || scenario === 'quoted-shell') {
      request.content = [
        {
          type: 'toolRequest',
          id: 'write',
          toolCall: {
            status: 'success',
            value:
              scenario === 'unexecuted-wrapper'
                ? {
                    name: 'code_execution__execute_code',
                    arguments: {
                      code: 'if (false) { await developer.text_editor({command:"write",path:"/elsewhere/report.md",file_text:"x"}) }',
                    },
                  }
                : {
                    name: 'developer__shell',
                    arguments: {
                      command: "printf 'echo > /elsewhere/report.md'",
                    },
                  },
          },
        },
      ];
    }
    expect(filePathsBeforeMessage([request, response], 2, 'local', '/work')).toEqual([]);
  });

  // The index is built for EVERY message of both transcript renderers, so it is
  // the first thing a shared transcript from a remote `base_url` reaches — and
  // `message.metadata.provenance` was read there without a guard. Measured in a
  // running dev GUI: a shared transcript whose messages carried no `metadata`
  // replaced the whole page with the app's error boundary. The payload is
  // repaired at the boundary now (`sharedSessions.normalizeSharedMessages`);
  // this pins the module itself as total, so the repair is belt and braces
  // rather than the only thing standing between a reader and a blank page.
  it('indexes nothing, and throws nothing, for a message missing metadata or content', () => {
    const noMetadata = {
      id: 'no-metadata',
      role: 'assistant',
      created: 1,
      content: [{ type: 'text', text: 'Created `/remote/report.md`.' }],
    } as unknown as Message;
    const noContent = {
      id: 'no-content',
      role: 'assistant',
      created: 2,
      metadata: { userVisible: true, agentVisible: true },
    } as unknown as Message;
    const wellFormed: Message = {
      id: 'well-formed',
      role: 'assistant',
      created: 3,
      metadata: { userVisible: true, agentVisible: true },
      content: [{ type: 'text', text: 'Created `/work/kept.md`.' }],
    };

    const messages = [noMetadata, noContent, wellFormed];
    expect(filePathsBeforeMessage(messages, 3, 'local', '/work')).toEqual(['/work/kept.md']);
    expect(filePathLookupBeforeMessage(messages, 3, 'local', '/work')('report.md')).toEqual([]);
  });
});
