import {
  AlertCircle,
  AlertTriangle,
  Check,
  CircleIcon,
  Eye,
  EyeOff,
  Flag,
  SlidersHorizontal,
  X,
} from '../icons/app-icons';
import { ENTITY_ICONS, type EntityIcon } from '../icons/entity-icons';
import { ProbabilityBar } from '../decisions/ProbabilityBar';
import type { SessionDecisions } from '../../hooks/useSessionDecisions';
import {
  decisionSourceRows,
  formatProbability,
  type DecisionGateSource,
  type DecisionHistoryFilter,
  type DecisionSourceRow,
  type DecisionSourceState,
} from '../../utils/sessionDecisions';
import { summaryCopy } from './copy';
import './summaryDecisions.css';

const copy = summaryCopy.decisions;

type Glyph = EntityIcon;

/**
 * A source's state, told apart by SHAPE (never by colour alone): an eye that
 * watches, a raised flag once it fired, a struck eye when off, a warning for an
 * error; a gate's check, cross or alert; the decision mark for calls that just
 * decided. Keyed by state, so a new one fails the build here.
 */
export const DECISION_STATE_ICONS: Record<DecisionSourceState, Glyph> = {
  pending: CircleIcon,
  armed: Eye,
  fired: Flag,
  off: EyeOff,
  error: AlertTriangle,
  done: ENTITY_ICONS.decision,
  passed: Check,
  failed: X,
  blocked: AlertCircle,
};

export interface SummaryDecisionsData {
  sessionId: string;
  data: SessionDecisions | null;
  loading: boolean;
  error: boolean;
  refresh: () => void;
  /** Open the decision history view, filtered to one source. */
  onOpenHistory: (filter: DecisionHistoryFilter) => void;
  /**
   * Open the Monitors dialog. The dialog is the chat's (BaseChat mounts it), not
   * this section's: in the header popover, this section unmounts the moment the
   * popover closes, and a dialog inside it would go with it.
   */
  onOpenMonitors: () => void;
  /** The plan's phase gates, so a gate row says passed, failed or blocked. */
  gates?: readonly DecisionGateSource[];
  /** The monitors the newest turn marker says fired. */
  fired?: ReadonlySet<string>;
}

/** The second line of a row: what it last said, and how sure. */
function rowMeta(row: DecisionSourceRow): string {
  if (row.origin === 'gate' && row.attempts) {
    const t = row.threshold !== null ? formatProbability(row.threshold) : null;
    const p = row.lastPassProbability !== null ? formatProbability(row.lastPassProbability) : null;
    if (row.state === 'blocked') {
      // A gate that ran out of attempts has used them all; one blocked by
      // evaluation errors in a row has not, since an error uses no attempt.
      return row.attempts.attempts >= row.attempts.maxAttempts
        ? copy.gateBlocked(row.attempts.maxAttempts)
        : copy.gateBlockedUnevaluated;
    }
    if (row.state === 'passed' && p && t) return `${p} ≥ ${t}`;
    if (row.state === 'failed' && p && t) {
      return `${p} < ${t} · ${copy.gateAttempt(Math.max(1, row.attempts.attempts), row.attempts.maxAttempts)}`;
    }
  }
  if (row.count === 0) return copy.notChecked;
  const parts = [
    row.lastAnswer ? copy.last(row.lastAnswer) : null,
    row.lastPassProbability !== null ? `P ${formatProbability(row.lastPassProbability)}` : null,
    row.errors > 0 ? copy.errors(row.errors) : null,
  ];
  return parts.filter(Boolean).join(' · ');
}

function rowLabel(row: DecisionSourceRow): string {
  return [`${copy.state[row.state]}, ${row.label}`, copy.calls(row.count), rowMeta(row)]
    .filter(Boolean)
    .join(', ');
}

