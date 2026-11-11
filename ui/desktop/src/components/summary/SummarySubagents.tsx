import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type FocusEvent,
  type KeyboardEvent,
} from 'react';
import { plainExcerpt } from './plainExcerpt';
import {
  AlertCircle,
  ArrowRight,
  Check,
  ChevronRight,
  CircleDotDashed,
  CircleIcon,
  Lock,
  Plus,
  X,
} from '../icons/app-icons';
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
import { SUBAGENT_STATUS_GLYPHS, SUBAGENT_STATUS_WORDS } from '../subagent/subagentStatus';
import { formatCount } from '../../utils/usageStats';
import {
  GROUP_ROW_LIMIT,
  SUBAGENT_GROUPS,
  completedPhaseCount,
  formatElapsed,
  heldByTitle,
  inSpawnOrder,
  isClosedPhase,
  isQuiet,
  phaseOpenByDefault,
  rowsByPhase,
  subagentCounts,
  subagentGroup,
  subagentListLayout,
  subagentTitle,
  type PhaseSnapshot,
  type PhaseStatus,
  type SessionSubagents,
  type SubagentGroup,
  type SubagentListLayout,
  type SubagentRow,
} from '../../utils/sessionSubagents';
import {
  phaseGateState,
  type PhaseGateSnapshot,
  type PhaseGateState,
} from '../../utils/sessionDecisions';
import { summaryCopy } from './copy';
import { SubagentPeek } from './SubagentPeek';
import { forgetRoster, rememberRoster, rememberedRoster } from './rosterMemory';
import './summaryAgents.css';
import './summaryDecisions.css';

const copy = summaryCopy.agents;

/**
 * The words a phase's gate chip and line say. Kept here, beside the chip that
 * says them, rather than in `summary/copy.ts`.
 */
export const PHASE_GATE_ROW_COPY = {
  waitsFor: (title: string) => `Waits for ${title}'s gate`,
  addGate: 'Add gate',
  addGateLabel: (phase: string) => `Add gate, ${phase}`,
};

/** A gate chip's own words (spec 3.3): the chip, then the sentence its tooltip says. */
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

type Glyph = typeof CircleIcon;

/** A phase's status by SHAPE, the To do list's shapes (never by colour alone). */
const PHASE_STATUS_ICONS: Record<PhaseStatus, Glyph> = {
  pending: CircleIcon,
  in_progress: CircleDotDashed,
  completed: Check,
  blocked: AlertCircle,
  skipped: ArrowRight,
};

/** A gate chip's shapes: the gate dialog's own. */
const GATE_ICONS: Record<PhaseGateState, Glyph> = {
  pending: CircleIcon,
  passed: Check,
  failed: X,
  blocked: AlertCircle,
  skipped: ArrowRight,
};

export interface SummarySubagentsData {
  /**
   * The chat these belong to. With it, the section keeps showing the last roster
   * it drew for this chat while a re-read is out or failed (H-12: a tab switch
   * that remounted the summary showed no Phases block until the next visit).
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
   * chip is drawn, but is not a control, and no phase offers "Add gate".
   */
  onOpenGate?: (phaseId: string) => void;
}

// ---------------------------------------------------------------------------
// Words
// ---------------------------------------------------------------------------

/**
 * A row's second line (spec 4.1): `Waiting to start`, `Working…`, `Quiet for 4m`,
 * `Stopping…`, or once it ended its result line, else its status word.
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

/** A dense row's trailing note: the run time, or `quiet 4m`; nothing without either. */
function denseTrailing(row: SubagentRow): string | null {
  if (isQuiet(row)) return copy.quietShort(formatElapsed(row.quietSeconds) ?? '');
  return formatElapsed(row.elapsedSeconds);
}

/**
 * A gate chip's words (spec 3.3): the chip's one word and the full sentence its
 * tooltip says, with the probability named by its outcome (R2: `P(yes) 0.62 ·
 * threshold 0.80`). A gate's questions are yes/no, so the watched outcome is yes.
 * Without the phase, its status is not known, and a person's decision reads as
 * one that stands.
 */
