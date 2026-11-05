import type { Message } from '../api';
import { navigateToRoute } from './navigationUtils';

/**
 * A chat's subagents and its orchestration plan, as `GET /sessions/{id}/subagents`
 * serves them.
 *
 * The shapes mirror the daemon's contract one to one: `SessionSubagents` and
 * `SubagentRow` in `crates/biorouter/src/agents/subagent_roster.rs`, and
 * `OrchestrationSnapshot`, `PhaseSnapshot` and `PhaseCounts` in
 * `crates/biorouter/src/agents/orchestration_plan.rs`. Fields are camelCase and
 * enum values snake_case on the wire. Rust's `Option<T>` arrives as `null`.
 *
 * The parser is defensive in one direction only: it never drops a row a person
 * should see. An unrecognised status degrades to `unknown` (the to-do list's
 * lesson, `sessionTodos.ts`: a status this build has not heard of made the item
 * vanish while the daemon still tracked it). A row with no session id is the one
 * exception, because nothing could open it or tell it apart from the next.
 */

export const SUBAGENT_RUN_STATUSES = [
  'queued',
  'running',
  'cancelling',
  'completed',
  'blocked',
  'incomplete',
  'failed',
  'interrupted',
  'unknown',
] as const;

export type SubagentRunStatus = (typeof SUBAGENT_RUN_STATUSES)[number];

export const PHASE_KINDS = [
  'plan',
  'research',
  'execution',
  'implementation',
  'verification',
  'testing',
  'auditing',
  'review',
  'meeting',
  'synthesis',
  'other',
] as const;

export type PhaseKind = (typeof PHASE_KINDS)[number];

export const PHASE_STATUSES = [
  'pending',
  'in_progress',
  'completed',
  'blocked',
  'skipped',
] as const;

export type PhaseStatus = (typeof PHASE_STATUSES)[number];

export type SubagentPrivacyTier = 'public' | 'private';

export interface SubagentRow {
  sessionId: string;
  /** The session name without its `Subagent: ` prefix. */
  name: string;
  role: string | null;
  phaseId: string | null;
  status: SubagentRunStatus;
  /** The child's agent-loop phase while it runs here (`tool_batch`, ...). */
  loopPhase: string | null;
  /** Seconds since the child's last recorded activity, while it is active. */
  quietSeconds: number | null;
  createdAt: string;
  updatedAt: string;
  endedAt: string | null;
  elapsedSeconds: number | null;
  providerName: string | null;
  modelName: string | null;
  extensions: string[];
  skills: string[];
  knowledgeBases: string[];
  /** `null` when the wire carried a tier this build does not know: drawn as not yet known. */
  privacyTier: SubagentPrivacyTier | null;
  totalTokens: number | null;
  messageCount: number;
  toolCallCount: number;
  /** First line of the final summary, the error, or the blocked question. */
  resultLine: string | null;
}

export interface PhaseCounts {
  total: number;
  done: number;
  running: number;
  blocked: number;
  failed: number;
  quiet: number;
}

export interface PhaseSnapshot {
  id: string;
  title: string;
  goal: string | null;
  kind: PhaseKind;
  status: PhaseStatus;
  dependsOn: string[];
  agentIds: string[];
  counts: PhaseCounts;
  handoff: string | null;
  startedAt: string | null;
  finishedAt: string | null;
}

export interface OrchestrationSnapshot {
  title: string;
  goal: string | null;
  revision: number;
  phases: PhaseSnapshot[];
}

export interface SessionSubagents {
  parentSessionId: string;
  /** Oldest first (spawn order). */
  subagents: SubagentRow[];
  plan: OrchestrationSnapshot | null;
}

function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function text(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

function count(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) && value > 0 ? Math.floor(value) : 0;
}

function optionalCount(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0
    ? Math.floor(value)
    : null;
}

