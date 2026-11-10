import {
  decisionSourceRows,
  type DecisionAggregate,
  type DecisionOrigin,
  type DecisionSettings,
  type SessionDecisionSummary,
} from '../../utils/sessionDecisions';

/**
 * The Chat summary's Decisions rows (live QA 2026-11-10, A-1 and F-1): what the
 * section draws, reshaped from the daemon's per-source aggregates and then read
 * through the one row rule every decision surface shares (`decisionSourceRows`,
 * unchanged, called with no gates).
 *
 * 1. One row per monitor that exists NOW, in settings order. A monitor no call
 *    has reached yet still has its row ("Not checked yet").
 * 2. A call that asked several monitors at once counts on EACH monitor's row.
 *    The daemon joins such a call's monitor ids with `+`
 *    (`PlannedRequest::origin_id`, `decision/monitor.rs`), and a merged
 *    "A + B" row is never drawn. Each copy carries the call's count and errors,
 *    and never its answer or probability: one combined record cannot say which
 *    monitor those belong to, so a monitor's state comes from its own calls and
 *    from the newest turn marker's `fired` set. The copy's time is left out for
 *    the same reason, so it never takes the place of a monitor's own last answer.
 * 3. A removed monitor leaves no row; its checks stay in the decision history.
 * 4. No gate rows: a gate's one place in the summary is its chip on its phase
 *    row (F-1, "never its own row").
 * 5. The `decide` row (every batch together) and the decision chat's row follow.
 */

/** What a row's trailing word says, by shape and word (R1: never passed or failed). */
export type SummaryDecisionState = 'fired' | 'quiet' | 'pending' | 'off' | 'error' | 'done';

export interface SummaryDecisionRow {
  /** `monitor:<id>`, `tool`, `chat`, `adhoc`. */
  key: string;
  origin: DecisionOrigin;
  /** The history filter's id: the monitor's own id, `null` for the whole origin. */
  originId: string | null;
  label: string;
  count: number;
  errors: number;
  state: SummaryDecisionState;
}

/** The ids a monitor aggregate stands for: one, or each of a `+`-joined call's. */
export function monitorIdsOf(originId: string | null): string[] {
  if (!originId) return [];
  return originId
    .split('+')
    .map((id) => id.trim())
    .filter(Boolean);
}

function reshape(
  summary: SessionDecisionSummary | null,
  settings: DecisionSettings | null
): SessionDecisionSummary | null {
  if (!summary) return null;
  const existing = new Set((settings?.monitors ?? []).map((monitor) => monitor.id));
  const aggregates: DecisionAggregate[] = [];
  for (const aggregate of summary.aggregates) {
    if (aggregate.origin === 'gate') continue;
    if (aggregate.origin !== 'monitor') {
      aggregates.push(aggregate);
      continue;
    }
    const ids = monitorIdsOf(aggregate.originId);
    if (ids.length === 1) {
      if (existing.has(ids[0])) aggregates.push({ ...aggregate, originId: ids[0] });
      continue;
    }
    for (const id of ids) {
      if (!existing.has(id)) continue;
      aggregates.push({
        ...aggregate,
        originId: id,
        label: '',
        passed: 0,
        failed: 0,
        lastAt: '',
        lastAnswer: null,
        lastPassProbability: null,
      });
    }
  }
  return { ...summary, aggregates };
}

/** The `fired` set with any `+`-joined id split into its monitors. */
function splitFired(fired: ReadonlySet<string> | undefined): Set<string> {
  const out = new Set<string>();
  for (const id of fired ?? []) for (const one of monitorIdsOf(id)) out.add(one);
  return out;
}

export function summaryDecisionRows(
  summary: SessionDecisionSummary | null,
  settings: DecisionSettings | null,
  options: { fired?: ReadonlySet<string> } = {}
): SummaryDecisionRow[] {
  const rows = decisionSourceRows(reshape(summary, settings), settings, {
    fired: splitFired(options.fired),
  });
  const monitors = new Map((settings?.monitors ?? []).map((monitor) => [monitor.id, monitor]));
  const out: SummaryDecisionRow[] = [];
  for (const row of rows) {
    if (row.origin === 'gate') continue;
    let state: SummaryDecisionState;
    let label = row.label;
    if (row.origin === 'monitor') {
      const monitor = monitors.get(row.originId ?? '');
      // Only monitors that exist now; a split copy never adds one.
      if (!monitor) continue;
      label = monitor.name;
      if (row.state === 'off') state = 'off';
      else if (row.state === 'error') state = 'error';
      else if (row.state === 'fired') state = 'fired';
      else state = row.count > 0 ? 'quiet' : 'pending';
    } else {
      state = row.state === 'error' ? 'error' : 'done';
    }
    out.push({
      key: row.key,
      origin: row.origin,
      originId: row.originId,
      label,
      count: row.count,
      errors: row.errors,
      state,
    });
  }
  return out;
}

/** What the section's one closed line says, and whether it opens on its own. */
export interface DecisionsHeadline {
  /** The visible count text: `1 fired`, `1 error`, `None fired`, `10 checks`. */
  text: string;
  /** What the live region reads. */
  spoken: string;
  /** A monitor fired or a row is in error: the section opens until the person says otherwise. */
  openByDefault: boolean;
}

const plural = (n: number, one: string, many: string) =>
  `${n.toLocaleString()} ${n === 1 ? one : many}`;

/**
 * Section 2.2's closed line, chosen by the first that applies: some monitor
 * fired; monitors exist and a row is in error; monitors exist and none fired;
 * no monitors (the count of checks). A fire and an error name both.
 */
export function decisionsHeadline(rows: readonly SummaryDecisionRow[]): DecisionsHeadline {
  const monitors = rows.filter((row) => row.origin === 'monitor');
  const fired = monitors.filter((row) => row.state === 'fired').length;
  const errors = rows.filter((row) => row.state === 'error').length;
  const checks = rows.reduce((sum, row) => sum + row.count, 0);
  const openByDefault = fired > 0 || errors > 0;
  if (fired > 0) {
    const firedText = `${fired.toLocaleString()} fired`;
    const firedSpoken = plural(fired, 'monitor fired', 'monitors fired');
    if (errors > 0) {
      const errorText = plural(errors, 'error', 'errors');
      return {
        text: `${firedText} · ${errorText}`,
        spoken: `${firedSpoken}, ${errorText}`,
        openByDefault,
      };
    }
    return { text: firedText, spoken: firedSpoken, openByDefault };
  }
  if (monitors.length > 0) {
    if (errors > 0) {
      const text = plural(errors, 'error', 'errors');
      return { text, spoken: text, openByDefault };
    }
    return { text: 'None fired', spoken: 'No monitor fired', openByDefault };
  }
  // No monitors: the count of checks, even with an error, which the open
  // section's row then shows by its warning shape and its word.
  return {
    text: plural(checks, 'check', 'checks'),
    spoken: plural(checks, 'decision check', 'decision checks'),
    openByDefault,
  };
}
