import { describe, expect, it, vi } from 'vitest';
import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ToolRequestMessageContent, ToolResponseMessageContent } from '../types/message';
import ToolCallWithResponse, { exploreStepOf } from './ToolCallWithResponse';
import { ToolCallRows, exploredCounts, foldExploreRuns } from './ToolCallRows';
import { EXPLORED_COPY } from './toolCallCopy';

const request = (
  id: string,
  name: string,
  args: Record<string, unknown> = {},
  metadata?: Record<string, unknown>
): ToolRequestMessageContent =>
  ({
    type: 'toolRequest',
    id,
    toolCall: { status: 'success', value: { name, arguments: args } },
    ...(metadata ? { metadata } : {}),
  }) as ToolRequestMessageContent;

const ok = (id: string, text = 'done'): ToolResponseMessageContent =>
  ({
    type: 'toolResponse',
    id,
    toolResult: { status: 'success', value: { content: [{ type: 'text', text }], isError: false } },
  }) as ToolResponseMessageContent;

const failed = (id: string): ToolResponseMessageContent =>
  ({
    type: 'toolResponse',
    id,
    toolResult: { status: 'error', error: 'No such file' },
  }) as ToolResponseMessageContent;

const view = (id: string, path: string) =>
  request(id, 'developer__text_editor', { command: 'view', path });
const list = (id: string) => request(id, 'files__files_list', { path: 'src' });
const search = (id: string) => request(id, 'code_execution__search_modules', { query: 'x' });
const shell = (id: string) => request(id, 'developer__shell', { command: 'ls' });

describe('foldExploreRuns', () => {
  const step = (kind: 'read' | 'list' | 'search' | null) =>
    kind ? { kind, running: false } : null;

  it('folds two or more consecutive steps and keeps the order of everything', () => {
    const items = ['r1', 'r2', 's', 'r3', 'x', 'r4'];
    const kinds: Record<string, 'read' | 'search' | null> = {
      r1: 'read',
      r2: 'read',
      s: 'search',
      r3: 'read',
      x: null,
      r4: 'read',
    };
    const segments = foldExploreRuns(items, (item) => step(kinds[item]));
    expect(segments).toEqual([
      {
        type: 'explored',
        items: ['r1', 'r2', 's', 'r3'],
        kinds: ['read', 'read', 'search', 'read'],
        running: false,
      },
      { type: 'single', item: 'x' },
      { type: 'single', item: 'r4' },
    ]);
  });

  it('leaves a lone read on its own line', () => {
    expect(foldExploreRuns(['a'], () => step('read'))).toEqual([{ type: 'single', item: 'a' }]);
  });

  it('is running while any step of the run is', () => {
    const [segment] = foldExploreRuns(['a', 'b'], (item) => ({
      kind: 'read' as const,
      running: item === 'b',
    }));
    expect(segment).toMatchObject({ type: 'explored', running: true });
  });
});

describe('exploredCounts', () => {
  it('names only the kinds the run holds, singular and plural', () => {
    expect(exploredCounts(['read', 'read', 'read', 'search'])).toBe('3 files, 1 search');
    expect(exploredCounts(['list', 'search', 'search'])).toBe('1 folder, 2 searches');
    expect(exploredCounts(['read'])).toBe('1 file');
  });
});