function names(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  const seen = new Set<string>();
  const out: string[] = [];
  for (const entry of value) {
    const name = text(entry);
    if (name && !seen.has(name)) {
      seen.add(name);
      out.push(name);
    }
  }
  return out;
}

function oneOf<T extends string>(values: readonly T[], value: unknown, fallback: T): T {
  return values.includes(value as T) ? (value as T) : fallback;
}

export function parseSubagentRow(value: unknown): SubagentRow | null {
  const row = record(value);
  const sessionId = text(row?.sessionId);
  if (!row || !sessionId) return null;
  const tier = row.privacyTier;
  return {
    sessionId,
    name: text(row.name) ?? '',
    role: text(row.role),
    phaseId: text(row.phaseId),
    status: oneOf(SUBAGENT_RUN_STATUSES, row.status, 'unknown'),
    loopPhase: text(row.loopPhase),
    quietSeconds: optionalCount(row.quietSeconds),
    createdAt: text(row.createdAt) ?? '',
    updatedAt: text(row.updatedAt) ?? '',
    endedAt: text(row.endedAt),
    elapsedSeconds: optionalCount(row.elapsedSeconds),
    providerName: text(row.providerName),
    modelName: text(row.modelName),
    extensions: names(row.extensions),
    skills: names(row.skills),
    knowledgeBases: names(row.knowledgeBases),
    privacyTier: tier === 'public' || tier === 'private' ? tier : null,
    totalTokens: optionalCount(row.totalTokens),
    messageCount: count(row.messageCount),
    toolCallCount: count(row.toolCallCount),
    resultLine: text(row.resultLine),
  };
}

function parseCounts(value: unknown): PhaseCounts {
  const counts = record(value);
  return {
    total: count(counts?.total),
    done: count(counts?.done),
    running: count(counts?.running),
    blocked: count(counts?.blocked),
    failed: count(counts?.failed),
    quiet: count(counts?.quiet),
  };
}

function parsePhase(value: unknown): PhaseSnapshot | null {
  const phase = record(value);
  const id = text(phase?.id);
  if (!phase || !id) return null;
  return {
    id,
    title: text(phase.title) ?? id,
    goal: text(phase.goal),
    kind: oneOf(PHASE_KINDS, phase.kind, 'other'),
    status: oneOf(PHASE_STATUSES, phase.status, 'pending'),
    dependsOn: names(phase.dependsOn),
    agentIds: names(phase.agentIds),
    counts: parseCounts(phase.counts),
    handoff: text(phase.handoff),
    startedAt: text(phase.startedAt),
    finishedAt: text(phase.finishedAt),
  };
}

export function parseOrchestrationSnapshot(value: unknown): OrchestrationSnapshot | null {
  const plan = record(value);
  if (!plan) return null;
  const seen = new Set<string>();
  const phases: PhaseSnapshot[] = [];
  for (const entry of Array.isArray(plan.phases) ? plan.phases : []) {
    const phase = parsePhase(entry);
    if (phase && !seen.has(phase.id)) {
      seen.add(phase.id);
      phases.push(phase);
    }
  }
  return {
    title: text(plan.title) ?? '',
    goal: text(plan.goal),
    revision: count(plan.revision),
    phases,
  };
}

/**
 * The route's answer, or `null` when it is not a `SessionSubagents` object at
 * all. Rows are deduplicated by session id (first wins) and keep spawn order.
 */
export function parseSessionSubagents(value: unknown): SessionSubagents | null {
  const body = record(value);
  if (!body) return null;
  const seen = new Set<string>();
  const subagents: SubagentRow[] = [];
  for (const entry of Array.isArray(body.subagents) ? body.subagents : []) {
    const row = parseSubagentRow(entry);
    if (row && !seen.has(row.sessionId)) {
      seen.add(row.sessionId);
      subagents.push(row);
    }
  }
  return {
    parentSessionId: text(body.parentSessionId) ?? '',
    subagents,
    plan:
      body.plan === null || body.plan === undefined ? null : parseOrchestrationSnapshot(body.plan),
  };
}

