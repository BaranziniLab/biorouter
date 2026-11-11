import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type FocusEvent,
  type KeyboardEvent,
  type ReactNode,
} from 'react';
import { plainExcerpt } from './plainExcerpt';
import { ChevronRight, Lock, Plus } from '../icons/app-icons';
import { usePrivacyTiersEnabled } from '../ConfigContext';
import { PopoverContent } from '../ui/popover';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { useRovingRows } from '../ui/useRovingRows';
import {
  FOOTER_CARD_COLLISION_PADDING,
  FooterCardGroup,
  FooterCardRoot,
} from '../bottom_menu/FooterCard';
import { footerCardTriggerProps, useFooterCard } from '../bottom_menu/useFooterCard';
import { thresholdLine } from '../decisions/decisionWords';
import { SUBAGENT_STATUS_WORDS } from '../subagent/subagentStatus';
import {
  completedPhaseCount,
  formatElapsed,
  heldByTitle,
  inSpawnOrder,
  isClosedPhase,
  isQuiet,
  phaseOpenByDefault,
  subagentCounts,
  subagentTitle,
  type PhaseSnapshot,
  type SessionSubagents,
  type SubagentRow,
} from '../../utils/sessionSubagents';
import { phaseGateState, type PhaseGateSnapshot } from '../../utils/sessionDecisions';
import { summaryCopy } from './copy';
import { SubagentPeek, peekModelName } from './SubagentPeek';
import { AgentMark } from './AgentMark';
import {
  AGENT_MARK_PX,
  agentGridSize,
  agentGroups,
  agentMarkKind,
  agentMarkPulses,
  agentMarkShape,
  agentRowMarkPx,
  gateMarkKind,
  groupDrawsGrid,
  splitHead,
  type AgentGridSize,
  type AgentGroup,
} from './agentGrid';
import { SummarySectionHeading, useSectionToggle } from './SummarySectionHeading';
import { forgetRoster, rememberRoster, rememberedRoster } from './rosterMemory';
import './summaryAgents.css';
import './summaryAgentGrid.css';
import './summaryDecisions.css';

const copy = summaryCopy.agents;

/**
 * The words a phase's gate mark and line say. Kept here, beside the mark that
 * says them, rather than in `summary/copy.ts`.
 */
export const PHASE_GATE_ROW_COPY = {
  waitsFor: (title: string) => `Waits for ${title}'s gate`,
  addGate: 'Add gate',
  addGateLabel: (phase: string) => `Add gate, ${phase}`,
};

/** A gate mark's own words (spec 3.3): the mark's one word, then the sentence its tooltip says. */
const GATE_CHIP_COPY = {
  chip: {
    pending: 'Gate',
    passed: 'Passed',
    failed: 'Not passed',
    needsYou: 'Needs you',
    blocked: 'Blocked',
    skipped: 'Skipped',
  },
  notAsked: 'Gate not asked yet',
  passed: 'Gate passed',
  notPassed: 'Gate not passed',
  needsYou: 'Gate needs you',
  blocked: 'Gate blocked',
  passedByYou: 'Passed by you',
  skippedByYou: 'Skipped by you',
  passStands: 'Your pass stands until its subagents finish',
  skipStands: 'Your skip stands until its subagents finish',
  attempt: (attempt: number, max: number) => `attempt ${attempt} of ${max}`,
  blockedAfter: (max: number) => `blocked after ${max} attempts`,
  unevaluated: 'could not be evaluated',
};

export interface SummarySubagentsData {
  /**
   * The chat these belong to. With it, the section keeps showing the last roster
   * it drew for this chat while a re-read is out or failed (H-12: a tab switch
   * that remounted the summary showed no Plan section until the next visit).
   */
  sessionId?: string;
  data: SessionSubagents | null;
  loading: boolean;
  error: boolean;
  refresh: () => void;
  /** Open the subagent as a tab and make it the active one. */
  onOpen: (sessionId: string, title?: string) => void;
  /**
   * Open a phase's gate dialog (`decisions/PhaseGateDialog.tsx`), which holds the
   * person's controls and the gate's decision history; for an open phase with no
   * gate, the dialog opens on its editor ("Add gate"). Without it a phase's gate
   * mark is drawn, but is not a control, and no phase offers "Add gate".
   */
  onOpenGate?: (phaseId: string) => void;
  /**
   * The chat's monitors that watch its subagents too (`subagentWatchers`): every
   * subagent's card says it is watched by them. The Chat summary passes them from
   * the chat's decision settings.
   */
  watchedBy?: readonly string[];
}

// ---------------------------------------------------------------------------
// Words
// ---------------------------------------------------------------------------

/**
 * What a child is doing, or found (spec 4.1): `Waiting to start`, `Working…`,
 * `Quiet for 4m`, `Stopping…`, or once it ended its result line, else its status
 * word.
 */