export function gateChipWords(
  gate: PhaseGateSnapshot,
  phase?: Pick<PhaseSnapshot, 'status'>
): { chip: string; sentence: string; needsYou: boolean } {
  const c = GATE_CHIP_COPY;
  const closed = phase ? isClosedPhase(phase) : false;
  const numbers = thresholdLine({ type: 'yes', p: gate.threshold }, null, gate.lastMinP);
  if (gate.personDecision === 'pass') {
    return {
      chip: c.chip.passed,
      sentence: closed ? c.passedByYou : c.passStands,
      needsYou: false,
    };
  }
  if (gate.personDecision === 'skip') {
    return {
      chip: c.chip.skipped,
      sentence: closed ? c.skippedByYou : c.skipStands,
      needsYou: false,
    };
  }
  const blockedReason =
    gate.attempts >= gate.maxAttempts ? c.blockedAfter(gate.maxAttempts) : c.unevaluated;
  if (gate.needsPerson) {
    return {
      chip: c.chip.needsYou,
      sentence: gate.blocked ? `${c.needsYou}: ${blockedReason}` : c.needsYou,
      needsYou: true,
    };
  }
  if (gate.blocked) {
    return { chip: c.chip.blocked, sentence: `${c.blocked}: ${blockedReason}`, needsYou: false };
  }
  if (gate.lastPassed === true) {
    return { chip: c.chip.passed, sentence: `${c.passed} · ${numbers}`, needsYou: false };
  }
  if (gate.lastPassed === false) {
    const attempt = c.attempt(Math.max(1, gate.attempts), gate.maxAttempts);
    return {
      chip: c.chip.failed,
      sentence: `${c.notPassed} · ${numbers} · ${attempt}`,
      needsYou: false,
    };
  }
  return { chip: c.chip.pending, sentence: `${c.notAsked} · ${numbers}`, needsYou: false };
}

// ---------------------------------------------------------------------------
// The view: what is drawn, and the row keys in document order
// ---------------------------------------------------------------------------

interface RowsSlice {
  shown: SubagentRow[];
  /** The rows "Show all N" would add. */
  hidden: number;
  total: number;
  /** The "Show all" / "Show fewer" row's key, when the list is longer than the limit. */
  moreKey: string | null;
  all: boolean;
}

interface PhaseView {
  phase: PhaseSnapshot;
  key: string;
  open: boolean;
  rows: RowsSlice;
}

interface GroupView {
  group: SubagentGroup;
  key: string;
  open: boolean;
  rows: RowsSlice;
}

interface ListView {
  label: string;
  rows: SubagentRow[];
  /** Comfortable lists are flat; dense ones are grouped by state. */
  flat: RowsSlice | null;
  groups: GroupView[];
}

interface View {
  layout: SubagentListLayout;
  phases: PhaseView[] | null;
  list: ListView | null;
  keys: string[];
  preferredKey: string | null;
}

/** A row whose peek is kept stays in the group it was kept in, so a poll never remounts it. */
type PinnedGroups = ReadonlyMap<string, SubagentGroup>;

const agentKey = (row: SubagentRow) => `agent:${row.sessionId}`;

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

function slice(rows: SubagentRow[], scope: string, showAll: Record<string, boolean>): RowsSlice {
  const all = showAll[scope] === true;
  const long = rows.length > GROUP_ROW_LIMIT;
  const shown = long && !all ? rows.slice(0, GROUP_ROW_LIMIT) : rows;
  return {
    shown,
    hidden: rows.length - shown.length,
    total: rows.length,
    moreKey: long ? `more:${scope}` : null,
    all,
  };
}

function pushSlice(keys: string[], rows: RowsSlice) {
  for (const row of rows.shown) keys.push(agentKey(row));
  if (rows.moreKey) keys.push(rows.moreKey);
}

function buildList(
  label: string,
  rows: SubagentRow[],
  layout: SubagentListLayout,
  scope: string,
  openState: Record<string, boolean>,
  showAll: Record<string, boolean>,
  keys: string[],
  pinned: PinnedGroups
): ListView {
  if (layout === 'comfortable') {
    const flat = slice(rows, scope, showAll);
    pushSlice(keys, flat);
    return { label, rows, flat, groups: [] };
  }
  const groups: GroupView[] = [];
  for (const group of SUBAGENT_GROUPS) {
    const members = rows.filter(
      (row) => (pinned.get(row.sessionId) ?? subagentGroup(row.status)) === group
    );
    if (members.length === 0) continue;
    const key = `group:${scope}:${group}`;
    const open = openState[key] ?? true;
    const view: GroupView = {
      group,
      key,
      open,
      rows: slice(members, `${scope}:${group}`, showAll),
    };
    keys.push(key);
    if (open) pushSlice(keys, view.rows);
    groups.push(view);
  }
  return { label, rows, flat: null, groups };
}

