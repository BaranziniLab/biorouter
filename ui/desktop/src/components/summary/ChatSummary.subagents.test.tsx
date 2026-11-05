import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { ChatSummary, type ChatSummaryProps } from '../ChatSummary';
import { parseSessionSubagents, type SessionSubagents } from '../../utils/sessionSubagents';
import { wireRow, wireSubagents } from '../../utils/sessionSubagents.fixture';
import { summaryCopy } from './copy';

const props: ChatSummaryProps = {
  toolCalls: '6',
  billedTokens: '211k',
  artifacts: '1',
  codeDelta: { added: 0, removed: 0 },
  hasWorkflow: false,
  onWorkflow: vi.fn(),
  onDiagnostics: vi.fn(),
  todos: {
    items: [{ id: '1', text: 'Compare clinic options', status: 'in_progress' }],
    loading: false,
    error: false,
    refresh: vi.fn(),
  },
};

const roster = parseSessionSubagents(
  wireSubagents([wireRow({ sessionId: 'a' })])
) as SessionSubagents;
const subagents = { data: roster, loading: false, error: false, refresh: vi.fn(), onOpen: vi.fn() };

/** Strip `//` and block comments, so a sentence about a pattern is not the pattern. */
const code = (path: string) =>
  readFileSync(join(process.cwd(), 'src', ...path.split('/')), 'utf8')
    .replace(/\/\*[\s\S]*?\*\//g, '')
    .replace(/^\s*\/\/.*$/gm, '');

describe('the Chat summary with subagents', () => {
  it('lists them between the to-do list and the statistics', () => {
    render(<ChatSummary {...props} subagents={subagents} />);
    const todo = screen.getByRole('region', { name: summaryCopy.todo });
    const agents = screen.getByRole('region', { name: summaryCopy.agents.subagents });
    const stats = screen.getByLabelText(summaryCopy.statsLabel);
    expect(todo.compareDocumentPosition(agents) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(agents.compareDocumentPosition(stats) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(agents.closest('[data-summary-body]')).not.toBeNull();
  });

  it('draws nothing for them when the prop is absent or the roster is empty', () => {
    const { rerender } = render(<ChatSummary {...props} />);
    expect(screen.queryByRole('region', { name: summaryCopy.agents.subagents })).toBeNull();
    rerender(
      <ChatSummary
        {...props}
        subagents={{
          ...subagents,
          data: parseSessionSubagents(wireSubagents([])) as SessionSubagents,
        }}
      />
    );
    expect(screen.queryByRole('region', { name: summaryCopy.agents.subagents })).toBeNull();
  });
});

/**
 * What only BaseChat can get wrong: how the roster reaches the summary. BaseChat
 * cannot be mounted in jsdom, so its joins are pinned at the source, as
 * `BaseChat.summaryRail.test.ts` does for the to-do list.
 */
describe('BaseChat: the subagents in the summary', () => {
  const base = code('components/BaseChat.tsx');

  it('reads the roster below the to-do list and above the early return', () => {
    const todos = base.indexOf('useSessionTodos(');
    const roster = base.indexOf('useSessionSubagents(');
    expect(roster).toBeGreaterThan(todos);
    expect(base.indexOf('if (sessionLoadError)')).toBeGreaterThan(roster);
  });

  it('gates it on the summary being shown or the turn running, as the to-do list is', () => {
    const call = base.slice(
      base.indexOf('useSessionSubagents('),
      base.indexOf('});', base.indexOf('useSessionSubagents('))
    );
    expect(call).toContain('visible: !chatLoading && (reviewOpen || summaryRail.shown),');
    expect(call).toContain(
      'turnRunning: !chatLoading && summaryActive && isRunningState(chatState),'
    );
  });

  it('hands the roster and the tab opener to both the rail and the popover', () => {
    expect(base).toContain(
      'subagents: { ...sessionSubagents, onOpen: openSubagent, onOpenGate: openPhaseGate },'
    );
    // One props object feeds both surfaces.
    expect(base).toContain('summary={summaryProps}');
    expect(base).toMatch(/<ChatSummaryRail[\s\S]*?\{\.\.\.summaryProps\}/);
    expect(base).toMatch(/openSubagentTab\(\s*\{ dispatch: chatGroupsDispatch/);
  });

  it('names running subagents on the header button only while the rail is hidden', () => {
    expect(base).toMatch(/const summaryRunningSubagents = summaryRail\.shown\s*\? 0/);
    expect(base).toContain('runningSubagents={summaryRunningSubagents}');
  });
});