export function subagentLine(row: SubagentRow): string {
  switch (row.status) {
    case 'queued':
      return SUBAGENT_STATUS_WORDS.queued;
    case 'running':
      return isQuiet(row) ? copy.quietFor(formatElapsed(row.quietSeconds) ?? '') : copy.working;
    case 'cancelling':
      return copy.stopping;
    case 'unknown':
      return SUBAGENT_STATUS_WORDS.unknown;
    default:
      // R2-5: a plain one-line excerpt; a result that begins with a markdown
      // table read "| Patient | sodium (Na) | …".
      return plainExcerpt(row.resultLine) || SUBAGENT_STATUS_WORDS[row.status];
  }
}

/** A row's trailing figure: the run time, or `quiet 4m`; nothing without either. */
function rowTrailing(row: SubagentRow): string | null {
  if (isQuiet(row)) return copy.quietShort(formatElapsed(row.quietSeconds) ?? '');
  return formatElapsed(row.elapsedSeconds);
}

/**
 * An agent's accessible name, the same on its square and its row: the status
 * word, the role or name, the model, the run time, and whether it is decision
 * work or private. "Running, Reader 2, sonnet-4-5, 4m". Named explicitly: the
 * visible spans are laid out as lines, and a name computed from them runs the
 * words together.
 */
export function agentLabel(row: SubagentRow, isPrivate: boolean): string {
  return [
    isQuiet(row)
      ? copy.quietFor(formatElapsed(row.quietSeconds) ?? '')
      : SUBAGENT_STATUS_WORDS[row.status],
    subagentTitle(row),
    peekModelName(row),
    formatElapsed(row.elapsedSeconds),
    agentMarkShape(row) === 'circle' ? copy.decisionModel.toLowerCase() : null,
    isPrivate ? copy.private : null,
  ]
    .filter(Boolean)
    .join(', ');
}

/**
 * A gate's words (spec 3.3): the mark's one word and the full sentence its
 * tooltip says, with the probability named by its outcome (R2: `P(yes) 0.62 ·
 * threshold 0.80`). A gate's questions are yes/no, so the watched outcome is yes.
 * Without the phase, its status is not known, and a person's decision reads as
 * one that stands. `needsYou` and `attention` say when the word is drawn beside
 * the mark: only when the gate wants a look.
 */
export function gateChipWords(
  gate: PhaseGateSnapshot,
  phase?: Pick<PhaseSnapshot, 'status'>
): { chip: string; sentence: string; needsYou: boolean; attention: boolean } {
  const c = GATE_CHIP_COPY;
  const closed = phase ? isClosedPhase(phase) : false;
  const numbers = thresholdLine({ type: 'yes', p: gate.threshold }, null, gate.lastMinP);
  if (gate.personDecision === 'pass') {
    return {
      chip: c.chip.passed,
      sentence: closed ? c.passedByYou : c.passStands,
      needsYou: false,
      attention: false,
    };
  }
  if (gate.personDecision === 'skip') {
    return {
      chip: c.chip.skipped,
      sentence: closed ? c.skippedByYou : c.skipStands,
      needsYou: false,
      attention: false,
    };
  }
  const blockedReason =
    gate.attempts >= gate.maxAttempts ? c.blockedAfter(gate.maxAttempts) : c.unevaluated;
  if (gate.needsPerson) {
    return {
      chip: c.chip.needsYou,
      sentence: gate.blocked ? `${c.needsYou}: ${blockedReason}` : c.needsYou,
      needsYou: true,
      attention: true,
    };
  }
  if (gate.blocked) {
    return {
      chip: c.chip.blocked,
      sentence: `${c.blocked}: ${blockedReason}`,
      needsYou: false,
      attention: true,
    };
  }
  if (gate.lastPassed === true) {
    return {
      chip: c.chip.passed,
      sentence: `${c.passed} · ${numbers}`,
      needsYou: false,
      attention: false,
    };
  }
  if (gate.lastPassed === false) {
    const attempt = c.attempt(Math.max(1, gate.attempts), gate.maxAttempts);
    return {
      chip: c.chip.failed,
      sentence: `${c.notPassed} · ${numbers} · ${attempt}`,
      needsYou: false,
      attention: true,
    };
  }
  return {
    chip: c.chip.pending,
    sentence: `${c.notAsked} · ${numbers}`,
    needsYou: false,
    attention: false,
  };
}

// ---------------------------------------------------------------------------
// The view: what is drawn, and the row keys in document order
// ---------------------------------------------------------------------------

interface GroupView {
  group: AgentGroup;
  /** The toggle's row key; `all` (no plan) has no toggle of its own. */
  key: string;
  /** The title: the phase's, "Other agents", or none for `all`. */
  title: string | null;
  open: boolean;
  /** Draws its grid of squares above its rows. */
  grid: boolean;
  /** The rows drawn: every one, or the head and, once shown, the rest. */
  shown: SubagentRow[];
  /** The rows "Show N more" would add. */
  hidden: number;
  /** The "Show N more" / "Show fewer" row's key, when the group has a rest. */
  moreKey: string | null;
  all: boolean;
  /**
   * Folded, it still draws its grid: some agent in it is not finished (running,
   * waiting, failed, not started), so the grid is the group's live summary, as
   * the reference's "Check 2/3" is. A folded group whose every agent finished is
   * its line alone.
   */
  gridWhileFolded: boolean;
}