// ---------------------------------------------------------------------------
// Status groups and counts
// ---------------------------------------------------------------------------

/** Still doing, or about to do, work. Mirrors `SubagentRunStatus::is_active`. */
export function isActiveStatus(status: SubagentRunStatus): boolean {
  return status === 'queued' || status === 'running' || status === 'cancelling';
}

export const SUBAGENT_GROUPS = ['running', 'attention', 'done'] as const;
export type SubagentGroup = (typeof SUBAGENT_GROUPS)[number];

/**
 * The three groups the dense list draws.
 *
 * `attention` is every way a run can end without its result: blocked on a
 * question, failed, ended short (`incomplete`) or cut off by its process going
 * away (`interrupted`). `unknown` is a child that is not running here and left
 * no outcome record (it predates them), so it sits with the finished ones rather
 * than claiming a problem nobody recorded.
 */
export function subagentGroup(status: SubagentRunStatus): SubagentGroup {
  if (isActiveStatus(status)) return 'running';
  if (
    status === 'blocked' ||
    status === 'failed' ||
    status === 'incomplete' ||
    status === 'interrupted'
  )
    return 'attention';
  return 'done';
}

/**
 * How long an active child may go without recorded activity before a row calls
 * it quiet. A heuristic, labelled as "quiet", never as "stalled": a long model
 * call or a slow tool is silent too.
 *
 * ⚠ Must equal `QUIET_AFTER_SECONDS` in `orchestration_plan.rs`, which decides a
 * phase's `counts.quiet`; a row and its phase would otherwise disagree.
 */
export const QUIET_AFTER_SECONDS = 180;

export function isQuiet(row: SubagentRow): boolean {
  return (
    isActiveStatus(row.status) &&
    row.quietSeconds !== null &&
    row.quietSeconds >= QUIET_AFTER_SECONDS
  );
}

export interface SubagentCounts {
  total: number;
  running: number;
  attention: number;
  done: number;
  quiet: number;
}

export function subagentCounts(rows: readonly SubagentRow[]): SubagentCounts {
  const counts: SubagentCounts = {
    total: rows.length,
    running: 0,
    attention: 0,
    done: 0,
    quiet: 0,
  };
  for (const row of rows) {
    counts[subagentGroup(row.status)] += 1;
    if (isQuiet(row)) counts.quiet += 1;
  }
  return counts;
}

export function hasActiveSubagent(data: SessionSubagents | null | undefined): boolean {
  return !!data?.subagents.some((row) => isActiveStatus(row.status));
}

/** The rows of `rows` in `group`, keeping spawn order. */
export function rowsInGroup(rows: readonly SubagentRow[], group: SubagentGroup): SubagentRow[] {
  return rows.filter((row) => subagentGroup(row.status) === group);
}

// ---------------------------------------------------------------------------
// Layout
// ---------------------------------------------------------------------------

/** Up to this many subagents get comfortable two-line rows; more get dense ones. */
export const COMFORTABLE_MAX_ROWS = 4;

/** A dense group or a phase shows at most this many rows before "Show all N". */
export const GROUP_ROW_LIMIT = 12;

export type SubagentListLayout = 'comfortable' | 'dense';

export function subagentListLayout(count: number): SubagentListLayout {
  return count <= COMFORTABLE_MAX_ROWS ? 'comfortable' : 'dense';
}

/**
 * Each phase's subagents, and the ones in no phase the plan knows.
 *
 * A row belongs to the first phase that names it, by the phase's own list or by
 * the row's `phaseId`. A row whose phase is not in the plan (removed, or a spawn
 * the plan has not recorded yet) is listed under "Other subagents" rather than
 * hidden.
 */
