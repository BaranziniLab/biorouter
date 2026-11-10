import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it, vi } from 'vitest';
import type { Message } from '../api';
import {
  COMFORTABLE_MAX_ROWS,
  PHASE_KINDS,
  PHASE_STATUSES,
  QUIET_AFTER_SECONDS,
  SUBAGENT_RUN_STATUSES,
  SETUP_COUNT_KINDS,
  formatElapsed,
  hasActiveSubagent,
  heldByTitle,
  inSpawnOrder,
  isClosedPhase,
  isQuiet,
  openSubagentTab,
  parseSessionSubagents,
  phaseOpenByDefault,
  rosterRevisionHasCalls,
  rowsByPhase,
  shortModelLabel,
  subagentCounts,
  subagentGroup,
  subagentListLayout,
  subagentRosterRevision,
  subagentSetup,
  subagentTitle,
  type PhaseSnapshot,
  type SubagentRow,
} from './sessionSubagents';
import { olderPhase, wirePhase, wireRow } from './sessionSubagents.fixture';

function row(overrides: Record<string, unknown> = {}): SubagentRow {
  return parseSessionSubagents({ parentSessionId: 'p', subagents: [wireRow(overrides)] })!
    .subagents[0];
}

function phase(overrides: Partial<PhaseSnapshot> = {}): PhaseSnapshot {
  return {
    id: 'research',
    title: 'Research',
    goal: null,
    kind: 'research',
    status: 'pending',
    dependsOn: [],
    agentIds: [],
    counts: { total: 0, done: 0, running: 0, blocked: 0, failed: 0, quiet: 0 },
    handoff: null,
    startedAt: null,
    finishedAt: null,
    ...overrides,
  };
}

/** The contract files, read so a variant added on one side fails here. */
const ROSTER_RS = readFileSync(
  join(process.cwd(), '..', '..', 'crates', 'biorouter', 'src', 'agents', 'subagent_roster.rs'),
  'utf8'
);
const PLAN_RS = readFileSync(
  join(process.cwd(), '..', '..', 'crates', 'biorouter', 'src', 'agents', 'orchestration_plan.rs'),
  'utf8'
);

/** The snake_case variant names of a Rust `pub enum`, as serde writes them. */
function rustVariants(source: string, name: string): string[] {
  const body = source.match(new RegExp(`pub enum ${name} \\{([\\s\\S]*?)\\n\\}`))?.[1];
  expect(body, name).toBeTruthy();
  return [...body!.replace(/\/\/[^\n]*/g, '').matchAll(/^\s*([A-Z][A-Za-z]*),?$/gm)].map((match) =>
    match[1].replace(/([a-z])([A-Z])/g, '$1_$2').toLowerCase()
  );
}

describe('the contract, read from the Rust side', () => {
  it('knows every run status, phase kind and phase status the daemon can send', () => {
    expect([...SUBAGENT_RUN_STATUSES].sort()).toEqual(
      rustVariants(ROSTER_RS, 'SubagentRunStatus').sort()
    );
    expect([...PHASE_KINDS].sort()).toEqual(rustVariants(PLAN_RS, 'PhaseKind').sort());
    expect([...PHASE_STATUSES].sort()).toEqual(rustVariants(PLAN_RS, 'PhaseStatus').sort());
  });

  it('calls a row quiet at the same age the daemon counts a phase’s quiet children', () => {
    const rust = PLAN_RS.match(/pub const QUIET_AFTER_SECONDS: u64 = (\d+);/)?.[1];
    expect(Number(rust)).toBe(QUIET_AFTER_SECONDS);
  });
});