interface View {
  size: AgentGridSize;
  plan: SessionSubagents['plan'];
  groups: GroupView[];
  total: number;
  keys: string[];
  preferredKey: string | null;
}

const agentKey = (row: SubagentRow) => `agent:${row.sessionId}`;
const gridKey = (group: AgentGroup) => `grid:${group.key}`;
const gateKey = (phase: PhaseSnapshot) => `gate:${phase.id}`;
const addGateKey = (phase: PhaseSnapshot) => `addgate:${phase.id}`;

/** An open phase with no gate offers "Add gate" (to the person, when the mark is a control). */
const offersAddGate = (phase: PhaseSnapshot) => !phase.gate && !isClosedPhase(phase);

/** The row element carrying `data-row={key}` inside `container`, if one is drawn. */
function rowElement(container: HTMLElement | null, key: string): HTMLElement | null {
  if (!container) return null;
  for (const element of container.querySelectorAll<HTMLElement>('[data-row]')) {
    if (element.getAttribute('data-row') === key) return element;
  }
  return null;
}

/** The row that holds focus: its key, and the element it was focused on. */
interface FocusedRow {
  key: string;
  node: HTMLElement;
}

function buildView(
  data: SessionSubagents | null,
  openState: Record<string, boolean>,
  showAll: Record<string, boolean>,
  gatesOpen: boolean,
  keep: ReadonlySet<string>
): View {
  const keys: string[] = [];
  const rows = inSpawnOrder(data?.subagents ?? []);
  const size = agentGridSize(rows.length);
  const plan = data?.plan ?? null;
  if (!data || (rows.length === 0 && !plan)) {
    return { size, plan, groups: [], total: 0, keys, preferredKey: null };
  }
  const groups = agentGroups(plan, rows).map((group): GroupView => {
    const phase = group.phase;
    const title = phase ? phase.title : group.key === 'other' ? copy.otherAgents : null;
    const key = group.key;
    const open =
      group.key === 'all'
        ? true
        : (openState[key] ?? (phase ? phaseOpenByDefault(phase, plan!.phases.length) : true));
    const grid = groupDrawsGrid(group.rows.length, size);
    const all = showAll[key] === true;
    let shown = group.rows;
    let hidden = 0;
    if (grid) {
      const split = splitHead(group.rows, keep);
      shown = all ? [...split.head, ...split.rest] : split.head;
      hidden = split.rest.length;
    }
    const moreKey = grid && hidden > 0 ? `more:${key}` : null;
    const gridWhileFolded =
      !open && grid && group.rows.some((row) => agentMarkKind(row) !== 'done');
    const view: GroupView = {
      group,
      key,
      title,
      open,
      grid,
      shown,
      hidden,
      moreKey,
      all,
      gridWhileFolded,
    };
    if (group.key !== 'all' && group.rows.length > 0) keys.push(key);
    // The gate mark (or "Add gate") sits at the end of the phase's line.
    if (phase && gatesOpen && phase.gate) keys.push(gateKey(phase));
    if (phase && gatesOpen && offersAddGate(phase)) keys.push(addGateKey(phase));
    if (gridWhileFolded) keys.push(gridKey(group));
    if (open && group.rows.length > 0) {
      if (grid) keys.push(gridKey(group));
      for (const row of shown) keys.push(agentKey(row));
      if (moreKey) keys.push(moreKey);
    }
    return view;
  });
  const firstRunning = rows.find((row) => agentMarkKind(row) === 'running');
  const runningKey = firstRunning ? agentKey(firstRunning) : null;
  return {
    size,
    plan,
    groups,
    total: rows.length,
    keys,
    preferredKey: runningKey && keys.includes(runningKey) ? runningKey : (keys[0] ?? null),
  };
}

// ---------------------------------------------------------------------------
// The card every square and row opens: hover peeks, click pins
// ---------------------------------------------------------------------------

interface RowControls {
  tabIndexFor: (key: string) => 0 | -1;
  onRowFocus: (key: string) => void;
  onOpen: SummarySubagentsData['onOpen'];
  onOpenGate?: SummarySubagentsData['onOpenGate'];
  /** The plan, for the title of the phase whose gate holds another. */
  plan: SessionSubagents['plan'];
  /** A row's or square's card was kept (`true`) or let go (`false`). */
  onKeptChange: (sessionId: string, kept: boolean) => void;
  watchedBy: readonly string[];
}

/**
 * One agent's card on the composer footer's own machinery (`useFooterCard`, R8,
 * D-43): a hover previews it after 300ms and the trigger stays at rest; a click,
 * Enter or Space keeps it and marks the trigger `data-pinned`; Escape, a press
 * outside or a second click closes it. The card holds "Open in a tab". A square
 * and a row are two triggers of the same card content, and one card is open at a
 * time across the roster (`FooterCardGroup`).
 */
