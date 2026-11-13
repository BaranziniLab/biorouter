import { useId } from 'react';
import { SlidersHorizontal } from '../icons/app-icons';
import { Button } from '../ui/button';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import type { SessionDecisions } from '../../hooks/useSessionDecisions';
import type { DecisionGateSource, DecisionHistoryFilter } from '../../utils/sessionDecisions';
import type { PhaseStatus } from '../../utils/sessionSubagents';
import { summaryCopy } from './copy';
import { SummarySectionHeading, useSectionToggle } from './SummarySectionHeading';
import { AgentMark } from './AgentMark';
import { AGENT_MARK_PX, DECISION_STATE_MARKS } from './agentGrid';
import {
  decisionsHeadline,
  summaryDecisionRows,
  type SummaryDecisionRow,
} from './summaryDecisionRows';
import './summaryDecisions.css';

const copy = summaryCopy.decisions;

/**
 * A phase gate as a caller reads it: the plan's gate, and, when the caller knows
 * it, the phase's status, which tells a pass or skip that closed the phase from
 * one that stands until its subagents finish.
 */
export interface SummaryGateSource extends DecisionGateSource {
  status?: PhaseStatus;
}

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
  /**
   * The plan's phase gates. The section draws no gate rows (a gate's one place
   * is its chip on its phase row, F-1); kept so callers need not change.
   */
  gates?: readonly SummaryGateSource[];
  /** The monitors the newest turn marker says fired. */
  fired?: ReadonlySet<string>;
  /** A decision chat (B-3): no Monitors button. */
  decisionChat?: boolean;
}

/** The trailing word: `Fired`, `Quiet`, `Off`, `Error`, `Not checked yet`, or the count. */
function trailingWord(row: SummaryDecisionRow): string {
  if (row.origin === 'monitor') return copy.state[row.state];
  if (row.state === 'error') return copy.state.error;
  return row.count.toLocaleString();
}

/**
 * Monitor rows: "Fired, Latch stop, checked 4 times"; `decide` and chat rows:
 * "Agent checks, 10 checks"; either adds ", 1 error".
 */
function rowLabel(row: SummaryDecisionRow): string {
  const parts =
    row.origin === 'monitor'
      ? [copy.state[row.state], row.label, copy.checkedTimes(row.count)]
      : [row.label, copy.checks(row.count)];
  if (row.errors > 0) parts.push(copy.errors(row.errors));
  return parts.join(', ');
}

function rowTitle(row: SummaryDecisionRow): string {
  if (row.origin === 'tool') return copy.agentChecks;
  if (row.origin === 'chat') return copy.chatAnswers;
  return row.label;
}

function SourceRow({
  row,
  onOpen,
}: {
  row: SummaryDecisionRow;
  onOpen: (filter: DecisionHistoryFilter) => void;
}) {
  const named = { ...row, label: rowTitle(row) };
  return (
    <li className="br-decisions__item">
      <button
        type="button"
        className="br-decisions__row"
        data-state={row.state}
        data-origin={row.origin}
        data-testid={`decision-source-${row.key}`}
        aria-label={rowLabel(named)}
        aria-description={copy.openHint}
        onClick={() => onOpen({ origin: row.origin, originId: row.originId ?? null })}
      >
        {/* Decision-model work, so a circle of the agent grid's notation
            (`agentGrid.ts`), filled by state, never by colour alone: half full
            once a monitor fired, solid for one that checked and stayed quiet
            (never the failed cross: a quiet monitor did nothing wrong, R1) and
            for `decide` batches and a decision chat's answers, hollow for one
            not checked yet, a bar when off, the outline with a small cross
            for an error. */}
        <span className="br-summary__glyph br-decisions__glyph" aria-hidden="true">
          <AgentMark
            kind={DECISION_STATE_MARKS[row.state]}
            shape="circle"
            size={AGENT_MARK_PX.grid}
          />
        </span>
        <span className="br-decisions__name" aria-hidden="true">
          {named.label}
        </span>
        <span className="br-decisions__count" aria-hidden="true">
          {trailingWord(row)}
        </span>
      </button>
    </li>
  );
}

/**
 * The chat's decision activity inside the Chat summary (the rail and its popover
 * alike), AGGREGATED per source (owner rule: decision calls can number in the
 * thousands, and no surface but the decision history lists them one by one).
 *
 * One line by default (F-1): "1 fired", "None fired" or "10 checks". It opens on
 * its own only when a monitor fired or a check errored, and once the person
 * toggles it their choice holds. Open, it shows one 28px row per monitor that
 * exists now (A-1), then the `decide` batches together and the decision chat:
 * a circle filled by state (decision-model work, in the agent grid's notation),
 * the name, and one trailing word. No probabilities and no bars
 * (R2: none can then sit beside the wrong answer; the history has them). A row
 * opens the decision history filtered to that source.
 *
 * Monitors is a quiet icon button on the heading line (F-16), not in a decision
 * chat. Nothing at all is drawn while the chat has made no decision calls but a
 * gate's and has no monitors.
 */
export function SummaryDecisions({
  data,
  error,
  refresh,
  onOpenHistory,
  onOpenMonitors,
  fired,
  decisionChat = false,
}: SummaryDecisionsData) {
  const summary = data?.summary ?? null;
  const settings = data?.settings ?? null;
  const monitors = settings?.monitors ?? [];
  const rows = summaryDecisionRows(summary, settings, { fired });
  // Drawn when it has a row to show or a monitor to speak of. Not on the
  // summary's total alone: that counts a phase gate's checks, whose one place is
  // their phase's chip, so a chat whose only decisions were a gate's read
  // "Decisions  0 checks" beside "Gate checked once, passed" (R2-6).
  const visible = rows.length > 0 || monitors.length > 0;
  const headline = decisionsHeadline(rows);
  const [open, setOpen] = useSectionToggle(headline.openByDefault);
  const listId = useId();
  if (!visible) return null;

  const monitorsButton = decisionChat ? null : (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          shape="round"
          size="xs"
          className="br-decisions__monitors"
          aria-label={copy.monitors}
          aria-haspopup="dialog"
          data-testid="summary-decisions-monitors"
          onClick={onOpenMonitors}
        >
          <SlidersHorizontal aria-hidden="true" />
        </Button>
      </TooltipTrigger>
      <TooltipContent side="top">{copy.monitors}</TooltipContent>
    </Tooltip>
  );

  return (
    <section
      aria-label={copy.title}
      className="br-summary__section br-decisions"
      data-testid="summary-decisions"
    >
      <SummarySectionHeading
        label={copy.title}
        count={headline.text}
        open={open}
        onOpenChange={setOpen}
        controls={listId}
        trailing={monitorsButton}
        testId="summary-decisions-toggle"
      >
        <span className="sr-only" aria-live="polite">
          {headline.spoken}
        </span>
      </SummarySectionHeading>
      {open && rows.length > 0 ? (
        <ul id={listId} className="br-decisions__list" aria-label={copy.listLabel}>
          {rows.map((row) => (
            <SourceRow key={row.key} row={row} onOpen={onOpenHistory} />
          ))}
        </ul>
      ) : null}
      {error ? (
        <div role="alert" className="br-summary__error br-decisions__error">
          <span>{copy.refreshFailed}</span>
          {/* A plain button at the error line's 12, as the roster's Retry is. */}
          <button type="button" className="br-decisions__retry" onClick={refresh}>
            {summaryCopy.retry}
          </button>
        </div>
      ) : null}
    </section>
  );
}

export default SummaryDecisions;
