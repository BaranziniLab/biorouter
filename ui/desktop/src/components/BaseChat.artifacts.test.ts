import { afterEach, describe, expect, it, vi } from 'vitest';
import { renderHook, waitFor } from '@testing-library/react';
import { readFileSync } from 'node:fs';
import type { Message } from '../api';
import {
  applyMentionedFileGate,
  collectArtifactsFromMessages,
  decideArtifactAutoOpen,
  mentionedArtifactPaths,
  shouldAutoRepairArtifact,
  useSessionArtifacts,
} from './BaseChat';
import type { ArtifactSource } from './artifacts/artifactTypes';
import {
  resetFileLinkStatusForTests,
  type FileLinkExistence,
  type FilePathCheckRequest,
} from './artifacts/fileLinkStatus';
import {
  getArtifactPanelExpansionContentWidth,
  getDefaultArtifactPanelWidth,
} from './artifacts/useArtifactPanel';
import { ChatState } from '../types/chatState';

const visibleMessage = (content: Message['content']): Message => ({
  id: crypto.randomUUID(),
  role: 'assistant',
  created: 1,
  metadata: { userVisible: true, agentVisible: true },
  content,
});

const hiddenToolResponse = (id: string, html: string): Message => ({
  id: crypto.randomUUID(),
  role: 'tool',
  created: 2,
  metadata: { userVisible: false, agentVisible: true },
  content: [
    {
      type: 'toolResponse',
      id,
      toolResult: {
        status: 'success',
        value: {
          is_error: false,
          content: [
            {
              resource: {
                uri: 'ui://chart.html',
                mimeType: 'text/html',
                text: html,
              },
            },
          ],
        },
      },
    },
  ],
});

const writeRequest = (id: string, name: string, args: Record<string, unknown>): Message =>
  visibleMessage([
    {
      type: 'toolRequest',
      id,
      toolCall: { status: 'success', value: { name, arguments: args } },
    },
  ]);

const textToolResponse = (id: string, text: string, isError = false): Message => ({
  id: crypto.randomUUID(),
  role: 'tool',
  created: 2,
  metadata: { userVisible: false, agentVisible: true },
  content: [
    {
      type: 'toolResponse',
      id,
      toolResult: {
        status: 'success',
        value: { is_error: isError, content: [{ type: 'text', text }] },
      },
    },
  ],
});