function AgentCard({
  row,
  controls,
  trigger,
}: {
  row: SubagentRow;
  controls: RowControls;
  trigger: (props: ReturnType<typeof footerCardTriggerProps>, pinned: boolean) => ReactNode;
}) {
  const card = useFooterCard();
  const cardId = useId();
  const title = subagentTitle(row);

  // A kept card's row stays drawn while it is kept, whatever a poll does to its rank.
  const { onKeptChange } = controls;
  const keptRef = useRef(false);
  useEffect(() => {
    if (card.pinned !== keptRef.current) {
      keptRef.current = card.pinned;
      onKeptChange(row.sessionId, card.pinned);
    }
  }, [card.pinned, onKeptChange, row.sessionId]);
  useEffect(
    () => () => {
      if (keptRef.current) onKeptChange(row.sessionId, false);
    },
    [onKeptChange, row.sessionId]
  );

  return (
    <FooterCardRoot card={card}>
      {trigger(footerCardTriggerProps(card, cardId), card.pinned)}
      {card.open ? (
        <PopoverContent
          id={cardId}
          ref={card.contentRef}
          side="left"
          align="start"
          sideOffset={12}
          collisionPadding={FOOTER_CARD_COLLISION_PADDING}
          aria-label={copy.peek.label(title)}
          data-testid={`subagent-card-${row.sessionId}`}
          data-pinned={card.pinned ? 'true' : undefined}
          className="br-footer-card br-agents-peek"
          {...card.contentHandlers}
        >
          <SubagentPeek
            row={row}
            watchedBy={controls.watchedBy}
            onOpen={() => {
              controls.onOpen(row.sessionId, title);
              card.close();
            }}
          />
        </PopoverContent>
      ) : null}
    </FooterCardRoot>
  );
}

// ---------------------------------------------------------------------------
// Rows and squares
// ---------------------------------------------------------------------------

/**
 * One agent as a row: its mark, its role or name, and its run time on the right.
 * In a chat of four agents or fewer (`rows`), a second muted line says the model
 * and what it is doing or found; otherwise it is one 28px line under the grid.
 * The lock of a private child sits at its end in muted ink (F-16).
 */
function AgentRow({
  row,
  size,
  controls,
}: {
  row: SubagentRow;
  size: AgentGridSize;
  controls: RowControls;
}) {
  const key = agentKey(row);
  const tiersEnabled = usePrivacyTiersEnabled();
  const title = subagentTitle(row);
  const quiet = isQuiet(row);
  const isPrivate = tiersEnabled && row.privacyTier === 'private';
  const layout = size === 'rows' ? 'comfortable' : 'dense';
  const trailing = rowTrailing(row);
  const model = peekModelName(row);
  const line =
    layout === 'comfortable' ? [model, subagentLine(row)].filter(Boolean).join(' · ') : null;
  const kind = agentMarkKind(row);
  return (
    <li className="br-agents__item" data-row-item="">
      <AgentCard
        row={row}
        controls={controls}
        trigger={(trigger) => (
          <button
            type="button"
            className="br-agents__row"
            data-row={key}
            data-layout={layout}
            data-status={row.status}
            data-mark={kind}
            data-quiet={quiet ? '' : undefined}
            data-testid={`subagent-row-${row.sessionId}`}
            aria-label={agentLabel(row, isPrivate)}
            tabIndex={controls.tabIndexFor(key)}
            {...trigger}
            onFocus={() => {
              controls.onRowFocus(key);
              trigger.onFocus();
            }}
          >
            <span className="br-summary__glyph br-agents__glyph" aria-hidden="true">
              <AgentMark
                kind={kind}
                shape={agentMarkShape(row)}
                size={agentRowMarkPx(size)}
                pulse={agentMarkPulses(row)}
              />
            </span>
            <span className="br-agents__text" aria-hidden="true">
              <span className="br-agents__name-line">
                <span className="br-agents__name">{title}</span>
                {trailing ? <span className="br-agents__trailing">{trailing}</span> : null}
                {isPrivate ? (
                  <span className="br-agents__lock">
                    <Lock />
                  </span>
                ) : null}
              </span>
              {line ? <span className="br-agents__meta">{line}</span> : null}
            </span>
          </button>
        )}
      />
    </li>
  );
}

/**
 * A group's grid: one square per agent, in spawn order, so a square never moves
 * while the group runs. A square's hover peeks its card and a click keeps it, as
 * a row's does. The grid is ONE stop of the roster's arrow-key order (its current
 * square carries the group's `data-row`): Up and Down leave it for the rows above
 * and below, Left and Right move between its squares.
 */
