import { isDecisionProvider } from '../decisions/decisionWire';
import {
  isQuiet,
  rowsByPhase,
  type OrchestrationSnapshot,
  type PhaseSnapshot,
  type SubagentRow,
  type SubagentRunStatus,
} from '../../utils/sessionSubagents';
import type { DecisionSettings, PhaseGateSnapshot } from '../../utils/sessionDecisions';
import type { SummaryDecisionState } from './summaryDecisionRows';

/**
 * THE AGENT GRID'S NOTATION (owner request, 2026-11-11), the one the Chat summary's
 * roster, its phase gates, its Decisions rows and the terminal's `/subagents` view
 * share: one small mark per piece of work, its SHAPE saying what kind of model did
 * it and its FILL saying where it stands, with the status ink on top.
 *
 * - Shape: a square for a language-model agent, a circle for decision-model work
 *   (a subagent whose own model is a decision model, a monitor, a `decide`
 *   fan-out or vote, a phase gate).
 * - Fill, so status never depends on colour alone: hollow (waiting to start),
 *   hollow with a core (running), half full (stalled, or waiting for the person),
 *   solid (finished), solid with a cross cut out (failed), hollow with a bar (no
 *   outcome: unknown, skipped, off).
 * - Ink: running the info blue, finished a muted success tint, failed the danger
 *   red, waiting the warning amber, the rest muted (`summaryAgentGrid.css`).
 *
 * Pure: no React and no DOM, so every rule here is tested without rendering.
 */

export const AGENT_MARK_KINDS = ['queued', 'running', 'waiting', 'done', 'failed', 'idle'] as const;
export type AgentMarkKind = (typeof AGENT_MARK_KINDS)[number];

/** A square is a language-model agent; a circle is decision-model work. */
export type AgentMarkShape = 'square' | 'circle';

/**
 * Each run status's mark. Keyed by status, so a status the daemon adds fails the
 * build here. A stopped (`incomplete`) or `interrupted` run ended without its
 * result and needs a look, so it wears the waiting fill and ink beside a run that
 * asked a question (`blocked`); its own word ("Stopped", "Interrupted") is in its
 * name and its card. A child stopping (`cancelling`) is still active.
 */
export const AGENT_STATUS_MARKS: Record<SubagentRunStatus, AgentMarkKind> = {
  queued: 'queued',
  running: 'running',
  cancelling: 'running',
  completed: 'done',
  blocked: 'waiting',
  incomplete: 'waiting',
  interrupted: 'waiting',
  failed: 'failed',
  unknown: 'idle',
};

/** A row's mark: its status's, except that an active child gone quiet reads as stalled. */
export function agentMarkKind(row: Pick<SubagentRow, 'status' | 'quietSeconds'>): AgentMarkKind {
  if (isQuiet(row as SubagentRow)) return 'waiting';
  return AGENT_STATUS_MARKS[row.status];
}

/** A subagent whose own model is a decision model is decision-model work: a circle. */
export function agentMarkShape(row: Pick<SubagentRow, 'providerName'>): AgentMarkShape {
  return isDecisionProvider(row.providerName) ? 'circle' : 'square';
}

/** Whether the mark's core pulses: a child that is running, not one that is stopping. */
export function agentMarkPulses(row: Pick<SubagentRow, 'status' | 'quietSeconds'>): boolean {
  return row.status === 'running' && agentMarkKind(row) === 'running';
}

// ---------------------------------------------------------------------------
// Size: the marks scale with the count
// ---------------------------------------------------------------------------

/**
 * `rows`: four agents or fewer in the chat. Every one is a two-line row led by a
 * 14px mark (the to-do list's glyph size, so a few agents read larger than a grid's
 * squares), and nothing draws a grid. `grid`: five to 99. Each group draws its
 * grid of 10px squares, then its first rows. `dense`: 100 or more, 7px squares
 * in a tight block.
 */
export type AgentGridSize = 'rows' | 'grid' | 'dense';

/** Up to this many agents in the chat, each is a two-line row and no group draws a grid. */
export const AGENT_ROWS_MAX = 4;

/** From this many agents in the chat, the grid's squares get smaller. */
export const AGENT_DENSE_FROM = 100;

/** A grid group lists this many agents under its grid before "Show N more". */
export const AGENT_HEAD_ROWS = 5;

/** A mark's side in CSS pixels, per size. Rows of the grid sizes lead with the grid's square. */
export const AGENT_MARK_PX: Record<AgentGridSize, number> = { rows: 14, grid: 10, dense: 7 };

/** The gap between two squares of a grid, per size. */
export const AGENT_GRID_GAP_PX: Record<AgentGridSize, number> = { rows: 4, grid: 4, dense: 2 };

export function agentGridSize(total: number): AgentGridSize {
  if (total <= AGENT_ROWS_MAX) return 'rows';
  return total >= AGENT_DENSE_FROM ? 'dense' : 'grid';
}

/** The size a row's own mark takes: 14px in a chat of rows, else the grid's 10px. */
export function agentRowMarkPx(size: AgentGridSize): number {
  return size === 'rows' ? AGENT_MARK_PX.rows : AGENT_MARK_PX.grid;
}

/**
 * Whether a group of `count` agents draws its grid: never in a chat of rows (four
 * agents or fewer), and in a larger chat every group that has an agent, however
 * small, so each group reads in the same notation and a folded one can still
 * show its live grid.
 */
export function groupDrawsGrid(count: number, size: AgentGridSize): boolean {
  return size !== 'rows' && count > 0;
}