describe('parseSessionSubagents', () => {
  it('reads every field of a row', () => {
    const parsed = row({ role: 'Reviewer', phaseId: 'research', endedAt: null });
    expect(parsed).toMatchObject({
      sessionId: 'child-1',
      name: 'Search PubMed for trials',
      role: 'Reviewer',
      phaseId: 'research',
      status: 'running',
      loopPhase: 'tool_batch',
      quietSeconds: 4,
      elapsedSeconds: 75,
      providerName: 'anthropic',
      modelName: 'claude-sonnet-4-5-20250929',
      extensions: ['developer', 'webdocuments'],
      skills: ['scientific-research'],
      knowledgeBases: ['soul'],
      privacyTier: 'public',
      totalTokens: 12345,
      messageCount: 7,
      toolCallCount: 3,
      resultLine: null,
    });
  });

  it('keeps a row with a status this build does not know, as unknown', () => {
    const data = parseSessionSubagents({
      parentSessionId: 'p',
      subagents: [wireRow({ status: 'paused_for_lunch' }), wireRow({ sessionId: 'b', status: 7 })],
      plan: null,
    });
    expect(data?.subagents.map((r) => r.status)).toEqual(['unknown', 'unknown']);
  });

  it('drops only a row with no session id, and the second of two with the same id', () => {
    const data = parseSessionSubagents({
      parentSessionId: 'p',
      subagents: [
        wireRow({ sessionId: '' }),
        wireRow({ sessionId: 'a', name: 'first' }),
        wireRow({ sessionId: 'a', name: 'second' }),
        'nonsense',
        wireRow({ sessionId: 'b' }),
      ],
    });
    expect(data?.subagents.map((r) => [r.sessionId, r.name])).toEqual([
      ['a', 'first'],
      ['b', 'Search PubMed for trials'],
    ]);
  });

  it('degrades malformed fields instead of failing the row', () => {
    const parsed = row({
      extensions: 'developer',
      skills: ['a', '', 3, 'a', 'b'],
      privacyTier: 'secret',
      totalTokens: -5,
      messageCount: 'many',
      quietSeconds: null,
      elapsedSeconds: Number.NaN,
    });
    expect(parsed.extensions).toEqual([]);
    expect(parsed.skills).toEqual(['a', 'b']);
    // A tier this build does not know is drawn as not yet known, never as public.
    expect(parsed.privacyTier).toBeNull();
    expect(parsed.totalTokens).toBeNull();
    expect(parsed.messageCount).toBe(0);
    expect(parsed.quietSeconds).toBeNull();
    expect(parsed.elapsedSeconds).toBeNull();
  });

  it('reads a plan, degrading an unknown kind to other and an unknown status to pending', () => {
    const data = parseSessionSubagents({
      parentSessionId: 'p',
      subagents: [],
      plan: {
        title: 'Literature review',
        goal: null,
        revision: 3,
        phases: [
          {
            id: 'plan',
            title: 'Plan',
            goal: null,
            kind: 'plan',
            status: 'completed',
            dependsOn: [],
            agentIds: ['x'],
            counts: { total: 1, done: 1, running: 0, blocked: 0, failed: 0, quiet: 0 },
            handoff: 'Three questions',
            startedAt: null,
            finishedAt: null,
          },
          { id: 'odd', kind: 'divination', status: 'thinking', counts: null },
          { title: 'no id' },
        ],
      },
    });
    expect(data?.plan?.title).toBe('Literature review');
    expect(data?.plan?.revision).toBe(3);
    expect(data?.plan?.phases.map((p) => [p.id, p.kind, p.status, p.title])).toEqual([
      ['plan', 'plan', 'completed', 'Plan'],
      ['odd', 'other', 'pending', 'odd'],
    ]);
    expect(data?.plan?.phases[1].counts).toEqual({
      total: 0,
      done: 0,
      running: 0,
      blocked: 0,
      failed: 0,
      quiet: 0,
    });
  });

  it('reads which gated phase holds a phase, tolerantly, and says it by that phase’s title', () => {
    const data = parseSessionSubagents({
      parentSessionId: 'p',
      subagents: [],
      plan: {
        title: 'Moon facts',
        revision: 1,
        phases: [
          wirePhase({ id: 'research', title: 'Research', status: 'in_progress' }),
          wirePhase({ id: 'write', title: 'Write', heldBy: 'research' }),
          wirePhase({ id: 'review', title: 'Review', heldBy: 'gone-phase' }),
          wirePhase({ id: 'ship', title: 'Ship', heldBy: 7 }),
          olderPhase({ id: 'old', title: 'Old' }),
          wirePhase({ id: 'closed', title: 'Closed', status: 'completed', heldBy: 'research' }),
        ],
      },
    })!;
    const phases = data.plan!.phases;
    expect(phases.map((phase) => phase.heldBy)).toEqual([
      null,
      'research',
      'gone-phase',
      null,
      null,
      'research',
    ]);
    expect(phases.map((phase) => heldByTitle(data.plan, phase))).toEqual([
      null,
      'Research',
      // A holder the plan no longer lists is named by its id.
      'gone-phase',
      null,
      null,
      // A closed phase waits for nothing.
      null,
    ]);
    expect(isClosedPhase(phases[5])).toBe(true);
    expect(isClosedPhase(phases[1])).toBe(false);
  });

  it('knows the snapshot field that names a held phase’s holder, read from the Rust side', () => {
    // `PhaseSnapshot.held_by`, camelCased on the wire.
    expect(PLAN_RS).toMatch(/pub held_by: Option<String>,/);
  });

  it('answers null for a body that is not an object', () => {
    expect(parseSessionSubagents(null)).toBeNull();
    expect(parseSessionSubagents([])).toBeNull();
    expect(parseSessionSubagents('x')).toBeNull();
    expect(parseSessionSubagents({})).toEqual({ parentSessionId: '', subagents: [], plan: null });
  });
});