function AgentGrid({
  group,
  size,
  title,
  controls,
}: {
  group: AgentGroup;
  size: AgentGridSize;
  title: string | null;
  controls: RowControls;
}) {
  const tiersEnabled = usePrivacyTiersEnabled();
  const key = gridKey(group);
  const [current, setCurrent] = useState(0);
  const index = Math.min(current, group.rows.length - 1);
  const listRef = useRef<HTMLUListElement>(null);
  const onKeyDown = (event: KeyboardEvent<HTMLUListElement>) => {
    if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return;
    if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey) return;
    const rtl = listRef.current ? getComputedStyle(listRef.current).direction === 'rtl' : false;
    const forward = (event.key === 'ArrowRight') !== rtl;
    const next = Math.max(0, Math.min(group.rows.length - 1, index + (forward ? 1 : -1)));
    event.preventDefault();
    if (next === index) return;
    setCurrent(next);
    listRef.current?.querySelectorAll<HTMLElement>('[data-grid-cell]')[next]?.focus();
  };
  return (
    <div className="br-agent-grid" data-size={size} data-row-item="">
      <ul
        ref={listRef}
        className="br-agent-grid__cells"
        aria-label={title ? copy.gridLabel(title) : copy.gridOfChat}
        onKeyDown={onKeyDown}
      >
        {group.rows.map((row, i) => {
          const isPrivate = tiersEnabled && row.privacyTier === 'private';
          const kind = agentMarkKind(row);
          return (
            <li key={row.sessionId} className="br-agent-grid__item">
              <AgentCard
                row={row}
                controls={controls}
                trigger={(trigger) => (
                  <button
                    type="button"
                    className="br-agent-grid__cell"
                    data-grid-cell=""
                    data-row={i === index ? key : undefined}
                    data-mark={kind}
                    data-shape={agentMarkShape(row)}
                    data-testid={`subagent-cell-${row.sessionId}`}
                    aria-label={agentLabel(row, isPrivate)}
                    tabIndex={i === index ? controls.tabIndexFor(key) : -1}
                    {...trigger}
                    onFocus={() => {
                      setCurrent(i);
                      controls.onRowFocus(key);
                      trigger.onFocus();
                    }}
                  >
                    <AgentMark
                      kind={kind}
                      shape={agentMarkShape(row)}
                      size={AGENT_MARK_PX[size]}
                      pulse={agentMarkPulses(row)}
                    />
                  </button>
                )}
              />
            </li>
          );
        })}
      </ul>
    </div>
  );
}

function MoreRow({
  view,
  controls,
  onToggle,
}: {
  view: GroupView;
  controls: RowControls;
  onToggle: (key: string) => void;
}) {
  if (!view.moreKey) return null;
  const key = view.moreKey;
  return (
    <li className="br-agents__item" data-row-item="">
      <button
        type="button"
        className="br-agents__more"
        data-row={key}
        tabIndex={controls.tabIndexFor(key)}
        onFocus={() => controls.onRowFocus(key)}
        onClick={() => onToggle(view.key)}
      >
        {view.all ? copy.showFewer : copy.showMore(view.hidden)}
      </button>
    </li>
  );
}