describe('collectArtifactsFromMessages', () => {
  it('file-link reliability: does not substitute the launch cwd for an unloaded session', () => {
    const source = readFileSync(`${process.cwd()}/src/components/BaseChat.tsx`, 'utf8');
    expect(source).not.toMatch(
      /const\s+sessionWorkingDir\s*=\s*session\?\.working_dir\s*\|\|\s*getInitialWorkingDir\(\)/
    );
  });

  it.each([
    ['/tmp/source.rs#L2', '/tmp/source.rs', 'source.rs'],
    ['/tmp/source.rs:2', '/tmp/source.rs', 'source.rs'],
    ['See /tmp/source.rs#L2.', '/tmp/source.rs', 'source.rs'],
    ['See /tmp/source.rs%23L42.', '/tmp/source.rs#L42', 'source.rs#L42'],
    ['/tmp/Study%20%231/report.md', '/tmp/Study #1/report.md', 'report.md'],
    ['[report](</tmp/Study %231/report.md>)', '/tmp/Study #1/report.md', 'report.md'],
    ['`/tmp/source.rs%23L42`', '/tmp/source.rs#L42', 'source.rs#L42'],
  ])(
    'file-link reliability: auto-artifact collection shares the click path for %s',
    (text, path, title) => {
      expect(
        collectArtifactsFromMessages([visibleMessage([{ type: 'text', text }])], '/work')
      ).toEqual([{ kind: 'file', path, title, mentionedOnly: true }]);
    }
  );

  it('file-link reliability: does not auto-open a guessed root for later shorthand', () => {
    const messages = [
      visibleMessage([{ type: 'text', text: 'Created `/work/run/results/report.md`.' }]),
      visibleMessage([{ type: 'text', text: '[Report](report.md)' }]),
    ];
    expect(collectArtifactsFromMessages(messages, '/work')).toEqual([
      {
        kind: 'file',
        path: '/work/run/results/report.md',
        title: 'report.md',
        mentionedOnly: true,
      },
    ]);
  });

  it('file-link reliability: waits for streaming prose to stabilize before opening its final path once', () => {
    const fileKeys = (artifacts: ReturnType<typeof collectArtifactsFromMessages>) =>
      artifacts.map((artifact) => {
        if (artifact.kind !== 'file') throw new Error('expected a file artifact');
        return `file:${artifact.path}`;
      });
    const streaming = visibleMessage([{ type: 'text', text: 'Created `/tmp/report.js`' }]);
    const partialArtifacts = collectArtifactsFromMessages([streaming], '/work', 0);
    expect(partialArtifacts).toEqual([]);
    expect(
      decideArtifactAutoOpen({
        scanDone: true,
        knownKeys: new Set(),
        reportedMessageCount: 1,
        loadedMessageCount: 1,
        artifactKeys: fileKeys(partialArtifacts),
        gatePending: false,
      })
    ).toEqual({ action: 'none' });

    streaming.content = [{ type: 'text', text: 'Created `/tmp/report.json`' }];
    expect(collectArtifactsFromMessages([streaming], '/work', 0)).toEqual([]);

    const stableArtifacts = collectArtifactsFromMessages([streaming], '/work');
    expect(stableArtifacts).toEqual([
      { kind: 'file', path: '/tmp/report.json', title: 'report.json', mentionedOnly: true },
    ]);
    const artifactKeys = fileKeys(stableArtifacts);
    const firstDecision = decideArtifactAutoOpen({
      scanDone: true,
      knownKeys: new Set(),
      reportedMessageCount: 1,
      loadedMessageCount: 1,
      artifactKeys,
      gatePending: false,
    });
    expect(firstDecision.action).toBe('open');
    if (firstDecision.action !== 'open') throw new Error('unreachable');
    expect(
      decideArtifactAutoOpen({
        scanDone: true,
        knownKeys: firstDecision.knownKeys,
        reportedMessageCount: 1,
        loadedMessageCount: 1,
        artifactKeys,
        gatePending: false,
      })
    ).toEqual({ action: 'none' });
  });

  it('file-link reliability: keeps successful structured artifacts available mid-stream', () => {
    const request = visibleMessage([
      { type: 'text', text: 'Writing `/tmp/report.js`' },
      {
        type: 'toolRequest',
        id: 'stream-write',
        toolCall: {
          status: 'success',
          value: {
            name: 'developer__text_editor',
            arguments: { command: 'write', path: '/tmp/report.json' },
          },
        },
      },
    ]);
    const response = hiddenToolResponse('stream-write', '<p>not a file resource</p>');
    const artifacts = collectArtifactsFromMessages([request, response], '/work', 0);
    expect(artifacts).toContainEqual({
      kind: 'file',
      path: '/tmp/report.json',
      title: 'report.json',
    });
    expect(artifacts).toContainEqual(
      expect.objectContaining({ kind: 'html', sourceUri: 'ui://chart.html' })
    );
    expect(artifacts).not.toContainEqual(expect.objectContaining({ path: '/tmp/report.js' }));
  });

  it('file-link reliability: does not auto-open malformed source locators', () => {
    expect(
      collectArtifactsFromMessages(
        [
          visibleMessage([
            { type: 'text', text: 'See /tmp/source.rs:42:7 and /tmp/source.rs#L42:7.' },
          ]),
        ],
        '/work'
      )
    ).toEqual([]);
  });

  it('collects artifacts from tool responses paired with visible assistant tool requests', () => {
    const messages: Message[] = [
      visibleMessage([
        {
          type: 'toolRequest',
          id: 'tool-1',
          toolCall: {
            status: 'success',
            value: {
              name: 'autovisualiser__show_chart',
              arguments: {},
            },
          },
        },
      ]),
      hiddenToolResponse('tool-1', '<html><body>Chart</body></html>'),
    ];

    const artifacts = collectArtifactsFromMessages(messages);

    expect(artifacts).toHaveLength(1);
    expect(artifacts[0]).toMatchObject({
      kind: 'html',
      title: 'chart.html',
      html: '<html><body>Chart</body></html>',
    });
  });

  it('collects external resource links as click-only preview artifacts', () => {
    const request = visibleMessage([
      {
        type: 'toolRequest',
        id: 'tool-link',
        toolCall: {
          status: 'success',
          value: { name: 'reports__publish', arguments: {} },
        },
      },
    ]);
    const response: Message = {
      id: crypto.randomUUID(),
      role: 'tool',
      created: 2,
      metadata: { userVisible: false, agentVisible: true },
      content: [
        {
          type: 'toolResponse',
          id: 'tool-link',
          toolResult: {
            status: 'success',
            value: {
              is_error: false,
              content: [
                {
                  uri: 'https://example.test/report.html',
                  name: 'report',
                  title: 'Study report',
                },
              ],
            },
          },
        },
      ],
    };

    expect(collectArtifactsFromMessages([request, response])).toEqual([
      {
        kind: 'externalUrl',
        title: 'Study report',
        url: 'https://example.test/report.html',
      },
    ]);
  });

  it('ignores orphaned hidden tool responses without a visible request', () => {
    expect(collectArtifactsFromMessages([hiddenToolResponse('tool-1', '<p>Hidden</p>')])).toEqual(
      []
    );
  });

  it('previews a markdown file the agent wrote', () => {
    const messages: Message[] = [
      writeRequest('t1', 'developer__text_editor', {
        command: 'write',
        path: '/work/report.md',
      }),
      textToolResponse('t1', 'wrote /work/report.md'),
    ];

    expect(collectArtifactsFromMessages(messages, '/work')).toEqual([
      { kind: 'file', title: 'report.md', path: '/work/report.md' },
    ]);
  });

  /**
   * R1-02 — camelCase is the wire spelling: rmcp serialises CallToolResult with
   * `rename_all = "camelCase"`, so a failed call arrives as `isError: true`.
   * Only successful tool responses may contribute artifacts.
   */
  it('ignores files named by a tool call that failed with the camelCase isError flag', () => {
    const failedResponse: Message = {
      id: crypto.randomUUID(),
      role: 'tool',
      created: 2,
      metadata: { userVisible: false, agentVisible: true },
      content: [
        {
          type: 'toolResponse',
          id: 't-camel-err',
          toolResult: {
            status: 'success',
            value: {
              isError: true,
              content: [{ type: 'text', text: 'could not write /work/report.md' }],
            },
          },
        },
      ],
    };
    const messages: Message[] = [
      writeRequest('t-camel-err', 'developer__text_editor', {
        command: 'write',
        path: '/work/report.md',
      }),
      failedResponse,
    ];

    expect(collectArtifactsFromMessages(messages, '/work')).toEqual([]);
  });

  it('does not auto-preview protocol examples or hidden repository metadata', () => {
    const messages = [
      visibleMessage([
        {
          type: 'text',
          text: 'These links require `file://` support. The folder also contains a hidden `.git` directory.',
        },
      ]),
    ];

    expect(collectArtifactsFromMessages(messages, '/Users/ada/Desktop')).toEqual([]);
  });

  it('still discovers a concrete file URI without trailing prose punctuation', () => {
    const messages = [
      visibleMessage([
        { type: 'text', text: 'Open file:///Users/ada/Desktop/results/report.pdf).' },
      ]),
    ];

    expect(collectArtifactsFromMessages(messages)).toEqual([
      {
        kind: 'file',
        title: 'report.pdf',
        path: '/Users/ada/Desktop/results/report.pdf',
        mentionedOnly: true,
      },
    ]);
  });

  it('does not extract web URLs or prefixes of non-artifact paths', () => {
    const messages = [
      visibleMessage([
        {
          type: 'text',
          text: 'Ignore https://example.com/report.pdf, /tmp/report.pdf.bak, and /tmp/report.pdf/notes.',
        },
      ]),
    ];

    expect(collectArtifactsFromMessages(messages)).toEqual([]);
  });

  it('drops a relative prose path when there is no working dir to anchor it', () => {
    // Defect B: keeping the raw `./results/plot.png` let the main process resolve
    // it against the ELECTRON process cwd — previewing whatever folder of that
    // name sat at the app's launch dir. Unanchorable relative paths are dropped.
    const messages = [
      visibleMessage([{ type: 'text', text: 'See ./results/plot.png in the output folder.' }]),
    ];
    expect(collectArtifactsFromMessages(messages)).toEqual([]);
  });

  it('anchors a relative prose path once a working dir is available', () => {
    const messages = [
      visibleMessage([{ type: 'text', text: 'See ./results/plot.png in the output folder.' }]),
    ];
    expect(collectArtifactsFromMessages(messages, '/work')).toEqual([
      { kind: 'file', title: 'plot.png', path: '/work/results/plot.png', mentionedOnly: true },
    ]);
  });

  it('disambiguates same-basename artifacts so the wrong folder is not opened by mistake', () => {
    // Defect C: two different folders/files sharing a basename ("data", "report.pdf")
    // both showed as just the basename — indistinguishable in the tab strip, and a
    // wrong-path preview hides behind the expected name. Widen the colliding labels
    // to the shortest trailing suffix that separates them; the path stays the identity.
    const messages = [
      visibleMessage([
        { type: 'text', text: 'Compare /proj/alpha/report.pdf and /proj/bravo/report.pdf here.' },
      ]),
    ];
    expect(collectArtifactsFromMessages(messages)).toEqual([
      {
        kind: 'file',
        title: 'alpha/report.pdf',
        path: '/proj/alpha/report.pdf',
        mentionedOnly: true,
      },
      {
        kind: 'file',
        title: 'bravo/report.pdf',
        path: '/proj/bravo/report.pdf',
        mentionedOnly: true,
      },
    ]);
  });

  it('widens a disambiguated label further when the parent segment also collides', () => {
    const messages = [
      visibleMessage([{ type: 'text', text: 'Compare /x/data/out.csv and /y/data/out.csv here.' }]),
    ];
    expect(collectArtifactsFromMessages(messages).map((a) => a.title)).toEqual([
      'x/data/out.csv',
      'y/data/out.csv',
    ]);
  });

  it('leaves a unique basename untouched', () => {
    const messages = [
      visibleMessage([{ type: 'text', text: 'Open /proj/alpha/report.pdf now please.' }]),
    ];
    expect(collectArtifactsFromMessages(messages)).toEqual([
      { kind: 'file', title: 'report.pdf', path: '/proj/alpha/report.pdf', mentionedOnly: true },
    ]);
  });

  it('previews an R script and the image its shell run produced', () => {
    const messages: Message[] = [
      writeRequest('t1', 'developer__text_editor', { command: 'write', path: 'analysis.R' }),
      textToolResponse('t1', 'ok'),
      writeRequest('t2', 'developer__shell', { command: 'Rscript analysis.R -o volcano.png' }),
      textToolResponse('t2', 'done'),
    ];

    expect(collectArtifactsFromMessages(messages, '/work')).toEqual([
      { kind: 'file', title: 'analysis.R', path: '/work/analysis.R' },
      { kind: 'file', title: 'volcano.png', path: '/work/volcano.png' },
    ]);
  });

  it('does not crash on a file whose name contains a literal percent sign', () => {
    // Regression: the agent writes a file like "results 100%.csv". basenameFromPath
    // called decodeURIComponent on it, which throws URIError: "URI malformed" on the
    // stray `%`. Because collectArtifactsFromMessages runs during chat render, that
    // throw crashed the whole app into the "Honk!" error boundary mid-task.
    const messages: Message[] = [
      writeRequest('t1', 'developer__text_editor', {
        command: 'write',
        path: '/work/results 100%.csv',
      }),
      textToolResponse('t1', 'ok'),
    ];

    expect(() => collectArtifactsFromMessages(messages, '/work')).not.toThrow();
    expect(collectArtifactsFromMessages(messages, '/work')).toEqual([
      { kind: 'file', title: 'results 100%.csv', path: '/work/results 100%.csv' },
    ]);
  });

  it('does not crash on a ui:// resource whose URI contains a stray percent sign', () => {
    const messages: Message[] = [
      writeRequest('t1', 'autovisualiser__show_chart', {}),
      hiddenToolResponse('t1', '<html><body>Chart</body></html>'),
    ];
    // Force the resource URI to carry an invalid percent-escape.
    (
      messages[1].content[0] as unknown as {
        toolResult: { value: { content: Array<{ resource: { uri: string } }> } };
      }
    ).toolResult.value.content[0].resource.uri = 'ui://charts/effect 50% panel.html';

    expect(() => collectArtifactsFromMessages(messages)).not.toThrow();
  });

  it('does not preview a file whose write failed', () => {
    const messages: Message[] = [
      writeRequest('t1', 'developer__text_editor', { command: 'write', path: '/work/report.md' }),
      textToolResponse('t1', 'permission denied', true),
    ];

    expect(collectArtifactsFromMessages(messages, '/work')).toEqual([]);
  });

  it('does not preview a file the agent only viewed', () => {
    const messages: Message[] = [
      writeRequest('t1', 'developer__text_editor', { command: 'view', path: '/work/report.md' }),
      textToolResponse('t1', '# Report'),
    ];

    expect(collectArtifactsFromMessages(messages, '/work')).toEqual([]);
  });

  it('does not duplicate a file already named in the assistant text', () => {
    const messages: Message[] = [
      visibleMessage([
        { type: 'text', text: 'I saved it to /work/report.md' },
        {
          type: 'toolRequest',
          id: 't1',
          toolCall: {
            status: 'success',
            value: {
              name: 'developer__text_editor',
              arguments: { command: 'write', path: '/work/report.md' },
            },
          },
        },
      ]),
      textToolResponse('t1', 'ok'),
    ];

    const artifacts = collectArtifactsFromMessages(messages, '/work');
    expect(artifacts).toHaveLength(1);
    // …and the surviving entry is no longer a mere mention. The prose was
    // collected first and won the dedupe, but a successful write is a receipt:
    // leaving the flag on would put a file the agent demonstrably created
    // behind an existence check it does not need, and give it the wrong
    // not-found copy if it were later deleted.
    expect(artifacts[0]).toEqual({
      kind: 'file',
      title: 'report.md',
      path: '/work/report.md',
      mentionedOnly: undefined,
    });
  });

  it('resolves relative files named in assistant text from the session working directory', () => {
    const messages = visibleMessage([
      { type: 'text', text: 'Open the generated page at ./dist/index.html' },
    ]);

    expect(collectArtifactsFromMessages([messages], '/work/site')).toEqual([
      {
        kind: 'file',
        title: 'index.html',
        path: '/work/site/dist/index.html',
        mentionedOnly: true,
      },
    ]);
  });

  const dashboardRequest = (id: string): Message =>
    visibleMessage([
      {
        type: 'toolRequest',
        id,
        toolCall: {
          status: 'success',
          value: { name: 'autovisualiser__render_dashboard', arguments: {} },
        },
      },
    ]);

  const dashboardResponse = (id: string, uri: string, html: string): Message => ({
    id: crypto.randomUUID(),
    role: 'tool',
    created: 2,
    metadata: { userVisible: false, agentVisible: true },
    content: [
      {
        type: 'toolResponse',
        id,
        toolResult: {
          status: 'success',
          value: {
            is_error: false,
            content: [{ resource: { uri, mimeType: 'text/html', text: html } }],
          },
        },
      },
    ],
  });

  const userTurn = (text: string): Message => ({
    id: crypto.randomUUID(),
    role: 'user',
    created: 3,
    metadata: { userVisible: true, agentVisible: true },
    content: [{ type: 'text', text }],
  });

  it('collapses a report re-rendered within one turn down to its final version', () => {
    // The model called render_dashboard twice in one turn (a refine): same
    // ui://dashboard/<slug> URI, different bytes. Only the last should surface.
    const messages: Message[] = [
      dashboardRequest('d1'),
      dashboardResponse('d1', 'ui://dashboard/orbit-report', '<html><body>DRAFT</body></html>'),
      dashboardRequest('d2'),
      dashboardResponse(
        'd2',
        'ui://dashboard/orbit-report',
        '<html><body>FINAL — longer, refined report body</body></html>'
      ),
    ];

    const artifacts = collectArtifactsFromMessages(messages);

    expect(artifacts).toHaveLength(1);
    expect(artifacts[0]).toMatchObject({
      kind: 'html',
      html: '<html><body>FINAL — longer, refined report body</body></html>',
    });
  });

  it('keeps a dashboard the user refines in a LATER turn as its own entry', () => {
    const messages: Message[] = [
      dashboardRequest('d1'),
      dashboardResponse(
        'd1',
        'ui://dashboard/orbit-report',
        '<html><body>version one</body></html>'
      ),
      userTurn('make the title bigger'),
      dashboardRequest('d2'),
      dashboardResponse(
        'd2',
        'ui://dashboard/orbit-report',
        '<html><body>version two</body></html>'
      ),
    ];

    expect(collectArtifactsFromMessages(messages)).toHaveLength(2);
  });

  it('keeps two genuinely different dashboards produced in the same turn', () => {
    const messages: Message[] = [
      dashboardRequest('d1'),
      dashboardResponse('d1', 'ui://dashboard/orbit-report', '<html><body>orbit</body></html>'),
      dashboardRequest('d2'),
      dashboardResponse('d2', 'ui://dashboard/thermal-report', '<html><body>thermal</body></html>'),
    ];

    expect(collectArtifactsFromMessages(messages)).toHaveLength(2);
  });
});