const gateKey = (phase: PhaseSnapshot) => `gate:${phase.id}`;
const addGateKey = (phase: PhaseSnapshot) => `addgate:${phase.id}`;

/** An open phase with no gate offers "Add gate" (to the person, when the chip is a control). */
const offersAddGate = (phase: PhaseSnapshot) => !phase.gate && !isClosedPhase(phase);

function buildView(
  data: SessionSubagents | null,
  openState: Record<string, boolean>,
  showAll: Record<string, boolean>,
  gatesOpen: boolean,
  pinned: PinnedGroups
): View {
  const keys: string[] = [];
  const rows = inSpawnOrder(data?.subagents ?? []);
  const layout = subagentListLayout(rows.length);
  if (!data || (rows.length === 0 && !data.plan)) {
    return { layout, phases: null, list: null, keys, preferredKey: null };
  }
  let phases: PhaseView[] | null = null;
  let list: ListView | null = null;
  if (data.plan) {
    const { phases: byPhase, unphased } = rowsByPhase(data.plan, rows);
    phases = data.plan.phases.map((phase) => {
      const members = byPhase.get(phase.id) ?? [];
      const key = `phase:${phase.id}`;
      const open = openState[key] ?? phaseOpenByDefault(phase, data.plan!.phases.length);
      const view: PhaseView = { phase, key, open, rows: slice(members, key, showAll) };
      if (members.length > 0) keys.push(key);
      // The gate chip (or "Add gate") sits at the end of the phase's line.
      if (gatesOpen && phase.gate) keys.push(gateKey(phase));
      if (gatesOpen && offersAddGate(phase)) keys.push(addGateKey(phase));
      if (members.length > 0 && open) pushSlice(keys, view.rows);
      return view;
    });
    if (unphased.length > 0) {
      list = buildList(
        copy.otherSubagents,
        unphased,
        layout,
        'other',
        openState,
        showAll,
        keys,
        pinned
      );
    }
  } else {
    list = buildList(copy.subagents, rows, layout, 'all', openState, showAll, keys, pinned);
  }
  const firstRunning = rows.find((row) => subagentGroup(row.status) === 'running');
  const runningKey = firstRunning ? agentKey(firstRunning) : null;
  return {
    layout,
    phases,
    list,
    keys,
    preferredKey: runningKey && keys.includes(runningKey) ? runningKey : (keys[0] ?? null),
  };
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

interface RowControls {
  tabIndexFor: (key: string) => 0 | -1;
  onRowFocus: (key: string) => void;
  onOpen: SummarySubagentsData['onOpen'];
  onOpenGate?: SummarySubagentsData['onOpenGate'];
  /** The plan, for the title of the phase whose gate holds another. */
  plan: SessionSubagents['plan'];
  /** A row's peek was kept (with the group it sits in) or let go (`null`). */
  onPinnedChange: (sessionId: string, group: SubagentGroup | null) => void;
}

/**
 * One subagent: the status glyph, its role or name, and one line saying what it
 * is doing or what it found (F-2); dense, one 28px line with the run time. The
 * lock of a private child in muted ink (F-16).
 *
 * Hover peeks, click pins (R8, D-43), on the composer footer's own machinery
 * (`useFooterCard`): a hover previews the peek after 300ms and the row stays at
 * rest; a click, Enter or Space keeps it and marks the row `data-pinned`; Escape,
 * a press outside or a second click closes it. A click no longer opens the tab:
 * "Open in a tab" is in the peek.
 */
function SubagentRowItem({
  row,
  layout,
  controls,
}: {
  row: SubagentRow;
  layout: SubagentListLayout;
  controls: RowControls;
}) {
  const key = agentKey(row);
  const card = useFooterCard();
  const cardId = useId();
  const trigger = footerCardTriggerProps(card, cardId);
  const tiersEnabled = usePrivacyTiersEnabled();
  const Icon = SUBAGENT_STATUS_GLYPHS[row.status];
  const word = SUBAGENT_STATUS_WORDS[row.status];
  const title = subagentTitle(row);
  const quiet = isQuiet(row);
  const isPrivate = tiersEnabled && row.privacyTier === 'private';
  const line = layout === 'comfortable' ? subagentLine(row) : null;
  const trailing = layout === 'dense' ? denseTrailing(row) : null;
  // Named explicitly: the visible spans are laid out as lines, and an accessible
  // name computed from them runs the words together.
  const label = [word, title, line ?? trailing, isPrivate ? copy.private : null]
    .filter(Boolean)
    .join(', ');

  // Keep the row in the group it was kept in while its peek is kept (spec 4.2).
  const { onPinnedChange } = controls;
  const pinnedGroup = card.pinned ? subagentGroup(row.status) : null;
  const pinnedRef = useRef<SubagentGroup | null>(null);
  useEffect(() => {
    if (card.pinned && pinnedRef.current === null) {
      pinnedRef.current = pinnedGroup;
      onPinnedChange(row.sessionId, pinnedGroup);
    } else if (!card.pinned && pinnedRef.current !== null) {
      pinnedRef.current = null;
      onPinnedChange(row.sessionId, null);
    }
  }, [card.pinned, pinnedGroup, onPinnedChange, row.sessionId]);
  useEffect(
    () => () => {
      if (pinnedRef.current !== null) onPinnedChange(row.sessionId, null);
    },
    [onPinnedChange, row.sessionId]
  );

  return (
    <li className="br-agents__item" data-row-item="">
      <FooterCardRoot card={card}>
        <button
          type="button"
          className="br-agents__row"
          data-row={key}
          data-layout={layout}
          data-status={row.status}
          data-quiet={quiet ? '' : undefined}
          data-testid={`subagent-row-${row.sessionId}`}
          aria-label={label}
          tabIndex={controls.tabIndexFor(key)}
          {...trigger}
          onFocus={() => {
            controls.onRowFocus(key);
            trigger.onFocus();
          }}
        >
          <span className="br-summary__glyph br-agents__glyph" aria-hidden="true">
            <Icon />
          </span>
          <span className="br-agents__text" aria-hidden="true">
            <span className="br-agents__name">{title}</span>
            {line ? <span className="br-agents__meta">{line}</span> : null}
          </span>
          {trailing ? (
            <span className="br-agents__trailing" aria-hidden="true">
              {trailing}
            </span>
          ) : null}
          {isPrivate ? (
            <span className="br-agents__lock" aria-hidden="true">
              <Lock />
            </span>
          ) : null}
        </button>
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
              onOpen={() => {
                controls.onOpen(row.sessionId, title);
                card.close();
              }}
            />
          </PopoverContent>
        ) : null}
      </FooterCardRoot>
    </li>
  );
}

