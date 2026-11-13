import { describe, expect, it } from 'vitest';
import {
  parseDecisionSettings,
  parsePhaseGateSnapshot,
  type PhaseGateSnapshot,
} from '../../utils/sessionDecisions';
import {
  SUBAGENT_RUN_STATUSES,
  inSpawnOrder,
  parseSessionSubagents,
  type SubagentRow,
} from '../../utils/sessionSubagents';
import { wirePhase, wireRow, wireSubagents } from '../../utils/sessionSubagents.fixture';
import {
  AGENT_DENSE_FROM,
  AGENT_HEAD_ROWS,
  AGENT_MARK_KINDS,
  AGENT_MARK_PX,
  AGENT_ROWS_MAX,
  AGENT_STATUS_MARKS,
  DECISION_STATE_MARKS,
  agentGridSize,
  agentGroups,
  agentMarkKind,
  agentMarkPulses,
  agentMarkShape,
  agentRowMarkPx,
  gateMarkKind,
  groupDrawsGrid,
  groupDrawsGridFolded,
  headRank,
  inHeadOrder,
  splitHead,
  subagentWatchers,
} from './agentGrid';

function rows(...wire: Record<string, unknown>[]): SubagentRow[] {
  return parseSessionSubagents(wireSubagents(wire))!.subagents;
}

function one(overrides: Record<string, unknown>): SubagentRow {
  return rows(wireRow(overrides))[0];
}

const gate = (overrides: Record<string, unknown>): PhaseGateSnapshot =>
  parsePhaseGateSnapshot({
    threshold: 0.8,
    questionCount: 1,
    attempts: 1,
    maxAttempts: 3,
    lastPassed: null,
    lastMinP: null,
    questions: { done: 'Is it done?' },
    ...overrides,
  })!;

describe('the size scales with the count', () => {
  it('lists four agents or fewer as rows, then draws a grid, then a dense block from 100', () => {
    expect(AGENT_ROWS_MAX).toBe(4);
    expect(AGENT_DENSE_FROM).toBe(100);
    expect([0, 1, 4].map(agentGridSize)).toEqual(['rows', 'rows', 'rows']);
    expect([5, 8, 99].map(agentGridSize)).toEqual(['grid', 'grid', 'grid']);
    expect([100, 150, 1000].map(agentGridSize)).toEqual(['dense', 'dense', 'dense']);
  });

  it('gets smaller as it grows: 14px marks in rows, 10px squares, 7px squares', () => {
    expect(AGENT_MARK_PX).toEqual({ rows: 14, grid: 10, dense: 7 });
    // A row's own mark is the bigger one in a chat of rows, else the grid's square.
    expect(agentRowMarkPx('rows')).toBe(14);
    expect(agentRowMarkPx('grid')).toBe(10);
    expect(agentRowMarkPx('dense')).toBe(10);
  });

  it('draws an open group’s grid from two agents in a larger chat, and none in a chat of rows', () => {
    expect(groupDrawsGrid(3, 'rows')).toBe(false);
    expect(groupDrawsGrid(0, 'grid')).toBe(false);
    // A group of one is its row alone: one square above the row it repeats is noise.
    expect(groupDrawsGrid(1, 'grid')).toBe(false);
    expect(groupDrawsGrid(1, 'dense')).toBe(false);
    expect(groupDrawsGrid(2, 'grid')).toBe(true);
    expect(groupDrawsGrid(150, 'dense')).toBe(true);
  });

  it('keeps a folded group’s grid while an agent in it is unfinished, a group of one too', () => {
    const running = one({ status: 'running', quietSeconds: 2 });
    const done = one({ status: 'completed', quietSeconds: null });
    // Folded, the square is the only sign of where a lone agent stands.
    expect(groupDrawsGridFolded([running], 'grid')).toBe(true);
    expect(groupDrawsGridFolded([done, running], 'dense')).toBe(true);
    // A folded group whose every agent finished is its line alone.
    expect(groupDrawsGridFolded([done], 'grid')).toBe(false);
    expect(groupDrawsGridFolded([], 'grid')).toBe(false);
    // A chat of rows draws no grid at all.
    expect(groupDrawsGridFolded([running], 'rows')).toBe(false);
  });
});