describe('groups and counts', () => {
  it('groups running, needs attention and done, keeping unknown with the finished', () => {
    expect(SUBAGENT_RUN_STATUSES.map((status) => [status, subagentGroup(status)])).toEqual([
      ['queued', 'running'],
      ['running', 'running'],
      ['cancelling', 'running'],
      ['completed', 'done'],
      ['blocked', 'attention'],
      ['incomplete', 'attention'],
      ['failed', 'attention'],
      ['interrupted', 'attention'],
      ['unknown', 'done'],
    ]);
  });

  it('counts each group and the quiet ones', () => {
    const rows = [
      row({ sessionId: 'a', status: 'running', quietSeconds: QUIET_AFTER_SECONDS }),
      row({ sessionId: 'b', status: 'queued', quietSeconds: 1 }),
      row({ sessionId: 'c', status: 'failed', quietSeconds: 9999 }),
      row({ sessionId: 'd', status: 'completed' }),
    ];
    expect(subagentCounts(rows)).toEqual({ total: 4, running: 2, attention: 1, done: 1, quiet: 1 });
    // Only an ACTIVE child can be quiet.
    expect(isQuiet(rows[2])).toBe(false);
    expect(hasActiveSubagent({ parentSessionId: 'p', subagents: rows, plan: null })).toBe(true);
    expect(hasActiveSubagent({ parentSessionId: 'p', subagents: [rows[3]], plan: null })).toBe(
      false
    );
    expect(hasActiveSubagent(null)).toBe(false);
  });

  it('switches from comfortable to dense rows past four subagents', () => {
    expect(COMFORTABLE_MAX_ROWS).toBe(4);
    expect(subagentListLayout(0)).toBe('comfortable');
    expect(subagentListLayout(4)).toBe('comfortable');
    expect(subagentListLayout(5)).toBe('dense');
  });
});

describe('phases', () => {
  it('places a row under the phase that lists it, else its phase id, else Other', () => {
    const plan = {
      title: 'Plan',
      goal: null,
      revision: 1,
      phases: [
        phase({ id: 'research', agentIds: ['a'] }),
        phase({ id: 'verify', kind: 'verification' }),
      ],
    };
    const rows = [
      row({ sessionId: 'a', phaseId: 'verify' }),
      row({ sessionId: 'b', phaseId: 'verify' }),
      row({ sessionId: 'c', phaseId: 'gone' }),
      row({ sessionId: 'd' }),
    ];
    const { phases, unphased } = rowsByPhase(plan, rows);
    expect(phases.get('research')?.map((r) => r.sessionId)).toEqual(['a']);
    expect(phases.get('verify')?.map((r) => r.sessionId)).toEqual(['b']);
    expect(unphased.map((r) => r.sessionId)).toEqual(['c', 'd']);
  });

  it('opens the running phase, and folds finished ones only in a plan of more than three', () => {
    expect(phaseOpenByDefault(phase({ status: 'in_progress' }), 8)).toBe(true);
    expect(phaseOpenByDefault(phase({ status: 'blocked' }), 8)).toBe(true);
    expect(phaseOpenByDefault(phase({ status: 'completed' }), 8)).toBe(false);
    expect(phaseOpenByDefault(phase({ status: 'completed' }), 3)).toBe(true);
    expect(phaseOpenByDefault(phase({ status: 'pending' }), 4)).toBe(false);
  });
});