function MoreRow({
  rows,
  scope,
  controls,
  onToggle,
}: {
  rows: RowsSlice;
  scope: string;
  controls: RowControls;
  onToggle: (scope: string) => void;
}) {
  if (!rows.moreKey) return null;
  const key = rows.moreKey;
  return (
    <li className="br-agents__item" data-row-item="">
      <button
        type="button"
        className="br-agents__more"
        data-row={key}
        tabIndex={controls.tabIndexFor(key)}
        onFocus={() => controls.onRowFocus(key)}
        onClick={() => onToggle(scope)}
      >
        {rows.all ? copy.showFewer : copy.showAll(rows.total)}
      </button>
    </li>
  );
}

function RowList({
  rows,
  scope,
  layout,
  controls,
  onToggleAll,
  id,
  label,
}: {
  rows: RowsSlice;
  scope: string;
  layout: SubagentListLayout;
  controls: RowControls;
  onToggleAll: (scope: string) => void;
  id?: string;
  label?: string;
}) {
  return (
    <ul id={id} aria-label={label} className="br-agents__list" data-layout={layout}>
      {rows.shown.map((row) => (
        <SubagentRowItem key={row.sessionId} row={row} layout={layout} controls={controls} />
      ))}
      <MoreRow rows={rows} scope={scope} controls={controls} onToggle={onToggleAll} />
    </ul>
  );
}

/**
 * The muted line under a phase, only when there is something to act on (spec
 * 3.1): what it waits for, and its failed or blocked subagents. Running and
 * quiet counts are on the rows.
 */