// ---------------------------------------------------------------------------
// The head list: who needs a look first, then the newest
// ---------------------------------------------------------------------------

/**
 * A row's place in the head list, lower first: a run waiting for the person (or
 * stalled), a failure, a running child, then everything else by when it last did
 * something, newest first.
 */
export function headRank(row: SubagentRow): number {
  switch (agentMarkKind(row)) {
    case 'waiting':
      return 0;
    case 'failed':
      return 1;
    case 'running':
      return 2;
    default:
      return 3;
  }
}

function lastActivity(row: SubagentRow): number {
  for (const stamp of [row.endedAt, row.updatedAt, row.createdAt]) {
    const time = stamp ? Date.parse(stamp) : Number.NaN;
    if (Number.isFinite(time)) return time;
  }
  return Number.NEGATIVE_INFINITY;
}

/**
 * `rows` (given in spawn order) in head order: by rank, and within a rank, the
 * ones that need a look keep spawn order while the rest go newest first. Ties
 * keep the order they came in, so a poll that changes nothing moves nothing.
 */
export function inHeadOrder(rows: readonly SubagentRow[]): SubagentRow[] {
  return rows
    .map((row, index) => ({ row, index, rank: headRank(row), at: lastActivity(row) }))
    .sort((a, b) => a.rank - b.rank || (a.rank === 3 ? b.at - a.at : 0) || a.index - b.index)
    .map(({ row }) => row);
}

export interface HeadSplit {
  /** The rows drawn under the grid, in head order. */
  head: SubagentRow[];
  /** The rows "Show N more" adds, in head order. */
  rest: SubagentRow[];
}

/**
 * The first `limit` rows in head order, plus any row in `keep` (a row whose card
 * is kept, or that holds focus) past them, so a poll that moves a row down the
 * ranks never unmounts it under the person.
 */
export function splitHead(
  rows: readonly SubagentRow[],
  keep: ReadonlySet<string> = new Set(),
  limit = AGENT_HEAD_ROWS
): HeadSplit {
  const ordered = inHeadOrder(rows);
  const head: SubagentRow[] = [];
  const rest: SubagentRow[] = [];
  ordered.forEach((row, index) => {
    if (index < limit || keep.has(row.sessionId)) head.push(row);
    else rest.push(row);
  });
  return { head, rest };
}

// ---------------------------------------------------------------------------
// Groups: one per phase in plan order, or one for a swarm with no plan
// ---------------------------------------------------------------------------

export interface AgentGroup {
  /** `phase:<id>`, `other` (in no phase the plan knows) or `all` (no plan). */
  key: string;
  phase: PhaseSnapshot | null;
  /** The rows in spawn order (`inSpawnOrder`, the caller's). */
  rows: SubagentRow[];
}

/**
 * The roster's groups: with a plan, every phase in plan order (one that used no
 * subagents too) and then the subagents in no phase it knows; without one, the
 * whole roster as one group.
 */
export function agentGroups(
  plan: OrchestrationSnapshot | null,
  rows: readonly SubagentRow[]
): AgentGroup[] {
  if (!plan) return rows.length > 0 ? [{ key: 'all', phase: null, rows: [...rows] }] : [];
  const { phases, unphased } = rowsByPhase(plan, rows);
  const groups: AgentGroup[] = plan.phases.map((phase) => ({
    key: `phase:${phase.id}`,
    phase,
    rows: phases.get(phase.id) ?? [],
  }));
  if (unphased.length > 0) groups.push({ key: 'other', phase: null, rows: unphased });
  return groups;
}

// ---------------------------------------------------------------------------
// Decision-model work: the gates, the Decisions rows, the monitors that watch
// ---------------------------------------------------------------------------

/**
 * A phase gate's mark (always a circle): passed, or passed by the person, is
 * finished; a pass not reached is failed, as is a gate blocked with nobody asked;
 * a gate that needs the person waits; a skip is idle; not asked yet is hollow.
 */
export function gateMarkKind(gate: PhaseGateSnapshot): AgentMarkKind {
  if (gate.personDecision === 'pass') return 'done';
  if (gate.personDecision === 'skip') return 'idle';
  if (gate.needsPerson) return 'waiting';
  if (gate.blocked) return 'failed';
  if (gate.lastPassed === true) return 'done';
  if (gate.lastPassed === false) return 'failed';
  return 'queued';
}

/**
 * Each Decisions row's mark (always a circle). A monitor that fired is waiting for
 * a look; one that checked and stayed quiet is finished, never failed (R1: a quiet
 * monitor did nothing wrong); one not checked yet is hollow; one switched off is
 * idle; an error is failed; `decide` batches and a decision chat's answers that
 * landed are finished.
 */
export const DECISION_STATE_MARKS: Record<SummaryDecisionState, AgentMarkKind> = {
  fired: 'waiting',
  quiet: 'done',
  pending: 'queued',
  off: 'idle',
  error: 'failed',
  done: 'done',
};

/**
 * The names of the chat's monitors that watch its subagents too: a person's
 * monitor copied onto every subagent the chat starts (`applyToSubagents`), while
 * it is on. A subagent's card says it is watched by them.
 */
export function subagentWatchers(settings: DecisionSettings | null | undefined): string[] {
  return (settings?.monitors ?? [])
    .filter((monitor) => monitor.applyToSubagents === true && monitor.enabled)
    .map((monitor) => monitor.name)
    .filter((name) => name.trim().length > 0);
}