describe('getArtifactPanelExpansionContentWidth', () => {
  it('requests enough extra window width for a narrow split pane', () => {
    expect(getArtifactPanelExpansionContentWidth(1000, 980)).toBe(1092);
  });

  it('does not request expansion when the split pane already fits the artifact panel', () => {
    expect(getArtifactPanelExpansionContentWidth(1200, 1100)).toBeNull();
  });
});

describe('getDefaultArtifactPanelWidth', () => {
  it('uses 48% of the available width for a squarer initial preview', () => {
    expect(getDefaultArtifactPanelWidth(1600)).toBe(768);
  });

  it('preserves the panel bounds and minimum chat width', () => {
    expect(getDefaultArtifactPanelWidth(900)).toBe(360);
    expect(getDefaultArtifactPanelWidth(3000)).toBe(920);
  });
});

/**
 * The panel's half of the file-link existence rule (#…): a path the assistant
 * only NAMED must not become a card.
 *
 * The reproduced defect: the assistant wrote, in prose, "writing it to a file
 * (e.g. `~/Desktop/kdps-intent.md`) and telling me the path is more reliable" —
 * a suggestion for a file that had never existed. `referencedFilePaths` pulled
 * it out of the backticks, the panel made a card, opened it in a tab and
 * rendered an error. The SAME extractor already refused to make it a chat link,
 * because only that consumer was hardened.
 *
 * Clicking the card was already denied by the main-process allowlist, so this is
 * not a read hole — it is chrome any model (or a prompt injection reaching one)
 * can put in the user's panel, and a panel full of dead cards is a panel nobody
 * trusts.
 */