/** A group's body: its grid, then its rows and "Show N more". */
function GroupBody({
  view,
  size,
  controls,
  onToggleAll,
  id,
}: {
  view: GroupView;
  size: AgentGridSize;
  controls: RowControls;
  onToggleAll: (key: string) => void;
  id?: string;
}) {
  return (
    <div id={id} className="br-agents__group-body">
      {view.grid ? (
        <AgentGrid group={view.group} size={size} title={view.title} controls={controls} />
      ) : null}
      <ul
        aria-label={view.title ? copy.rowsLabel(view.title) : copy.rowsOfChat}
        className="br-agents__list"
        data-layout={size === 'rows' ? 'comfortable' : 'dense'}
      >
        {view.shown.map((row) => (
          <AgentRow key={row.sessionId} row={row} size={size} controls={controls} />
        ))}
        <MoreRow view={view} controls={controls} onToggle={onToggleAll} />
      </ul>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Phases and their gates
// ---------------------------------------------------------------------------

/**
 * The muted line under a phase, only when an earlier gate holds it (spec 3.1):
 * "Waits for Build's gate". Its failed and blocked subagents are its grid's
 * crosses and half-filled marks, and words in its name.
 */
function phaseNoticeLine(heldBy: string | null): string {
  return heldBy ? PHASE_GATE_ROW_COPY.waitsFor(heldBy) : '';
}

/** "1 failed, 1 blocked", for a phase's accessible name; `null` when neither. */
function phaseTroubleWords(phase: PhaseSnapshot): string | null {
  const { counts } = phase;
  const words = [
    counts.failed > 0 ? copy.failed(counts.failed) : null,
    counts.blocked > 0 ? copy.blocked(counts.blocked) : null,
  ].filter(Boolean);
  return words.length > 0 ? words.join(', ') : null;
}

/**
 * A phase gate as a small circle at the end of its phase's line (owner rule: a
 * gate's evaluations are never transcript rows, and F-1: never a row of its own).
 * A gate is decision-model work, so its mark is a circle, filled by its state;
 * its word is drawn beside it only when the gate wants a look ("Needs you", "Not
 * passed", "Blocked"). The full sentence is its tooltip and its name. Pressing it
 * opens the gate's dialog.
 */
function GateChip({ phase, controls }: { phase: PhaseSnapshot; controls: RowControls }) {
  const gate = phase.gate;
  if (!gate) return null;
  // The gate dialog reads the same rule.
  const state = phaseGateState(gate);
  const words = gateChipWords(gate, phase);
  const name = `${words.sentence}, ${phase.title}`;
  const content = (
    <>
      <span className="br-agents__gate-glyph" aria-hidden="true">
        <AgentMark kind={gateMarkKind(gate)} shape="circle" size={AGENT_MARK_PX.grid} />
      </span>
      {words.attention ? (
        <span className="br-agents__gate-text" aria-hidden="true">
          {words.chip}
        </span>
      ) : null}
    </>
  );
  if (!controls.onOpenGate) {
    return (
      <span
        className="br-agents__gate"
        role="img"
        data-state={state}
        data-static=""
        data-attention={words.attention ? '' : undefined}
        data-needs-person={words.needsYou ? '' : undefined}
        data-testid={`phase-gate-${phase.id}`}
        aria-label={name}
      >
        {content}
      </span>
    );
  }
  const key = gateKey(phase);
  const open = controls.onOpenGate;
  return (
    <span data-row-item="" className="br-agents__gate-slot">
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            className="br-agents__gate"
            data-row={key}
            data-state={state}
            data-attention={words.attention ? '' : undefined}
            data-needs-person={words.needsYou ? '' : undefined}
            data-testid={`phase-gate-${phase.id}`}
            aria-label={name}
            tabIndex={controls.tabIndexFor(key)}
            onFocus={() => controls.onRowFocus(key)}
            onClick={() => open(phase.id)}
          >
            {content}
          </button>
        </TooltipTrigger>
        <TooltipContent side="top">{words.sentence}</TooltipContent>
      </Tooltip>
    </span>
  );
}

/**
 * "Add gate" on an open phase with no gate, drawn in the gate's slot as a 20px
 * plus (its words are its name and its tooltip, so it never takes the phase
 * title's room): quiet until the phase is hovered or the button focused, always
 * in the arrow-key order, so a keyboard finds it the way a pointer does. Pressing
 * it opens the gate dialog on its editor.
 */
function AddGateButton({
  phase,
  controls,
  shown,
}: {
  phase: PhaseSnapshot;
  controls: RowControls;
  shown: boolean;
}) {
  const [focused, setFocused] = useState(false);
  const open = controls.onOpenGate;
  if (!open || !offersAddGate(phase)) return null;
  const key = addGateKey(phase);
  const visible = shown || focused;
  return (
    <span data-row-item="" className="br-agents__gate-slot">
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            className="br-agents__gate"
            data-row={key}
            data-add-gate=""
            data-visible={visible ? '' : undefined}
            data-testid={`phase-add-gate-${phase.id}`}
            aria-label={PHASE_GATE_ROW_COPY.addGateLabel(phase.title)}
            tabIndex={controls.tabIndexFor(key)}
            onFocus={() => {
              setFocused(true);
              controls.onRowFocus(key);
            }}
            onBlur={() => setFocused(false)}
            onClick={() => open(phase.id)}
          >
            <span className="br-agents__gate-glyph" aria-hidden="true">
              <Plus />
            </span>
          </button>
        </TooltipTrigger>
        <TooltipContent side="top">{PHASE_GATE_ROW_COPY.addGate}</TooltipContent>
      </Tooltip>
    </span>
  );
}

/**
 * One group of the Plan section: a phase (or "Other agents"), as a collapsible
 * line with its title, "done of total" on the right and its gate's mark, then its
 * grid, its rows and "Show N more". A phase that used no subagents is one static
 * line that says where it stands ("Pending", "Skipped").
 */
function GroupItem({
  view,
  size,
  controls,
  onToggle,
  onToggleAll,
}: {
  view: GroupView;
  size: AgentGridSize;
  controls: RowControls;
  onToggle: (key: string, open: boolean) => void;
  onToggleAll: (key: string) => void;
}) {
  const { group, key, open } = view;
  const phase = group.phase;
  const bodyId = useId();
  const [hovered, setHovered] = useState(false);
  const rowsTotal = group.rows.length;
  const total = phase ? Math.max(phase.counts.total, rowsTotal) : rowsTotal;
  const done = phase ? Math.min(phase.counts.done, total) : subagentCounts(group.rows).done;
  // H-12: a phase that used no subagents (its work was the parent's own) shows no
  // "0 of 0"; its status word says where it stands.
  const count =
    total > 0 ? summaryCopy.todoCount(done, total) : phase ? copy.phaseStatus[phase.status] : null;
  const heldBy = phase ? heldByTitle(controls.plan, phase) : null;
  const notice = phaseNoticeLine(heldBy);
  const title = view.title ?? copy.agents;
  // "Read the trials, research phase, in progress, 1 of 3 subagents done": named
  // explicitly, for the reason a row is. The kind lives here only.
  const label = phase
    ? [
        phase.title,
        copy.phaseKindWords(copy.phaseKind[phase.kind]),
        copy.phaseStatus[phase.status].toLowerCase(),
        total > 0 ? `${summaryCopy.todoCount(done, total)}${copy.phaseAgentsSuffix}` : null,
        phaseTroubleWords(phase),
        heldBy ? PHASE_GATE_ROW_COPY.waitsFor(heldBy) : null,
      ]
        .filter(Boolean)
        .join(', ')
    : `${title}, ${summaryCopy.todoCount(done, total)}${copy.doneSuffix}`;
  const heading = (
    <>
      <span className="br-agents__chevron" data-open={open ? '' : undefined} aria-hidden="true">
        {rowsTotal > 0 ? <ChevronRight /> : null}
      </span>
      <span className="br-agents__phase-title" aria-hidden="true">
        {title}
      </span>
      {count ? (
        <span className="br-summary__count" aria-hidden="true">
          {count}
        </span>
      ) : null}
    </>
  );
  return (
    <li
      className="br-agents__phase"
      data-status={phase?.status}
      data-phase-id={phase?.id}
      data-group={group.key}
      data-held-by={heldBy ? (phase?.heldBy ?? undefined) : undefined}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      <div className="br-agents__phase-line">
        {rowsTotal > 0 ? (
          <span data-row-item="" className="br-agents__phase-slot">
            <button
              type="button"
              className="br-agents__phase-toggle"
              data-row={key}
              aria-label={label}
              aria-expanded={open}
              aria-controls={open ? bodyId : undefined}
              tabIndex={controls.tabIndexFor(key)}
              onFocus={() => controls.onRowFocus(key)}
              onClick={() => onToggle(key, !open)}
            >
              {heading}
            </button>
          </span>
        ) : (
          <div className="br-agents__phase-toggle" data-static="" role="group" aria-label={label}>
            {heading}
          </div>
        )}
        {phase ? <GateChip phase={phase} controls={controls} /> : null}
        {phase ? <AddGateButton phase={phase} controls={controls} shown={hovered} /> : null}
      </div>
      {notice ? <p className="br-agents__phase-counts">{notice}</p> : null}
      {open && rowsTotal > 0 ? (
        <GroupBody
          id={bodyId}
          view={view}
          size={size}
          controls={controls}
          onToggleAll={onToggleAll}
        />
      ) : view.gridWhileFolded ? (
        <AgentGrid group={group} size={size} title={view.title} controls={controls} />
      ) : null}
    </li>
  );
}

function ListCount({ rows }: { rows: readonly SubagentRow[] }) {
  const counts = subagentCounts(rows);
  return (
    <>
      {counts.running > 0 ? `${copy.running(counts.running)} · ` : ''}
      {summaryCopy.todoCount(counts.done, counts.total)}
    </>
  );
}

/**
 * The chat's subagents, inside the Chat summary (the docked rail and its popover
 * alike), drawn in the agent grid's notation (`agentGrid.ts`): one mark per
 * agent, a square for a language-model agent and a circle for decision-model
 * work, filled and inked by status.
 *
 * - With a phase plan, a Plan section: one collapsible group per phase in plan
 *   order (title, "done of total", the gate's circle), then "Other agents".
 * - Without one, an Agents section holding the whole roster as one group.
 * - In a chat of four agents or fewer every agent is a two-line row led by a 12px
 *   mark. With more, a group of more than four draws its grid of squares (10px,
 *   7px from 100 agents), then its first five rows (waiting for the person,
 *   failed, running, then the newest), then "Show N more".
 *
 * A square's or row's hover previews its card and a click keeps it (one open at a
 * time); the card holds "Open in a tab" and the setup. The roster is one tab stop;
 * the arrow keys, Home and End move between rows, and Left and Right between a
 * grid's squares, opening nothing.
 */
export function SummarySubagents({
  sessionId,
  data: given,
  loading,
  error,
  refresh,
  onOpen,
  onOpenGate,
  watchedBy = [],
}: SummarySubagentsData) {
  const [openState, setOpenState] = useState<Record<string, boolean>>({});
  const [showAll, setShowAll] = useState<Record<string, boolean>>({});
  const [kept, setKept] = useState<ReadonlySet<string>>(new Set());
  const [focusedAgent, setFocusedAgent] = useState<string | null>(null);
  const [sectionOpen, setSectionOpen] = useSectionToggle(true);
  const sectionBodyId = useId();

  // H-12: while a re-read is out or failed, keep drawing what this chat's roster
  // last was; a real answer replaces it, and an answer of "none" forgets it.
  const data =
    given ?? (sessionId && (loading || error) ? rememberedRoster(sessionId) : null) ?? null;
  useEffect(() => {
    if (!sessionId) return;
    if (given) rememberRoster(sessionId, given);
    else if (!loading && !error) forgetRoster(sessionId);
  }, [sessionId, given, loading, error]);

  // A row whose card is kept, or that holds focus, stays drawn whatever its rank.
  const keep = focusedAgent ? new Set([...kept, focusedAgent]) : kept;
  const view = buildView(data, openState, showAll, !!onOpenGate, keep);
  const keys = sectionOpen ? view.keys : [];
  const roving = useRovingRows(keys, view.preferredKey);

  // A row can be remounted under the person's focus: a poll that moves a child
  // into or out of a group's grid, or a fifth spawn that turns the rows into a
  // grid, draws the row under a different parent, and React keys match only
  // siblings, so its old button is removed and focus falls to the body. After
  // every render that could redraw rows (new data, a disclosure), put focus back
  // on the same row's new button. Only for a row whose element was REMOVED while
  // focused: focus the person moved elsewhere is left alone.
  const focusedRow = useRef<FocusedRow | null>(null);
  useLayoutEffect(() => {
    const held = focusedRow.current;
    if (held && !held.node.isConnected) {
      const active = document.activeElement;
      const lost = !active || active === document.body;
      const next = lost ? rowElement(roving.containerRef.current, held.key) : null;
      if (next) next.focus({ preventScroll: true });
      else focusedRow.current = null;
    }
  }, [data, openState, showAll, roving.containerRef]);

  const onKeptChange = useRef((id: string, isKept: boolean) => {
    setKept((current) => {
      if (current.has(id) === isKept) return current;
      const next = new Set(current);
      if (isKept) next.add(id);
      else next.delete(id);
      return next;
    });
  }).current;

  const plan = view.plan;
  if (!data || view.groups.length === 0) {
    // A first read that failed has nothing to keep drawing: say so, with Retry.
    return error ? <RefreshError refresh={refresh} /> : null;
  }

  const controls: RowControls = {
    tabIndexFor: roving.tabIndexFor,
    onRowFocus: roving.onRowFocus,
    onOpen,
    onOpenGate,
    plan,
    onKeptChange,
    watchedBy,
  };
  const toggle = (key: string, open: boolean) =>
    setOpenState((current) => ({ ...current, [key]: open }));
  const toggleAll = (key: string) =>
    setShowAll((current) => ({ ...current, [key]: !current[key] }));
  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => roving.onKeyDown(event);
  const onFocus = (event: FocusEvent<HTMLElement>) => {
    roving.onFocus(event);
    const target = event.target;
    const key = target.getAttribute('data-row');
    focusedRow.current = key !== null ? { key, node: target } : null;
    const agent = key?.startsWith('agent:') ? key.slice('agent:'.length) : null;
    if (agent !== focusedAgent) setFocusedAgent(agent);
  };
  const onBlur = (event: FocusEvent<HTMLElement>) => {
    roving.onBlur(event);
    const next = event.relatedTarget;
    if (next instanceof Node && roving.containerRef.current?.contains(next)) return;
    if (next instanceof Node) {
      focusedRow.current = null;
      setFocusedAgent(null);
      return;
    }
    // Focus went nowhere: the window lost focus, a click landed on nothing
    // focusable, or (in some browsers) the focused row was removed. Decide once
    // that removal, if it is one, has been committed: a removed row is the
    // restore's to handle above.
    const left = event.target;
    queueMicrotask(() => {
      if (!left.isConnected) return;
      if (focusedRow.current?.node === left) focusedRow.current = null;
    });
  };

  const allRows = data.subagents;
  const sectionLabel = plan ? copy.plan : copy.agents;
  const sectionName = plan ? copy.planLabel(plan.title) : copy.agents;
  const visibleCount = plan ? (
    summaryCopy.todoCount(completedPhaseCount(plan), plan.phases.length)
  ) : (
    <ListCount rows={allRows} />
  );
  const spokenCount = plan
    ? `${summaryCopy.todoCount(completedPhaseCount(plan), plan.phases.length)}${copy.phasesCountSuffix}`
    : `${summaryCopy.todoCount(subagentCounts(allRows).done, allRows.length)}${copy.doneSuffix}`;
  const lone = !plan ? view.groups[0] : null;

  return (
    <FooterCardGroup>
      <section
        aria-label={sectionName}
        className="br-summary__section br-agents__section"
        data-testid={plan ? 'summary-plan' : 'summary-agents'}
        data-size={view.size}
      >
        <SummarySectionHeading
          label={sectionLabel}
          count={visibleCount}
          open={sectionOpen}
          onOpenChange={setSectionOpen}
          controls={sectionBodyId}
          testId={plan ? 'summary-plan-toggle' : 'summary-agents-toggle'}
        >
          <span className="sr-only" aria-live="polite">
            {spokenCount}
          </span>
        </SummarySectionHeading>
        {sectionOpen ? (
          <div
            id={sectionBodyId}
            ref={roving.containerRef}
            className="br-agents"
            data-size={view.size}
            onKeyDown={onKeyDown}
            onFocus={onFocus}
            onBlur={onBlur}
          >
            {lone ? (
              <GroupBody view={lone} size={view.size} controls={controls} onToggleAll={toggleAll} />
            ) : (
              <ol aria-label={copy.phaseListLabel} className="br-agents__phases">
                {view.groups.map((groupView) => (
                  <GroupItem
                    key={groupView.key}
                    view={groupView}
                    size={view.size}
                    controls={controls}
                    onToggle={toggle}
                    onToggleAll={toggleAll}
                  />
                ))}
              </ol>
            )}
          </div>
        ) : null}
        {error && <RefreshError refresh={refresh} />}
      </section>
    </FooterCardGroup>
  );
}

/**
 * The refresh error and its Retry: a plain button at the error line's 12, so the
 * card keeps its three Button primitives (styles/summaryRail.test.ts).
 */
function RefreshError({ refresh }: { refresh: () => void }) {
  return (
    <div role="alert" className="br-summary__error br-agents__error">
      <span>{copy.refreshFailed}</span>
      <button type="button" className="br-agents__retry" onClick={refresh}>
        {summaryCopy.retry}
      </button>
    </div>
  );
}