describe('labels', () => {
  it('leads with the role, then the name', () => {
    expect(subagentTitle(row({ role: 'Reviewer' }))).toBe('Reviewer');
    expect(subagentTitle(row())).toBe('Search PubMed for trials');
    expect(subagentTitle(row({ name: '' }))).toBe('Subagent');
  });

  it('shortens model names to fit a narrow row', () => {
    expect(shortModelLabel('claude-sonnet-4-5-20250929')).toBe('claude-sonnet-4-5');
    expect(shortModelLabel('accounts/fireworks/models/qwen3-235b')).toBe('qwen3-235b');
    expect(shortModelLabel('us.anthropic.claude-opus-4-1-20250805-v1:0')).toBe('claude-opus-4-1');
    expect(shortModelLabel('qwen3.8:latest')).toBe('qwen3.8');
    expect(shortModelLabel('gpt-5.5-2026-01-15')).toBe('gpt-5.5');
    expect(shortModelLabel('a-really-long-model-name-that-goes-on-and-on')).toBe(
      'a-really-long-model-nam…'
    );
    expect(shortModelLabel(null, 'ollama')).toBe('ollama');
    expect(shortModelLabel(null, null)).toBeNull();
  });

  it('formats run time compactly', () => {
    expect(formatElapsed(null)).toBeNull();
    expect(formatElapsed(-1)).toBeNull();
    expect(formatElapsed(0)).toBe('0s');
    expect(formatElapsed(59)).toBe('59s');
    expect(formatElapsed(60)).toBe('1m');
    expect(formatElapsed(3599)).toBe('59m');
    expect(formatElapsed(3600)).toBe('1h');
    expect(formatElapsed(3900)).toBe('1h 5m');
    expect(formatElapsed(86400 * 2 + 5)).toBe('2d');
  });

  it('gives the second line the model, the setup as counts and the run time', () => {
    expect(subagentSetup(row())).toEqual({
      model: 'claude-sonnet-4-5',
      counts: [
        { kind: 'extension', count: 2 },
        { kind: 'skill', count: 1 },
        { kind: 'knowledge', count: 1 },
      ],
      elapsed: '1m',
    });
    // A kind the child has none of is left out, not counted as zero.
    expect(
      subagentSetup(
        row({ extensions: [], skills: ['a', 'b'], knowledgeBases: [], elapsedSeconds: null })
      )
    ).toEqual({ model: 'claude-sonnet-4-5', counts: [{ kind: 'skill', count: 2 }], elapsed: null });
    expect(
      subagentSetup(row({ modelName: 'gpt-5.5', extensions: [], skills: [], elapsedSeconds: 5 }))
    ).toEqual({ model: 'gpt-5.5', counts: [{ kind: 'knowledge', count: 1 }], elapsed: '5s' });
    expect(SETUP_COUNT_KINDS).toEqual(['extension', 'skill', 'knowledge']);
  });
});

describe('spawn order', () => {
  const at = (second: number) => `2026-11-04T10:00:${String(second).padStart(2, '0')}Z`;
  const ids = (rows: SubagentRow[]) => inSpawnOrder(rows).map((r) => r.sessionId);

  it('puts the oldest first, whatever order the rows arrive in', () => {
    const rows = [
      row({ sessionId: 'c', createdAt: at(2), status: 'completed' }),
      row({ sessionId: 'a', createdAt: at(0), status: 'completed' }),
      row({ sessionId: 'b', createdAt: at(1), status: 'running' }),
    ];
    expect(ids(rows)).toEqual(['a', 'b', 'c']);
    // A copy: the roster the caller holds is not reordered.
    expect(rows.map((r) => r.sessionId)).toEqual(['c', 'a', 'b']);
  });

  it('orders a fan-out spawned in one second by label, in natural order, then by name', () => {
    const rows = [
      row({ sessionId: 'r10', createdAt: at(0), name: 'ribosome 10' }),
      row({ sessionId: 'r2', createdAt: at(0), name: 'ribosome 2' }),
      row({ sessionId: 'r1', createdAt: at(0), name: 'Ribosome 1' }),
      row({ sessionId: 'early', createdAt: '2026-11-04T09:59:59Z', name: 'zeta' }),
    ];
    expect(ids(rows)).toEqual(['early', 'r1', 'r2', 'r10']);
    // The role is the label a row leads with, so it decides before the name.
    const roles = [
      row({ sessionId: 'x', createdAt: at(0), role: 'Reviewer', name: 'a' }),
      row({ sessionId: 'y', createdAt: at(0), role: 'Analyst', name: 'z' }),
      row({ sessionId: 'z', createdAt: at(0), role: 'Analyst', name: 'b' }),
    ];
    expect(ids(roles)).toEqual(['z', 'y', 'x']);
  });

  it('never moves a row for its state, and keeps the roster’s order on a full tie', () => {
    const rows = [
      row({ sessionId: 'first', createdAt: at(0), name: 'same', status: 'completed' }),
      row({ sessionId: 'second', createdAt: at(0), name: 'same', status: 'running' }),
    ];
    expect(ids(rows)).toEqual(['first', 'second']);
    expect(ids(rows.map((r) => ({ ...r, status: 'failed' as const })))).toEqual([
      'first',
      'second',
    ]);
    // A row with no spawn time sorts after the ones that have one.
    expect(
      ids([row({ sessionId: 'unknown', createdAt: '' }), row({ sessionId: 'known' })])
    ).toEqual(['known', 'unknown']);
  });
});