function SourceRow({
  row,
  onOpen,
}: {
  row: DecisionSourceRow;
  onOpen: (filter: DecisionHistoryFilter) => void;
}) {
  const Icon = DECISION_STATE_ICONS[row.state];
  return (
    <li className="br-decisions__item">
      <button
        type="button"
        className="br-decisions__row"
        data-state={row.state}
        data-origin={row.origin}
        data-testid={`decision-source-${row.key}`}
        aria-label={rowLabel(row)}
        aria-description={copy.openHint}
        onClick={() => onOpen({ origin: row.origin, originId: row.originId ?? null })}
      >
        <span className="br-summary__glyph br-decisions__glyph" aria-hidden="true">
          <Icon />
        </span>
        <span className="br-decisions__text" aria-hidden="true">
          <span className="br-decisions__line">
            <span className="br-decisions__name">{row.label}</span>
            <span className="br-decisions__count">{row.count.toLocaleString()}</span>
          </span>
          <span className="br-decisions__line br-decisions__meta">
            <span className="br-decisions__last">{rowMeta(row)}</span>
            {row.lastPassProbability !== null ? (
              <ProbabilityBar
                p={row.lastPassProbability}
                threshold={row.threshold}
                className="br-decisions__bar"
              />
            ) : null}
          </span>
        </span>
      </button>
    </li>
  );
}

/**
 * The chat's decision activity inside the Chat summary (the rail and its
 * popover alike), AGGREGATED per source (owner rule: decision calls can number
 * in the thousands, and no surface but the decision history view lists them
 * one by one). One row per monitor, one for every `decide` batch together, one
 * per phase gate, one for the decision chat: its label, its call count, what it
 * last answered with a thin bar of the last pass probability (the threshold
 * marked), and its state by shape. A row opens the decision history filtered to
 * that source.
 *
 * Under the rows, "Monitors" opens the Monitors dialog. It is a plain button in
 * this module, not the Button primitive, so the summary card keeps its three.
 * Nothing at all is drawn while the chat has made no decision calls and has no
 * monitors.
 */
export function SummaryDecisions({
  data,
  error,
  refresh,
  onOpenHistory,
  onOpenMonitors,
  gates,
  fired,
}: SummaryDecisionsData) {
  const summary = data?.summary ?? null;
  const settings = data?.settings ?? null;
  const total = summary?.total ?? 0;
  const monitors = settings?.monitors ?? [];
  const visible = total > 0 || monitors.length > 0;
  if (!visible) return null;

  const rows = decisionSourceRows(summary, settings, { gates, fired });
  const enabledMonitors = monitors.filter((monitor) => monitor.enabled).length;
  return (
    <section
      aria-label={copy.title}
      className="br-summary__section br-decisions"
      data-testid="summary-decisions"
    >
      <div className="br-summary__heading">
        <h3 className="br-summary__label">{copy.title}</h3>
        <span className="br-summary__count" aria-hidden="true">
          {total.toLocaleString()}
        </span>
        <span className="sr-only" aria-live="polite">
          {total.toLocaleString()}
          {copy.countSuffix}
        </span>
      </div>
      <ul className="br-decisions__list" aria-label={copy.listLabel}>
        {rows.map((row) => (
          <SourceRow key={row.key} row={row} onOpen={onOpenHistory} />
        ))}
      </ul>
      <button
        type="button"
        className="br-decisions__monitors"
        data-testid="summary-decisions-monitors"
        onClick={onOpenMonitors}
      >
        <span className="br-summary__glyph" aria-hidden="true">
          <SlidersHorizontal />
        </span>
        <span className="br-decisions__name">{copy.monitors}</span>
        <span className="br-decisions__count">
          {monitors.length === 0 ? copy.monitorsAdd : copy.monitorsOn(enabledMonitors)}
        </span>
      </button>
      {error ? (
        <div role="alert" className="br-summary__error br-decisions__error">
          <span>{copy.refreshFailed}</span>
          <button type="button" className="br-decisions__retry" onClick={refresh}>
            {summaryCopy.retry}
          </button>
        </div>
      ) : null}
    </section>
  );
}

export default SummaryDecisions;