describe('applyMentionedFileGate', () => {
  const mentioned = (path: string): ArtifactSource => ({
    kind: 'file',
    title: path.split('/').pop() as string,
    path,
    mentionedOnly: true,
  });
  const written = (path: string): ArtifactSource => ({
    kind: 'file',
    title: path.split('/').pop() as string,
    path,
  });
  const figure: ArtifactSource = { kind: 'html', title: 'chart.html', html: '<p>c</p>' };
  const always = (existence: FileLinkExistence) => (): FileLinkExistence => existence;

  it('drops a mentioned path the main process cannot find', () => {
    const artifacts = [mentioned('/Users/ada/Desktop/kdps-intent.md'), figure];

    expect(applyMentionedFileGate(artifacts, always('missing'))).toEqual([figure]);
  });

  it('keeps a mentioned path that is really on disk, and marks it confirmed', () => {
    // Clearing the flag is what earns the honest not-found copy later: a file
    // seen on disk and gone at read time really was moved or deleted.
    expect(applyMentionedFileGate([mentioned('/work/report.md')], always('present'))).toEqual([
      { kind: 'file', title: 'report.md', path: '/work/report.md', mentionedOnly: undefined },
    ]);
  });

  it('shows nothing while the answer is still in flight', () => {
    // The link path's rule, unchanged: a dead path is never a card, not even for
    // ONE frame. A card that appears and vanishes a tick later is the same bug
    // on a shorter timescale, so a confirmed hit UPGRADES rather than a hit
    // being walked back.
    expect(applyMentionedFileGate([mentioned('/work/maybe.md')], always('checking'))).toEqual([]);
  });

  it('keeps every mentioned path where the check is unavailable', () => {
    // `biorouter serve` (whose `window.electron` shim carries no
    // `checkFilePaths`) and every vitest suite without a bridge. "Start hidden"
    // there would mean the panel silently loses every prose artifact it has
    // ever had, so `unchecked` keeps the pre-existing behaviour.
    const artifacts = [mentioned('/work/report.md')];

    expect(applyMentionedFileGate(artifacts, always('unchecked'))).toEqual(artifacts);
  });

  it('never gates a path a tool call wrote, whatever the check says', () => {
    // A successful write is a stronger signal than a stat, and gating it would
    // add a round trip to the common case.
    const artifacts = [written('/work/plot.png'), figure];

    expect(applyMentionedFileGate(artifacts, always('missing'))).toEqual(artifacts);
  });

  it('preserves array identity when nothing is gated, so the panel memo is stable', () => {
    const artifacts = [written('/work/plot.png'), figure];

    expect(applyMentionedFileGate(artifacts, always('missing'))).toBe(artifacts);
  });

  it('asks only about the mentioned paths, in tab order', () => {
    const artifacts = [
      written('/work/plot.png'),
      mentioned('/work/a.md'),
      figure,
      mentioned('/work/b.md'),
    ];

    expect(mentionedArtifactPaths(artifacts)).toEqual(['/work/a.md', '/work/b.md']);
  });

  it('gates the prose path end to end while keeping the tool-written one', () => {
    // The whole chain on one transcript: the agent writes `plot.png` and, in the
    // same breath, SUGGESTS a `notes.md` it never creates.
    const messages: Message[] = [
      visibleMessage([
        { type: 'text', text: 'Wrote /work/plot.png. You could also keep /work/notes.md.' },
        {
          type: 'toolRequest',
          id: 't1',
          toolCall: {
            status: 'success',
            value: {
              name: 'developer__text_editor',
              arguments: { command: 'write', path: '/work/plot.png' },
            },
          },
        },
      ]),
      textToolResponse('t1', 'ok'),
    ];
    const collected = collectArtifactsFromMessages(messages, '/work');
    expect(mentionedArtifactPaths(collected)).toEqual(['/work/notes.md']);

    expect(applyMentionedFileGate(collected, always('missing'))).toEqual([
      { kind: 'file', title: 'plot.png', path: '/work/plot.png', mentionedOnly: undefined },
    ]);
  });

  it('feeds the panel the GATED list, not the raw collection', () => {
    // The defect was one consumer of a shared extractor left unhardened, which
    // is invisible to a unit test of either half. `useSessionArtifacts` is the
    // seam BaseChatContent actually reads, so this asserts the wiring at the
    // source; `useSessionArtifacts` itself is exercised for real below.
    const source = readFileSync(`${process.cwd()}/src/components/BaseChat.tsx`, 'utf8');
    expect(source).toMatch(
      /const \{ artifacts: sessionArtifacts, gatePending \} = useSessionArtifacts\(/
    );
    expect(source).toMatch(/gatePending,/);
  });
});

/**
 * The seam BaseChatContent reads, exercised for real: messages in, cards out,
 * over the actual batched IPC bridge. This is what proves the panel's consumer
 * of `referencedFilePaths` is hardened — asserting the gate and the hook
 * separately cannot, because the defect WAS the two not being connected.
 */
describe('useSessionArtifacts', () => {
  /** A `window.electron` carrying only the existence bridge. */
  function installCheckBridge(exists: (path: string) => boolean) {
    const checkFilePaths = vi.fn(async (requests: FilePathCheckRequest[]) =>
      requests.map((request) => ({ exists: exists(request.path), isDirectory: false }))
    );
    Object.defineProperty(window, 'electron', { configurable: true, value: { checkFilePaths } });
    return checkFilePaths;
  }

  afterEach(() => {
    // @ts-expect-error — remove the per-test electron stub.
    delete window.electron;
    resetFileLinkStatusForTests();
    vi.restoreAllMocks();
  });

  // The reproduced defect, verbatim: the assistant SUGGESTED writing a spec to a
  // path that had never existed, and the panel made a card for it, opened it in
  // a tab and rendered "File not available".
  const suggestion = () => [
    visibleMessage([
      {
        type: 'text',
        text: 'If the spec is long, writing it to a file (e.g. `/Users/ada/Desktop/kdps-intent.md`) and telling me the path is more reliable than pasting into chat.',
      },
    ]),
  ];

  it('never cards a prose path that does not exist', async () => {
    const checkFilePaths = installCheckBridge(() => false);
    const messages = suggestion();

    const { result } = renderHook(() => useSessionArtifacts(messages, '/work'));

    // Not even for one frame: the first render is already empty, and it stays
    // empty once the answer lands.
    expect(result.current.artifacts).toEqual([]);
    await waitFor(() => expect(result.current.gatePending).toBe(false));
    expect(result.current.artifacts).toEqual([]);
    expect(checkFilePaths.mock.calls[0][0]).toEqual([
      { path: '/Users/ada/Desktop/kdps-intent.md', workingDir: '/work' },
    ]);
  });

  it('cards a prose path that is really on disk', async () => {
    installCheckBridge(() => true);
    const messages = suggestion();

    const { result } = renderHook(() => useSessionArtifacts(messages, '/work'));

    await waitFor(() => expect(result.current.artifacts).toHaveLength(1));
    expect(result.current.artifacts[0]).toEqual({
      kind: 'file',
      title: 'kdps-intent.md',
      path: '/Users/ada/Desktop/kdps-intent.md',
      // Confirmed on disk, so a later read failure is a real disappearance.
      mentionedOnly: undefined,
    });
    expect(result.current.gatePending).toBe(false);
  });

  it('cards every prose path where no bridge can answer', () => {
    // `biorouter serve` and every bridgeless suite. Hiding here would silently
    // strip the panel of every prose artifact on the browser surface.
    const { result } = renderHook(() => useSessionArtifacts(suggestion(), '/work'));

    expect(result.current.artifacts).toEqual([
      {
        kind: 'file',
        title: 'kdps-intent.md',
        path: '/Users/ada/Desktop/kdps-intent.md',
        mentionedOnly: true,
      },
    ]);
    // Nothing to wait for, so the panel's baseline must not defer on our account.
    expect(result.current.gatePending).toBe(false);
  });

  it('cards a tool-written file without asking about it at all', async () => {
    const checkFilePaths = installCheckBridge(() => false);
    const messages: Message[] = [
      writeRequest('t1', 'developer__text_editor', { command: 'write', path: '/work/report.md' }),
      textToolResponse('t1', 'ok'),
    ];

    const { result } = renderHook(() => useSessionArtifacts(messages, '/work'));

    // A successful write is a receipt: it survives a bridge that says "no", and
    // costs no round trip.
    expect(result.current.artifacts).toEqual([
      { kind: 'file', title: 'report.md', path: '/work/report.md' },
    ]);
    expect(result.current.gatePending).toBe(false);
    await Promise.resolve();
    expect(checkFilePaths).not.toHaveBeenCalled();
  });

  it('reports the gate pending until the sweep settles', async () => {
    installCheckBridge(() => true);

    const { result } = renderHook(() => useSessionArtifacts(suggestion(), '/work'));

    // The panel's one-time auto-open baseline reads this: taken now, it would
    // bank an empty list and then treat the confirmed card as newly created.
    expect(result.current.gatePending).toBe(true);
    await waitFor(() => expect(result.current.gatePending).toBe(false));
  });
});

describe('decideArtifactAutoOpen', () => {
  const empty = new Set<string>();

  it('waits for a saved transcript to hydrate before taking a baseline', () => {
    // Session claims history but the transcript has not loaded (0 messages).
    // Snapshotting now would bank an empty baseline and spring the panel on
    // every old artifact once they load.
    expect(
      decideArtifactAutoOpen({
        scanDone: false,
        knownKeys: empty,
        reportedMessageCount: 8,
        loadedMessageCount: 0,
        artifactKeys: [],
        gatePending: false,
      })
    ).toEqual({ action: 'wait' });
  });

  it('SNAPSHOTS a reopened session and opens NOTHING (historical-reload guard)', () => {
    // The prove-by-revert gate: reopening an old chat whose transcript already
    // holds artifacts must bank them as known and open none. If the first scan
    // were ever changed to open, this asserts against it.
    const decision = decideArtifactAutoOpen({
      scanDone: false,
      knownKeys: empty,
      reportedMessageCount: 8,
      loadedMessageCount: 8,
      artifactKeys: ['file:/w/a.png', 'file:/w/b.png'],
      gatePending: false,
    });
    expect(decision.action).toBe('snapshot');
    if (decision.action !== 'snapshot') throw new Error('unreachable');
    expect([...decision.knownKeys]).toEqual(['file:/w/a.png', 'file:/w/b.png']);
    // No `openIndex` field exists on a snapshot decision — nothing opens.
    expect('openIndex' in decision).toBe(false);
  });

  it('waits for the mentioned-file gate before taking a baseline', () => {
    // The transcript has hydrated, but the prose paths are still being checked,
    // so `artifactKeys` is PARTIAL. Snapshotting now would bank the short list
    // and then treat every path confirmed a moment later as newly created —
    // springing the panel on a reopened saved session, which is the exact
    // failure the baseline exists to prevent, on a shorter timescale.
    expect(
      decideArtifactAutoOpen({
        scanDone: false,
        knownKeys: empty,
        reportedMessageCount: 8,
        loadedMessageCount: 8,
        artifactKeys: ['file:/w/a.png'],
        gatePending: true,
      })
    ).toEqual({ action: 'wait' });
  });

  it("still opens a live turn's new artifact while the gate is resolving", () => {
    // After the baseline, a gate answer landing is just another artifact
    // appearing — which is what auto-open is FOR. Deferring here would mean a
    // figure the agent just made sat unopened behind an unrelated check.
    const decision = decideArtifactAutoOpen({
      scanDone: true,
      knownKeys: new Set(['file:/w/a.png']),
      reportedMessageCount: 2,
      loadedMessageCount: 4,
      artifactKeys: ['file:/w/a.png', 'file:/w/b.png'],
      gatePending: true,
    });
    expect(decision.action).toBe('open');
  });

  it('snapshots an empty new session without waiting', () => {
    expect(
      decideArtifactAutoOpen({
        scanDone: false,
        knownKeys: empty,
        reportedMessageCount: 0,
        loadedMessageCount: 0,
        artifactKeys: [],
        gatePending: false,
      })
    ).toEqual({ action: 'snapshot', knownKeys: new Set() });
  });

  it('opens the newest previously-unseen artifact of a live turn', () => {
    const decision = decideArtifactAutoOpen({
      scanDone: true,
      knownKeys: new Set(['file:/w/a.png']),
      reportedMessageCount: 2,
      loadedMessageCount: 4,
      artifactKeys: ['file:/w/a.png', 'file:/w/b.png', 'file:/w/c.png'],
      gatePending: false,
    });
    expect(decision.action).toBe('open');
    if (decision.action !== 'open') throw new Error('unreachable');
    // The LAST unseen artifact is the one that opens.
    expect(decision.openIndex).toBe(2);
    // Every newly-seen artifact is banked, not only the opened one.
    expect(decision.knownKeys.has('file:/w/b.png')).toBe(true);
    expect(decision.knownKeys.has('file:/w/c.png')).toBe(true);
  });

  it('does not re-open artifacts already seen (tab switch / re-render)', () => {
    expect(
      decideArtifactAutoOpen({
        scanDone: true,
        knownKeys: new Set(['file:/w/a.png', 'file:/w/b.png']),
        reportedMessageCount: 2,
        loadedMessageCount: 4,
        artifactKeys: ['file:/w/a.png', 'file:/w/b.png'],
        gatePending: false,
      })
    ).toEqual({ action: 'none' });
  });
});

describe('shouldAutoRepairArtifact', () => {
  const now = 1_000_000;

  it('auto-fixes while the agent is actively working, regardless of timing', () => {
    for (const state of [
      ChatState.Thinking,
      ChatState.Streaming,
      ChatState.WaitingForUserInput,
      ChatState.Compacting,
      ChatState.RestartingAgent,
    ]) {
      // lastActive is stale, but the live state alone is enough.
      expect(shouldAutoRepairArtifact(state, 0, now)).toBe(true);
    }
  });

  it('auto-fixes an artifact that fails just after a turn finishes (grace window)', () => {
    expect(shouldAutoRepairArtifact(ChatState.Idle, now - 2_000, now)).toBe(true);
  });

  it('does NOT resume a conversation that has been idle past the grace window', () => {
    // The failure surfaced long after the agent last worked — user housekeeping.
    expect(shouldAutoRepairArtifact(ChatState.Idle, now - 60_000, now)).toBe(false);
  });

  it('does NOT resume when reopening a saved conversation (never active this session)', () => {
    // lastAgentActiveAt is 0 (default) — an old artifact re-rendering on load
    // must not resume the finished chat.
    expect(shouldAutoRepairArtifact(ChatState.LoadingConversation, 0, now)).toBe(false);
    expect(shouldAutoRepairArtifact(ChatState.Idle, 0, now)).toBe(false);
  });
});

/**
 * Defect 3.5. Four sibling FILES named in the assistant's prose were offered as
 * artifacts and the DIRECTORY holding them was not. Nothing downstream is at
 * fault — `looksLikePreviewableFile` accepts a folder, the main process stats
 * the path and answers `kind: 'directory'`, and the panel has
 * `DirectoryTreePreview` ready for it. The gap is in collection:
 * `PREVIEWABLE_TEXT_ARTIFACT_RE` requires a literal `.` plus an extension from
 * a closed allowlist, and a directory has neither.
 *
 * The signal used is a TRAILING SLASH, not "any extensionless path". A bare
 * `/usr/bin` is indistinguishable from an extensionless file and would drag
 * every command path and prose fragment into the tab strip; `results/` is the
 * convention the assistant actually writes when it means a folder.
 */
describe('a directory named in prose', () => {
  it('is offered alongside its siblings', () => {
    const messages = [
      visibleMessage([
        {
          type: 'text',
          text: 'Everything landed in /work/results/ — see /work/results/plot.png for the figure.',
        },
      ]),
    ];

    expect(collectArtifactsFromMessages(messages, '/work')).toEqual([
      { kind: 'file', title: 'results', path: '/work/results', mentionedOnly: true },
      { kind: 'file', title: 'plot.png', path: '/work/results/plot.png', mentionedOnly: true },
    ]);
  });

  it('anchors a relative folder against the working dir, and drops it without one', () => {
    const messages = [visibleMessage([{ type: 'text', text: 'Wrote them all to ./results/.' }])];

    expect(collectArtifactsFromMessages(messages, '/work')).toEqual([
      { kind: 'file', title: 'results', path: '/work/results', mentionedOnly: true },
    ]);
    expect(collectArtifactsFromMessages(messages)).toEqual([]);
  });

  // The discipline the trailing slash buys. Without it every one of these
  // becomes a tab.
  it('does not treat a bare extensionless word as a folder', () => {
    const messages = [
      visibleMessage([
        { type: 'text', text: 'It is on the PATH at /usr/bin and the ratio was 3/4.' },
      ]),
    ];

    expect(collectArtifactsFromMessages(messages, '/work')).toEqual([]);
  });
});