function call(id: string, name: string, answered = true, result: unknown = { content: [] }) {
  return {
    role: 'assistant',
    created: 0,
    metadata: { agentVisible: true, userVisible: true },
    content: [
      { type: 'toolRequest', id, toolCall: { status: 'success', value: { name, arguments: {} } } },
      ...(answered
        ? [{ type: 'toolResponse', id, toolResult: { status: 'success', value: result } }]
        : []),
    ],
  } as unknown as Message;
}

describe('subagentRosterRevision', () => {
  it('moves when a spawn, the plan tool, a watch or a close answers, whatever its prefix', () => {
    const before = subagentRosterRevision([]);
    expect(rosterRevisionHasCalls(before)).toBe(false);
    const names = [
      'workspace__subagent',
      'subagent',
      'mcp__biorouter__workspace__subagent',
      'workspace__workspace_orchestrate',
      'workspace__workspace_watch',
      'workspace__workspace_close',
    ];
    let previous = before;
    const messages: Message[] = [];
    names.forEach((name, index) => {
      messages.push(call(`c${index}`, name));
      const next = subagentRosterRevision(messages);
      expect(next, name).not.toBe(previous);
      previous = next;
    });
    expect(rosterRevisionHasCalls(previous)).toBe(true);
  });

  it('does not move for other tools, an unanswered call or a replay', () => {
    const base = [call('a', 'workspace__subagent')];
    const revision = subagentRosterRevision(base);
    expect(subagentRosterRevision([...base, call('b', 'developer__shell')])).toBe(revision);
    expect(subagentRosterRevision([...base, call('c', 'workspace__subagent', false)])).toBe(
      revision
    );
    expect(subagentRosterRevision([...base])).toBe(revision);
    // `workspace_list` reads the roster but changes nothing.
    expect(subagentRosterRevision([...base, call('d', 'workspace__workspace_list')])).toBe(
      revision
    );
  });

  it('counts roster calls a script ran through code execution', () => {
    const script = call('run', 'code_execution__execute_code', true, {
      content: [],
      _meta: { 'biorouter/tool-calls': [{ tool: 'workspace__subagent', status: 'ok' }] },
    });
    expect(rosterRevisionHasCalls(subagentRosterRevision([script]))).toBe(true);
  });
});

describe('openSubagentTab', () => {
  it('opens and activates a tab in place on /pair, born titled', () => {
    const dispatch = vi.fn();
    const navigate = vi.fn();
    openSubagentTab({ dispatch, pathname: '/pair', navigate }, 'child-1', ' Reviewer ');
    expect(dispatch).toHaveBeenCalledWith({
      type: 'openTab',
      payload: { sessionId: 'child-1', title: 'Reviewer' },
    });
    expect(navigate).not.toHaveBeenCalled();
  });

  it('goes through /pair from any other route, or with no tab strip', () => {
    const dispatch = vi.fn();
    const navigate = vi.fn();
    openSubagentTab({ dispatch, pathname: '/sessions', navigate }, 'child-1', 'Reviewer');
    expect(dispatch).not.toHaveBeenCalled();
    expect(navigate).toHaveBeenCalledWith('/pair?resumeSessionId=child-1', {
      replace: undefined,
      state: { title: 'Reviewer' },
    });
    navigate.mockClear();
    openSubagentTab({ pathname: '/pair', navigate }, 'child 2');
    expect(navigate).toHaveBeenCalledWith('/pair?resumeSessionId=child%202', {
      replace: undefined,
      state: undefined,
    });
  });
});