export function rowsByPhase(
  plan: OrchestrationSnapshot,
  rows: readonly SubagentRow[]
): { phases: Map<string, SubagentRow[]>; unphased: SubagentRow[] } {
  const phases = new Map<string, SubagentRow[]>(plan.phases.map((phase) => [phase.id, []]));
  const unphased: SubagentRow[] = [];
  for (const row of rows) {
    const phase =
      plan.phases.find((candidate) => candidate.agentIds.includes(row.sessionId)) ??
      plan.phases.find((candidate) => candidate.id === row.phaseId);
    if (phase) phases.get(phase.id)!.push(row);
    else unphased.push(row);
  }
  return { phases, unphased };
}

/**
 * Whether a phase's disclosure starts open. The running phase (and a blocked
 * one, which needs a look) is open. Finished, skipped and pending phases fold
 * once the plan has more than three phases, so a long plan reads as its
 * progress rather than as every child it ever ran.
 */
export function phaseOpenByDefault(phase: PhaseSnapshot, phaseCount: number): boolean {
  if (phase.status === 'in_progress' || phase.status === 'blocked') return true;
  return phaseCount <= 3;
}

export function completedPhaseCount(plan: OrchestrationSnapshot): number {
  return plan.phases.filter((phase) => phase.status === 'completed').length;
}

// ---------------------------------------------------------------------------
// Labels
// ---------------------------------------------------------------------------

/** The label a row leads with: the role the parent gave, else the session name. */
export function subagentTitle(row: SubagentRow): string {
  return row.role ?? (row.name || 'Subagent');
}

const MODEL_LABEL_MAX = 24;

/**
 * A model name short enough for a 240px row: no provider path
 * (`accounts/fireworks/models/`, `us.anthropic.`), no release date suffix
 * (`-20251101`, `@20250514`) and no `:latest`. Falls back to the provider's name
 * when the row has no model, and to `null` when it has neither.
 */
export function shortModelLabel(
  modelName: string | null,
  providerName?: string | null
): string | null {
  const raw = modelName?.trim();
  if (!raw) return providerName?.trim() || null;
  let label = raw.slice(raw.lastIndexOf('/') + 1);
  // Bedrock-style ids: `us.anthropic.claude-sonnet-4-5-20250929-v1:0`.
  const dotted = label.match(
    /^(?:[a-z]{2}\.)?(?:anthropic|meta|amazon|cohere|mistral|ai21)\.(.+)$/
  );
  if (dotted) label = dotted[1];
  label = label
    .replace(/:latest$/, '')
    .replace(/-v\d+:\d+$/, '')
    .replace(/[-@]20\d{6}$/, '')
    .replace(/-\d{4}-\d{2}-\d{2}$/, '');
  if (!label) label = raw;
  return label.length > MODEL_LABEL_MAX ? `${label.slice(0, MODEL_LABEL_MAX - 1)}…` : label;
}

/** `45s`, `4m`, `1h 5m`, `2d`. `null` in, `null` out. */
export function formatElapsed(seconds: number | null | undefined): string | null {
  if (seconds === null || seconds === undefined || !Number.isFinite(seconds) || seconds < 0)
    return null;
  const whole = Math.floor(seconds);
  if (whole < 60) return `${whole}s`;
  const minutes = Math.floor(whole / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    const rest = minutes % 60;
    return rest ? `${hours}h ${rest}m` : `${hours}h`;
  }
  return `${Math.floor(hours / 24)}d`;
}

/** `opus-4-5 · 3 ext · 1 skill · 2 KB · 4m`: the second line of a comfortable row. */
export function subagentSetupLine(row: SubagentRow): string {
  const parts = [
    shortModelLabel(row.modelName, row.providerName),
    row.extensions.length > 0 ? `${row.extensions.length} ext` : null,
    row.skills.length > 0
      ? `${row.skills.length} ${row.skills.length === 1 ? 'skill' : 'skills'}`
      : null,
    row.knowledgeBases.length > 0 ? `${row.knowledgeBases.length} KB` : null,
    formatElapsed(row.elapsedSeconds),
  ];
  return parts.filter((part): part is string => !!part).join(' · ');
}