describe('a status is a shape first, then an ink', () => {
  it('gives every run status a mark, and every kind a distinct meaning', () => {
    for (const status of SUBAGENT_RUN_STATUSES) {
      expect(AGENT_MARK_KINDS, status).toContain(AGENT_STATUS_MARKS[status]);
    }
    expect(AGENT_STATUS_MARKS).toEqual({
      queued: 'queued',
      running: 'running',
      cancelling: 'running',
      completed: 'done',
      blocked: 'waiting',
      incomplete: 'waiting',
      interrupted: 'waiting',
      failed: 'failed',
      unknown: 'idle',
    });
  });

  it('reads an active child gone quiet as stalled, not running', () => {
    expect(agentMarkKind(one({ status: 'running', quietSeconds: 4 }))).toBe('running');
    expect(agentMarkKind(one({ status: 'running', quietSeconds: 600 }))).toBe('waiting');
    // A finished child is never quiet, however long ago it last spoke.
    expect(agentMarkKind(one({ status: 'completed', quietSeconds: 600 }))).toBe('done');
  });

  it('pulses only a child that is running, never one stopping or gone quiet', () => {
    expect(agentMarkPulses(one({ status: 'running', quietSeconds: 4 }))).toBe(true);
    expect(agentMarkPulses(one({ status: 'cancelling' }))).toBe(false);
    expect(agentMarkPulses(one({ status: 'running', quietSeconds: 600 }))).toBe(false);
    expect(agentMarkPulses(one({ status: 'completed' }))).toBe(false);
  });
});

describe('decision-model work is a circle', () => {
  it('draws a subagent whose own model is a decision model as a circle, any other as a square', () => {
    expect(agentMarkShape(one({ providerName: 'anthropic' }))).toBe('square');
    expect(agentMarkShape(one({ providerName: 'ollama_decision', modelName: 'tev1:0.8b' }))).toBe(
      'circle'
    );
    expect(agentMarkShape(one({ providerName: 'baseten_decision', modelName: 'joey-1' }))).toBe(
      'circle'
    );
    expect(agentMarkShape(one({ providerName: null }))).toBe('square');
  });

  it('fills a gate by its state: passed, needs you, not passed, skipped, not asked', () => {
    expect(gateMarkKind(gate({ lastPassed: true }))).toBe('done');
    expect(gateMarkKind(gate({ lastPassed: false }))).toBe('failed');
    expect(gateMarkKind(gate({ needsPerson: true, blocked: true }))).toBe('waiting');
    expect(gateMarkKind(gate({ blocked: true }))).toBe('failed');
    expect(gateMarkKind(gate({ personDecision: 'pass', lastPassed: false }))).toBe('done');
    expect(gateMarkKind(gate({ personDecision: 'skip' }))).toBe('idle');
    expect(gateMarkKind(gate({}))).toBe('queued');
  });

  it('fills a Decisions row by its state, and never draws a quiet monitor as failed', () => {
    expect(DECISION_STATE_MARKS).toEqual({
      fired: 'waiting',
      quiet: 'done',
      pending: 'queued',
      off: 'idle',
      error: 'failed',
      done: 'done',
    });
  });

  it('names the monitors that also watch the subagents, while they are on', () => {
    const settings = parseDecisionSettings({
      defaultModel: null,
      monitors: [
        {
          id: 'scope',
          name: 'Scope drift',
          question: { instructions: 'Off task?', kind: { binary: {} } },
          threshold: 0.8,
          applyToSubagents: true,
        },
        {
          id: 'cost',
          name: 'Cost check',
          question: { instructions: 'Over budget?', kind: { binary: {} } },
          threshold: 0.8,
        },
        {
          id: 'off',
          name: 'Switched off',
          question: { instructions: 'Anything?', kind: { binary: {} } },
          threshold: 0.8,
          applyToSubagents: true,
          enabled: false,
        },
      ],
    });
    expect(subagentWatchers(settings)).toEqual(['Scope drift']);
    expect(subagentWatchers(null)).toEqual([]);
  });
});