function phaseNoticeLine(phase: PhaseSnapshot, heldBy: string | null): string {
  const { counts } = phase;
  return [
    heldBy ? PHASE_GATE_ROW_COPY.waitsFor(heldBy) : null,
    counts.failed > 0 ? copy.failed(counts.failed) : null,
    counts.blocked > 0 ? copy.blocked(counts.blocked) : null,
  ]
    .filter(Boolean)
    .join(' · ');
}

/**
 * A phase gate as a 20px chip at the end of its phase's line (owner rule: a
 * gate's evaluations are never transcript rows, and F-1: never a row of its
 * own). Pressing it opens the gate's dialog. Its state is a glyph and a word,
 * never colour alone; the full sentence is its tooltip and its name.
 */
function GateChip({ phase, controls }: { phase: PhaseSnapshot; controls: RowControls }) {
  const gate = phase.gate;
  if (!gate) return null;
  // The gate dialog reads the same rule.
  const state = phaseGateState(gate);
  const Icon = GATE_ICONS[state];
  const words = gateChipWords(gate, phase);
  const name = `${words.sentence}, ${phase.title}`;
  const content = (
    <>
      <span className="br-agents__gate-glyph" aria-hidden="true">
        <Icon />
      </span>
      <span className="br-agents__gate-text" aria-hidden="true">
        {words.chip}
      </span>
    </>
  );
  if (!controls.onOpenGate) {
    return (
      <span
        className="br-agents__gate"
        role="img"
        data-state={state}
        data-static=""
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
 * "Add gate" on an open phase with no gate, drawn as the chip in the chip's
 * slot: quiet until the phase is hovered or the button focused, always in the
 * arrow-key order, so a keyboard finds it the way a pointer does. Pressing it
 * opens the gate dialog on its editor.
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
        <span className="br-agents__gate-text">{PHASE_GATE_ROW_COPY.addGate}</span>
      </button>
    </span>
  );
}

function PhaseItem({
  view,
  layout,
  controls,
  onToggle,
  onToggleAll,
}: {
  view: PhaseView;
  layout: SubagentListLayout;
  controls: RowControls;
  onToggle: (key: string, open: boolean) => void;
  onToggleAll: (scope: string) => void;
}) {
  const { phase, key, open, rows } = view;
  const listId = useId();
  const [hovered, setHovered] = useState(false);
  const Icon = PHASE_STATUS_ICONS[phase.status];
  const total = Math.max(phase.counts.total, rows.total);
  const done = Math.min(phase.counts.done, total);
  // H-12: a phase that used no subagents (its work was the parent's own) shows no
  // "0 of 0"; its glyph says where it stands.
  const count = total > 0 ? summaryCopy.todoCount(done, total) : null;
  const heldBy = heldByTitle(controls.plan, phase);
  const notice = phaseNoticeLine(phase, heldBy);
  // "Read the trials, research phase, in progress, 1 of 3 subagents done": named
  // explicitly, for the reason a subagent row is. The kind lives here only.
  const label = [
    phase.title,
    copy.phaseKindWords(copy.phaseKind[phase.kind]),
    copy.phaseStatus[phase.status].toLowerCase(),
    count ? `${count}${copy.phaseAgentsSuffix}` : null,
    heldBy ? PHASE_GATE_ROW_COPY.waitsFor(heldBy) : null,
  ]
    .filter(Boolean)
    .join(', ');
  const heading = (
    <>
      <span className="br-summary__glyph" aria-hidden="true">
        <Icon />
      </span>
      <span className="br-agents__phase-title" aria-hidden="true">
        {phase.title}
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
      data-status={phase.status}
      data-phase-id={phase.id}
      data-held-by={heldBy ? (phase.heldBy ?? undefined) : undefined}
      onMouseEnter={() => setHovered(true)}
      onMouseLeave={() => setHovered(false)}
    >
      <div className="br-agents__phase-line">
        {rows.total > 0 ? (
          <span data-row-item="" className="br-agents__phase-slot">
            <button
              type="button"
              className="br-agents__phase-toggle"
              data-row={key}
              aria-label={label}
              aria-expanded={open}
              aria-controls={open ? listId : undefined}
              tabIndex={controls.tabIndexFor(key)}
              onFocus={() => controls.onRowFocus(key)}
              onClick={() => onToggle(key, !open)}
            >
              {heading}
              <span
                className="br-agents__chevron"
                data-open={open ? '' : undefined}
                aria-hidden="true"
              >
                <ChevronRight />
              </span>
            </button>
          </span>
        ) : (
          <div className="br-agents__phase-toggle" data-static="" role="group" aria-label={label}>
            {heading}
          </div>
        )}
        <GateChip phase={phase} controls={controls} />
        <AddGateButton phase={phase} controls={controls} shown={hovered} />
      </div>
      {notice ? <p className="br-agents__phase-counts">{notice}</p> : null}
      {open && rows.total > 0 ? (
        <RowList
          id={listId}
          rows={rows}
          scope={key}
          layout={layout}
          controls={controls}
          onToggleAll={onToggleAll}
        />
      ) : null}
    </li>
  );
}

function GroupItem({
  view,
  layout,
  controls,
  onToggle,
  onToggleAll,
  scope,
}: {
  view: GroupView;
  layout: SubagentListLayout;
  controls: RowControls;
  onToggle: (key: string, open: boolean) => void;
  onToggleAll: (scope: string) => void;
  scope: string;
}) {
  const listId = useId();
  const { group, key, open, rows } = view;
  return (
    <div className="br-agents__group" data-group={group}>
      <div data-row-item="">
        <button
          type="button"
          className="br-agents__group-toggle"
          data-row={key}
          aria-label={`${copy.groups[group]}, ${formatCount(rows.total)}`}
          aria-expanded={open}
          aria-controls={open ? listId : undefined}
          tabIndex={controls.tabIndexFor(key)}
          onFocus={() => controls.onRowFocus(key)}
          onClick={() => onToggle(key, !open)}
        >
          <span className="br-agents__group-label">{copy.groups[group]}</span>
          <span className="br-summary__count">{formatCount(rows.total)}</span>
          <span className="br-agents__chevron" data-open={open ? '' : undefined} aria-hidden="true">
            <ChevronRight />
          </span>
        </button>
      </div>
      {open ? (
        <RowList
          id={listId}
          rows={rows}
          scope={`${scope}:${group}`}
          layout={layout}
          controls={controls}
          onToggleAll={onToggleAll}
          label={copy.groups[group]}
        />
      ) : null}
    </div>
  );
}

function ListCount({ rows }: { rows: SubagentRow[] }) {
  const counts = subagentCounts(rows);
  const visible = `${counts.running > 0 ? `${copy.running(counts.running)} · ` : ''}${summaryCopy.todoCount(counts.done, counts.total)}`;
  return (
    <>
      <span className="br-summary__count" aria-hidden="true">
        {visible}
      </span>
      <span className="sr-only" aria-live="polite">
        {summaryCopy.todoCount(counts.done, counts.total)}
        {copy.doneSuffix}
      </span>
    </>
  );
}

/**
 * The chat's subagents, and its phase plan when its agent declared one, inside
 * the Chat summary (the docked rail and its popover alike).
 *
 * Up to four subagents get comfortable two-line rows (status by shape and the
 * role or name, then what it is doing or found). Five or more get dense 28px
 * rows grouped by state, Running, Needs attention and Done, each group
 * collapsible and showing at most 12 rows before "Show all N". With a plan, the
 * subagents sit under their phases, one line per phase (status glyph, title,
 * count, chevron, and the gate's chip), and any in no phase under "Other
 * subagents".
 *
 * A row's hover previews its peek and a click keeps it (one open at a time); the
 * peek holds "Open in a tab" and the setup. The whole area is one tab stop; the
 * arrow keys, Home and End move between rows and open nothing.
 */
export function SummarySubagents({
  sessionId,
  data: given,
  loading,
  error,
  refresh,
  onOpen,
  onOpenGate,
}: SummarySubagentsData) {
  const [openState, setOpenState] = useState<Record<string, boolean>>({});
  const [showAll, setShowAll] = useState<Record<string, boolean>>({});
  const [pinned, setPinned] = useState<ReadonlyMap<string, SubagentGroup>>(new Map());

  // H-12: while a re-read is out or failed, keep drawing what this chat's roster
  // last was; a real answer replaces it, and an answer of "none" forgets it.
  const data =
    given ?? (sessionId && (loading || error) ? rememberedRoster(sessionId) : null) ?? null;
  useEffect(() => {
    if (!sessionId) return;
    if (given) rememberRoster(sessionId, given);
    else if (!loading && !error) forgetRoster(sessionId);
  }, [sessionId, given, loading, error]);

  const view = buildView(data, openState, showAll, !!onOpenGate, pinned);
  const roving = useRovingRows(view.keys, view.preferredKey);

  // A row can be remounted under the person's focus. A poll that moves a child
  // to another state group (it finished), or a fifth spawn that turns the flat
  // list into groups, draws the row under a different parent, and React keys
  // match only siblings, so its old button is removed and focus falls to the
  // body. After every render that could redraw rows (new data, a disclosure),
  // put focus back on the same row's new button. Only for a row whose element
  // was REMOVED while focused: focus the person moved elsewhere is left alone.
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

  const onPinnedChange = useRef((id: string, group: SubagentGroup | null) => {
    setPinned((current) => {
      if (group === null ? !current.has(id) : current.get(id) === group) return current;
      const next = new Map(current);
      if (group === null) next.delete(id);
      else next.set(id, group);
      return next;
    });
  }).current;

  const plan = data?.plan ?? null;
  if (!data || (!view.phases && !view.list)) {
    // A first read that failed has nothing to keep drawing: say so, with Retry.
    return error ? <RefreshError refresh={refresh} /> : null;
  }

  const controls: RowControls = {
    tabIndexFor: roving.tabIndexFor,
    onRowFocus: roving.onRowFocus,
    onOpen,
    onOpenGate,
    plan,
    onPinnedChange,
  };
  const toggle = (key: string, open: boolean) =>
    setOpenState((current) => ({ ...current, [key]: open }));
  const toggleAll = (scope: string) =>
    setShowAll((current) => ({ ...current, [scope]: !current[scope] }));
  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => roving.onKeyDown(event);
  const onFocus = (event: FocusEvent<HTMLElement>) => {
    roving.onFocus(event);
    const target = event.target;
    const key = target.getAttribute('data-row');
    focusedRow.current = key !== null ? { key, node: target } : null;
  };
  const onBlur = (event: FocusEvent<HTMLElement>) => {
    roving.onBlur(event);
    const next = event.relatedTarget;
    if (next instanceof Node && roving.containerRef.current?.contains(next)) return;
    if (next instanceof Node) {
      focusedRow.current = null;
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

  return (
    <FooterCardGroup>
      <div
        ref={roving.containerRef}
        className="br-agents"
        data-layout={view.layout}
        onKeyDown={onKeyDown}
        onFocus={onFocus}
        onBlur={onBlur}
      >
        {plan && view.phases && (
          <section
            aria-label={copy.phasesLabel(plan.title)}
            className="br-summary__section br-agents__section"
          >
            <div className="br-summary__heading">
              <h3 className="br-summary__label">{copy.phases}</h3>
              <span className="br-summary__count" aria-hidden="true">
                {summaryCopy.todoCount(completedPhaseCount(plan), plan.phases.length)}
              </span>
              <span className="sr-only" aria-live="polite">
                {summaryCopy.todoCount(completedPhaseCount(plan), plan.phases.length)}
                {copy.phasesCountSuffix}
              </span>
            </div>
            <ol aria-label={copy.phaseListLabel} className="br-agents__phases">
              {view.phases.map((phaseView) => (
                <PhaseItem
                  key={phaseView.phase.id}
                  view={phaseView}
                  layout={view.layout}
                  controls={controls}
                  onToggle={toggle}
                  onToggleAll={toggleAll}
                />
              ))}
            </ol>
          </section>
        )}
        {view.list && (
          <section aria-label={view.list.label} className="br-summary__section br-agents__section">
            <div className="br-summary__heading">
              <h3 className="br-summary__label">{view.list.label}</h3>
              <ListCount rows={view.list.rows} />
            </div>
            {view.list.flat ? (
              <RowList
                rows={view.list.flat}
                scope={plan ? 'other' : 'all'}
                layout={view.layout}
                controls={controls}
                onToggleAll={toggleAll}
                label={copy.subagentListLabel}
              />
            ) : (
              view.list.groups.map((group) => (
                <GroupItem
                  key={group.key}
                  view={group}
                  layout={view.layout}
                  controls={controls}
                  onToggle={toggle}
                  onToggleAll={toggleAll}
                  scope={plan ? 'other' : 'all'}
                />
              ))
            )}
          </section>
        )}
        {error && <RefreshError refresh={refresh} />}
      </div>
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