describe('exploreStepOf', () => {
  it('takes built-in reads, lists and searches', () => {
    expect(exploreStepOf(view('a', 'a.ts'), ok('a'), false)).toEqual({
      kind: 'read',
      running: false,
    });
    expect(exploreStepOf(list('b'), ok('b'), false)).toMatchObject({ kind: 'list' });
    expect(exploreStepOf(search('c'), ok('c'), false)).toMatchObject({ kind: 'search' });
  });

  it('marks a call without an answer as running while the turn runs', () => {
    expect(exploreStepOf(view('a', 'a.ts'), undefined, true)).toEqual({
      kind: 'read',
      running: true,
    });
  });

  it('keeps writes, shells and external extensions on their own lines', () => {
    expect(
      exploreStepOf(
        request('w', 'developer__text_editor', { command: 'write', path: 'a.ts' }),
        ok('w'),
        false
      )
    ).toBeNull();
    expect(exploreStepOf(shell('s'), ok('s'), false)).toBeNull();
    expect(exploreStepOf(request('e', 'acme__read_file', { path: 'a' }), ok('e'), false)).toBe(
      null
    );
  });

  it('keeps a failure, a stopped call and a sandboxed child call visible on its own line', () => {
    expect(exploreStepOf(view('f', 'a.ts'), failed('f'), false)).toBeNull();
    expect(exploreStepOf(view('s', 'a.ts'), undefined, false)).toBeNull();
    expect(
      exploreStepOf(
        request('c', 'Read', { file_path: 'a.ts' }, { biorouterProviderExecuted: 'child' }),
        ok('c'),
        false
      )
    ).toBeNull();
  });
});

describe('ToolCallRows', () => {
  const renderRows = (
    toolRequests: ToolRequestMessageContent[],
    responses: ToolResponseMessageContent[],
    turnActive = false
  ) =>
    render(
      <ToolCallRows
        toolRequests={toolRequests}
        toolResponses={new Map(responses.map((response) => [response.id, response]))}
        turnActive={turnActive}
        renderCall={(toolRequest) => (
          <ToolCallWithResponse
            isCancelledMessage={false}
            toolRequest={toolRequest}
            toolResponse={responses.find((response) => response.id === toolRequest.id)}
            turnActive={turnActive}
            onOpenArtifact={vi.fn()}
          />
        )}
      />
    );

  it('draws a run of reads and searches as one Explored row that opens onto the calls', async () => {
    const user = userEvent.setup();
    const { container } = renderRows(
      [view('a', 'a.ts'), view('b', 'b.ts'), search('c'), shell('d')],
      [ok('a'), ok('b'), ok('c'), ok('d')]
    );

    // Two entries: the folded run and the shell call, each a message tool block.
    expect(container.querySelectorAll('.biorouter-message-tool')).toHaveLength(2);
    const trigger = screen.getByRole('button', { name: 'Explored · 2 files, 1 search' });
    expect(screen.queryByText('Read a.ts')).toBeNull();
    expect(screen.getByText('Ran ls')).toBeInTheDocument();

    await user.click(trigger);
    const row = screen.getByTestId('tool-explored-row');
    expect(within(row).getByText('Read a.ts')).toBeInTheDocument();
    expect(within(row).getByText('Read b.ts')).toBeInTheDocument();
    expect(row.querySelector('[data-tool-glyph="explored"]')).not.toBeNull();
    // The calls sit on the plain body, not in a well: no box in a box.
    expect(row.querySelector('.br-transcript-row-well')).toBeNull();
  });

  it('says Exploring and breathes while a call of the run still works', () => {
    renderRows([view('a', 'a.ts'), view('b', 'b.ts')], [ok('a')], true);
    expect(screen.getByText(EXPLORED_COPY.running)).toBeInTheDocument();
    expect(screen.getByTestId('tool-explored-row')).toHaveAttribute('data-running');
  });

  it('leaves a failed read in view between the runs it splits', () => {
    renderRows(
      [view('a', 'a.ts'), view('b', 'b.ts'), view('c', 'missing.ts'), view('d', 'd.ts')],
      [ok('a'), ok('b'), failed('c'), ok('d')]
    );
    expect(screen.getAllByTestId('tool-explored-row')).toHaveLength(1);
    expect(screen.getByText('Failed to read missing.ts')).toBeInTheDocument();
    expect(screen.getByText('Read d.ts')).toBeInTheDocument();
  });

  it('draws a lone read as an ordinary row', () => {
    renderRows([view('a', 'a.ts')], [ok('a')]);
    expect(screen.queryByTestId('tool-explored-row')).toBeNull();
    expect(screen.getByText('Read a.ts')).toBeInTheDocument();
  });
});