describe('the head list: who needs a look first, then the newest', () => {
  const roster = rows(
    wireRow({ sessionId: 'done-old', status: 'completed', endedAt: '2026-11-04T10:01:00Z' }),
    wireRow({ sessionId: 'run', status: 'running', quietSeconds: 4 }),
    wireRow({ sessionId: 'failed', status: 'failed', endedAt: '2026-11-04T10:02:00Z' }),
    wireRow({ sessionId: 'done-new', status: 'completed', endedAt: '2026-11-04T10:09:00Z' }),
    wireRow({ sessionId: 'asks', status: 'blocked' }),
    wireRow({ sessionId: 'quiet', status: 'running', quietSeconds: 900 }),
    wireRow({ sessionId: 'done-mid', status: 'completed', endedAt: '2026-11-04T10:05:00Z' })
  );

  it('ranks waiting and stalled, then failed, then running, then the rest', () => {
    expect(roster.map(headRank)).toEqual([3, 2, 1, 3, 0, 0, 3]);
    expect(inHeadOrder(roster).map((row) => row.sessionId)).toEqual([
      'asks',
      'quiet',
      'failed',
      'run',
      'done-new',
      'done-mid',
      'done-old',
    ]);
  });

  it('shows five, then keeps the rest for Show more', () => {
    expect(AGENT_HEAD_ROWS).toBe(5);
    const { head, rest } = splitHead(roster);
    expect(head.map((row) => row.sessionId)).toEqual([
      'asks',
      'quiet',
      'failed',
      'run',
      'done-new',
    ]);
    expect(rest.map((row) => row.sessionId)).toEqual(['done-mid', 'done-old']);
  });

  it('keeps a row whose card is kept, or that holds focus, in the head past the five', () => {
    const { head, rest } = splitHead(roster, new Set(['done-old']));
    expect(head.map((row) => row.sessionId)).toContain('done-old');
    expect(rest.map((row) => row.sessionId)).toEqual(['done-mid']);
  });
});

describe('groups follow the plan', () => {
  const plan = {
    title: 'Review',
    goal: null,
    revision: 1,
    phases: [
      wirePhase({ id: 'audit', title: 'Audit', agentIds: ['a1', 'a2'] }),
      wirePhase({ id: 'verify', title: 'Verify' }),
      wirePhase({ id: 'draft', title: 'Draft' }),
    ],
  };
  const wire = [
    wireRow({ sessionId: 'd1', phaseId: 'draft', createdAt: '2026-11-04T10:00:03Z' }),
    wireRow({ sessionId: 'a1', createdAt: '2026-11-04T10:00:00Z' }),
    wireRow({ sessionId: 'loose', createdAt: '2026-11-04T10:00:04Z' }),
    wireRow({ sessionId: 'a2', createdAt: '2026-11-04T10:00:01Z' }),
  ];

  it('makes one group per phase in plan order, empty phases too, then Other', () => {
    const parsed = parseSessionSubagents(wireSubagents(wire, plan))!;
    const groups = agentGroups(parsed.plan, inSpawnOrder(parsed.subagents));
    expect(groups.map((group) => group.key)).toEqual([
      'phase:audit',
      'phase:verify',
      'phase:draft',
      'other',
    ]);
    expect(groups.map((group) => group.rows.map((row) => row.sessionId))).toEqual([
      ['a1', 'a2'],
      [],
      ['d1'],
      ['loose'],
    ]);
  });

  it('makes one group of a swarm with no plan, and none of an empty roster', () => {
    const parsed = parseSessionSubagents(wireSubagents(wire))!;
    const groups = agentGroups(null, inSpawnOrder(parsed.subagents));
    expect(groups.map((group) => group.key)).toEqual(['all']);
    expect(groups[0].rows.map((row) => row.sessionId)).toEqual(['a1', 'a2', 'd1', 'loose']);
    expect(agentGroups(null, [])).toEqual([]);
  });
});