// ---------------------------------------------------------------------------
// When to refetch
// ---------------------------------------------------------------------------

/**
 * Tools whose answer can change a chat's roster or plan: a spawn, the plan tool,
 * a watch (it reports children finishing) and a close (it stops one). Matched on
 * the name's last `__` segment, so the workspace extension's own spelling
 * (`workspace__subagent`), a bridged coding agent's relay spelling and a bare
 * `subagent` all count.
 */
const ROSTER_TOOLS = new Set([
  'subagent',
  'workspace_orchestrate',
  'workspace_watch',
  'workspace_close',
]);

/** Must match `TOOL_CALLS_META_KEY` in `code_execution_extension.rs` (see `sessionTodos.ts`). */
const EXECUTED_CALLS_META_KEY = 'biorouter/tool-calls';

function isRosterTool(name: unknown): boolean {
  if (typeof name !== 'string' || !name) return false;
  const segments = name.split('__');
  return ROSTER_TOOLS.has(segments[segments.length - 1]);
}

function executedRosterCalls(resultValue: unknown): number {
  const calls = record(record(resultValue)?._meta)?.[EXECUTED_CALLS_META_KEY];
  if (!Array.isArray(calls)) return 0;
  return calls.filter((candidate) => isRosterTool(record(candidate)?.tool)).length;
}

/**
 * A string that changes whenever a roster tool call ANSWERS, and only then.
 *
 * Keyed on answered calls, not proposed ones: a spawn's child exists once its
 * response lands. Any answer counts, an error included, because a refused spawn
 * is still worth one cheap re-read and a partly applied plan call may have
 * changed something. Streamed prose and other tools never move it, and replaying
 * the same transcript derives the identical string.
 */
export function subagentRosterRevision(messages: readonly Message[]): string {
  const requests = new Set<string>();
  for (const message of messages) {
    for (const content of message.content) {
      if (content.type === 'toolRequest') {
        const call = record(content.toolCall.value);
        if (isRosterTool(call?.name)) requests.add(content.id);
      }
    }
  }
  const answered = new Set<string>();
  for (const message of messages) {
    for (const content of message.content) {
      if (content.type !== 'toolResponse') continue;
      const executed = executedRosterCalls(content.toolResult.value);
      if (executed > 0) answered.add(JSON.stringify([content.id, executed]));
      if (requests.has(content.id)) answered.add(content.id);
    }
  }
  return JSON.stringify([...answered].sort());
}

/** `true` once the transcript holds any answered roster call. */
export function rosterRevisionHasCalls(revision: string): boolean {
  return revision !== '' && revision !== '[]';
}

// ---------------------------------------------------------------------------
// Opening a subagent
// ---------------------------------------------------------------------------

export interface OpenSubagentTarget {
  /** `useChatGroups()?.dispatch`, or `undefined` outside a provider. */
  dispatch?: (action: { type: 'openTab'; payload: { sessionId: string; title?: string } }) => void;
  /** The current route's pathname. */
  pathname: string;
  navigate: Parameters<typeof navigateToRoute>[0];
}

/**
 * Open a subagent as a tab and make it the active one.
 *
 * On `/pair` the tab strip's reducer does it in place; it dedupes by session id,
 * so a child already open anywhere is activated rather than opened twice. From
 * any other route (or with no tab strip) it goes through `/pair` the way the
 * sidebar does, with the name riding along so the tab is born titled.
 */
export function openSubagentTab(
  target: OpenSubagentTarget,
  sessionId: string,
  title?: string
): void {
  const name = title?.trim() || undefined;
  if (target.dispatch && target.pathname === '/pair') {
    target.dispatch({ type: 'openTab', payload: { sessionId, ...(name ? { title: name } : {}) } });
    return;
  }
  navigateToRoute(
    target.navigate,
    `/pair?resumeSessionId=${encodeURIComponent(sessionId)}`,
    name ? { title: name } : undefined
  );
}
