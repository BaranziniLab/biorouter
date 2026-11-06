import {
  Fragment,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type FocusEvent,
  type KeyboardEvent,
  type ReactNode,
} from 'react';
import {
  AlertTriangle,
  Check,
  CheckCircle,
  ChevronRight,
  CircleDotDashed,
  CircleHelp,
  CircleIcon,
  Code,
  Eye,
  FlaskConical,
  Hash,
  Layers,
  ListTodo,
  Lock,
  MessagesSquare,
  MoreHorizontal,
  PauseCircle,
  Play,
  ScrollText,
  Search,
  StopCircle,
  X,
} from '../icons/app-icons';
import { ENTITY_ICONS } from '../icons/entity-icons';
import { ChatKindIcon } from '../chats/ChatKindIcon';
import { usePrivacyTiersEnabled } from '../ConfigContext';
import { Progress } from '../ui/progress';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { useRovingRows } from '../ui/useRovingRows';
import { formatCount, formatTokensCompact } from '../../utils/usageStats';
import {
  GROUP_ROW_LIMIT,
  SUBAGENT_GROUPS,
  completedPhaseCount,
  formatElapsed,
  inSpawnOrder,
  isQuiet,
  phaseOpenByDefault,
  rowsByPhase,
  rowsInGroup,
  shortModelLabel,
  subagentCounts,
  subagentGroup,
  subagentListLayout,
  subagentSetup,
  subagentTitle,
  type PhaseKind,
  type PhaseSnapshot,
  type SessionSubagents,
  type SubagentSetup,
  type SubagentGroup,
  type SubagentListLayout,
  type SubagentRow,
  type SubagentRunStatus,
} from '../../utils/sessionSubagents';
import { formatProbability, type PhaseGateSnapshot } from '../../utils/sessionDecisions';
import { summaryCopy } from './copy';
import './summaryAgents.css';
import './summaryDecisions.css';

const copy = summaryCopy.agents;

type Glyph = typeof CircleIcon;

/**
 * A child's state, told apart by SHAPE (never by colour alone), keyed by status
 * so a status added to the daemon fails the build here instead of falling back
 * to one of the others. The word itself is read to assistive technology ahead
 * of the name.
 */
const STATUS_ICONS: Record<SubagentRunStatus, Glyph> = {
  queued: CircleIcon,
  running: CircleDotDashed,
  cancelling: StopCircle,
  completed: Check,
  blocked: CircleHelp,
  incomplete: AlertTriangle,
  failed: X,
  interrupted: PauseCircle,
  unknown: MoreHorizontal,
};

/** What a phase is for, drawn as its leading glyph. A glyph, not a behaviour. */
const PHASE_ICONS: Record<PhaseKind, Glyph> = {
  plan: ListTodo,
  research: Search,
  execution: Play,
  implementation: Code,
  verification: CheckCircle,
  testing: FlaskConical,
  auditing: ScrollText,
  review: Eye,
  meeting: MessagesSquare,
  synthesis: Layers,
  other: Hash,
};

/**
 * The hover card waits for intent, as the sidebar's chat-row card does
 * (`CHAT_HOVER_CARD_DELAY_MS` in `BioRouterSidebar/RecentChats.tsx`).
 */
export const SUBAGENT_CARD_DELAY_MS = 700;

/** At most this many names per list in the hover card, then "+N more". */
const CARD_NAME_LIMIT = 8;

export interface SummarySubagentsData {
  data: SessionSubagents | null;
  loading: boolean;
  error: boolean;
  refresh: () => void;
  /** Open the subagent as a tab and make it the active one. */
  onOpen: (sessionId: string, title?: string) => void;
  /**
   * Open the decision history filtered to a phase's gate. Without it a phase's
   * gate chip is drawn, but is not a control.
   */
  onOpenGate?: (phaseId: string) => void;
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
  keys: string[]
): ListView {
  if (layout === 'comfortable') {
    const flat = slice(rows, scope, showAll);
    pushSlice(keys, flat);
    return { label, rows, flat, groups: [] };
  }
  const groups: GroupView[] = [];
  for (const group of SUBAGENT_GROUPS) {
    const members = rowsInGroup(rows, group);
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

function buildView(
  data: SessionSubagents | null,
  openState: Record<string, boolean>,
  showAll: Record<string, boolean>,
  gatesOpen = false
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
      // The gate chip sits under the phase's heading, before its rows.
      if (gatesOpen && phase.gate) keys.push(gateKey(phase));
      if (members.length > 0 && open) pushSlice(keys, view.rows);
      return view;
    });
    if (unphased.length > 0) {
      list = buildList(copy.otherSubagents, unphased, layout, 'other', openState, showAll, keys);
    }
  } else {
    list = buildList(copy.subagents, rows, layout, 'all', openState, showAll, keys);
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
// The hover card
// ---------------------------------------------------------------------------

function nameList(names: string[]): string {
  if (names.length === 0) return copy.card.none;
  const shown = names.slice(0, CARD_NAME_LIMIT).join(', ');
  const more = names.length - CARD_NAME_LIMIT;
  return more > 0 ? `${shown}, +${more} more` : shown;
}

function statusLine(row: SubagentRow): string {
  const parts: string[] = [copy.status[row.status]];
  if (isQuiet(row)) parts.push(copy.quietFor(formatElapsed(row.quietSeconds) ?? ''));
  const head = parts.join(' · ');
  return row.resultLine ? `${head}: ${row.resultLine}` : head;
}

function SubagentCard({ row, phaseTitle }: { row: SubagentRow; phaseTitle: string | null }) {
  const tiersEnabled = usePrivacyTiersEnabled();
  const model = row.modelName
    ? row.providerName
      ? `${row.modelName} (${row.providerName})`
      : row.modelName
    : (row.providerName ?? copy.card.none);
  const figures = [
    row.totalTokens !== null
      ? `${formatTokensCompact(row.totalTokens)} ${copy.card.tokens.toLowerCase()}`
      : null,
    `${formatCount(row.toolCallCount)} ${copy.card.toolCalls.toLowerCase()}`,
    `${formatCount(row.messageCount)} ${copy.card.messages.toLowerCase()}`,
    formatElapsed(row.elapsedSeconds),
  ].filter((part): part is string => !!part);
  const ModelIcon = ENTITY_ICONS.model;
  const ExtensionIcon = ENTITY_ICONS.extension;
  const SkillIcon = ENTITY_ICONS.skill;
  const KnowledgeIcon = ENTITY_ICONS.knowledge;
  return (
    <div className="br-agents-card" data-testid={`subagent-card-${row.sessionId}`}>
      <p className="br-agents-card__title">
        <ChatKindIcon
          session={{ session_type: 'sub_agent' }}
          tier={row.privacyTier}
          testId={`subagent-card-glyph-${row.sessionId}`}
        />
        <span>{row.name || subagentTitle(row)}</span>
      </p>
      {(row.role || phaseTitle) && (
        <p className="br-agents-card__line">
          {[
            row.role ? `${copy.card.role}: ${row.role}` : null,
            phaseTitle ? `${copy.card.phase}: ${phaseTitle}` : null,
          ]
            .filter(Boolean)
            .join(' · ')}
        </p>
      )}
      <p className="br-agents-card__line br-agents-card__status">{statusLine(row)}</p>
      <dl className="br-agents-card__facts">
        <dt>
          <ModelIcon aria-hidden="true" />
          {copy.card.model}
        </dt>
        <dd>{model}</dd>
        <dt>
          <ExtensionIcon aria-hidden="true" />
          {copy.card.extensions}
        </dt>
        <dd>{nameList(row.extensions)}</dd>
        <dt>
          <SkillIcon aria-hidden="true" />
          {copy.card.skills}
        </dt>
        <dd>{nameList(row.skills)}</dd>
        <dt>
          <KnowledgeIcon aria-hidden="true" />
          {copy.card.knowledgeBases}
        </dt>
        <dd>{nameList(row.knowledgeBases)}</dd>
      </dl>
      <p className="br-agents-card__line">{figures.join(' · ')}</p>
      {tiersEnabled && row.privacyTier === 'private' && (
        <p className="br-agents-card__line br-agents-card__private">
          <Lock aria-hidden="true" />
          {copy.card.private}
        </p>
      )}
      <p className="br-agents-card__hint">{copy.card.openHint}</p>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Rows
// ---------------------------------------------------------------------------

/**
 * A comfortable row's second line: `opus-4-5 · [puzzle]2 [skill]1 [graph]1 · 4m`.
 *
 * Each setup count is its entity's one glyph (`ENTITY_ICONS`, the marks the
 * sidebar's Extensions, Skills and Knowledge rows wear) and a number, named in
 * words ("1 knowledge base") for whatever reads the pair on its own. The row
 * itself is read by its `aria-label`, which says the same words. A count of zero
 * is not drawn.
 *
 * The model name is the part that gives way (`br-agents__fit`, cut with an
 * ellipsis), and the hover card has it in full. The time gives way only once the
 * model is gone, which takes the rail's narrowest 240px, a lock and a quiet note.
 * The counts never do. The time is the run time, or on a quiet child the quiet
 * note in its place, as on a dense row: both did not fit beside the counts.
 */
function setupLine(setup: SubagentSetup, time: string | null): ReactNode {
  const segments: { key: string; node: ReactNode }[] = [];
  if (setup.model) {
    segments.push({ key: 'model', node: <span className="br-agents__fit">{setup.model}</span> });
  }
  if (setup.counts.length > 0) {
    segments.push({
      key: 'counts',
      node: (
        <span className="br-agents__counts">
          {setup.counts.map(({ kind, count }) => {
            const Icon = ENTITY_ICONS[kind];
            return (
              <span
                key={kind}
                className="br-agents__count"
                data-count={kind}
                role="img"
                aria-label={copy.setupCount[kind](count)}
              >
                <Icon aria-hidden="true" />
                {count}
              </span>
            );
          })}
        </span>
      ),
    });
  }
  if (time) segments.push({ key: 'time', node: <span className="br-agents__time">{time}</span> });
  if (segments.length === 0) return null;
  return segments.map(({ key, node }, index) => (
    <Fragment key={key}>
      {index > 0 ? <span className="br-agents__sep"> · </span> : null}
      {node}
    </Fragment>
  ));
}

interface RowControls {
  tabIndexFor: (key: string) => 0 | -1;
  onRowFocus: (key: string) => void;
  /** The row whose card a keyboard move opened, if any. */
  keyboardCard: string | null;
  onCardClosed: (key: string) => void;
  onOpen: SummarySubagentsData['onOpen'];
  onOpenGate?: SummarySubagentsData['onOpenGate'];
  phaseTitles: Map<string, string>;
}

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
  const [hoverOpen, setHoverOpen] = useState(false);
  const shown = hoverOpen || controls.keyboardCard === key;
  const [built, setBuilt] = useState(false);
  if (shown && !built) setBuilt(true);
  const tiersEnabled = usePrivacyTiersEnabled();
  const Icon = STATUS_ICONS[row.status];
  const title = subagentTitle(row);
  const quiet = isQuiet(row);
  const quietText = quiet ? copy.quietFor(formatElapsed(row.quietSeconds) ?? '') : null;
  const phaseTitle = row.phaseId ? (controls.phaseTitles.get(row.phaseId) ?? null) : null;
  const isPrivate = tiersEnabled && row.privacyTier === 'private';
  const trailing =
    layout === 'dense'
      ? quiet
        ? quietText
        : (formatElapsed(row.elapsedSeconds) ?? shortModelLabel(row.modelName, row.providerName))
      : null;
  const setup = layout === 'comfortable' ? subagentSetup(row) : null;
  // A quiet child's note takes the run time's place, on both layouts.
  const setupTime = setup ? (quietText ?? setup.elapsed) : null;
  const meta = setup ? setupLine(setup, setupTime) : null;
  // Named explicitly: the visible spans are laid out as lines, and an accessible
  // name computed from them runs the words together. Commas, not the visible
  // middle dots, which a screen reader may read out as "dot", and the setup
  // counts as words, not as their glyphs and numbers.
  const label = [
    `${copy.status[row.status]}, ${title}`,
    ...(setup
      ? [
          setup.model,
          ...setup.counts.map(({ kind, count }) => copy.setupCount[kind](count)),
          setupTime,
        ]
      : [trailing]),
    isPrivate ? copy.card.private : null,
  ]
    .filter(Boolean)
    .join(', ');

  return (
    <li className="br-agents__item" data-row-item="">
      <Tooltip
        delayDuration={SUBAGENT_CARD_DELAY_MS}
        open={shown}
        onOpenChange={(open) => {
          setHoverOpen(open);
          if (!open) controls.onCardClosed(key);
        }}
      >
        <TooltipTrigger asChild>
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
            onFocus={() => controls.onRowFocus(key)}
            onClick={() => controls.onOpen(row.sessionId, row.name || undefined)}
          >
            <span className="br-summary__glyph br-agents__glyph" aria-hidden="true">
              <Icon />
            </span>
            <span className="br-agents__text" aria-hidden="true">
              <span className="br-agents__name">{title}</span>
              {meta ? <span className="br-agents__meta">{meta}</span> : null}
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
        </TooltipTrigger>
        {built ? (
          <TooltipContent
            side="left"
            align="start"
            sideOffset={12}
            hideWhenDetached
            className="br-agents-card-surface"
          >
            <SubagentCard row={row} phaseTitle={phaseTitle} />
          </TooltipContent>
        ) : null}
      </Tooltip>
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

function phaseCountsLine(phase: PhaseSnapshot): string {
  const { counts } = phase;
  return [
    counts.running > 0 ? copy.running(counts.running) : null,
    counts.quiet > 0 ? copy.quiet(counts.quiet) : null,
    counts.blocked > 0 ? copy.blocked(counts.blocked) : null,
    counts.failed > 0 ? copy.failed(counts.failed) : null,
    phase.status === 'blocked' || phase.status === 'skipped'
      ? copy.phaseStatus[phase.status]
      : null,
  ]
    .filter(Boolean)
    .join(' · ');
}

/** `Gate 0.91 ≥ 0.80 passed`, `Gate 0.62 < 0.80, attempt 2 of 3`, `Gate blocked`. */
export function gateChipText(gate: PhaseGateSnapshot): string {
  const gateCopy = summaryCopy.decisions.gate;
  const t = formatProbability(gate.threshold);
  if (gate.blocked) return gateCopy.blocked;
  if (gate.lastPassed === true && gate.lastMinP !== null) {
    return gateCopy.passed(formatProbability(gate.lastMinP), t);
  }
  if (gate.lastPassed === false && gate.lastMinP !== null) {
    return gateCopy.failed(
      formatProbability(gate.lastMinP),
      t,
      Math.max(1, gate.attempts),
      gate.maxAttempts
    );
  }
  return gateCopy.pending(t);
}

function gateState(gate: PhaseGateSnapshot): 'blocked' | 'passed' | 'failed' | 'pending' {
  if (gate.blocked) return 'blocked';
  if (gate.lastPassed === true) return 'passed';
  if (gate.lastPassed === false) return 'failed';
  return 'pending';
}

const GATE_ICONS: Record<ReturnType<typeof gateState>, Glyph> = {
  pending: CircleIcon,
  passed: Check,
  failed: X,
  blocked: AlertTriangle,
};

/**
 * A phase gate as a chip on its phase row (owner rule: a gate's evaluations are
 * never transcript rows). Pressing it opens the decision history filtered to
 * that gate. Its state is a glyph and its words, never colour alone.
 */
function GateChip({ phase, controls }: { phase: PhaseSnapshot; controls: RowControls }) {
  const gate = phase.gate;
  if (!gate) return null;
  const state = gateState(gate);
  const Icon = GATE_ICONS[state];
  const text = gateChipText(gate);
  const content = (
    <>
      <span className="br-summary__glyph" aria-hidden="true">
        <Icon />
      </span>
      <span className="br-agents__gate-text">{text}</span>
    </>
  );
  if (!controls.onOpenGate) {
    return (
      <p className="br-agents__phase-counts" data-gate-state={state}>
        {text}
      </p>
    );
  }
  const key = gateKey(phase);
  const open = controls.onOpenGate;
  return (
    <div data-row-item="">
      <button
        type="button"
        className="br-agents__gate"
        data-row={key}
        data-state={state}
        data-testid={`phase-gate-${phase.id}`}
        aria-label={`${text}, ${phase.title}`}
        tabIndex={controls.tabIndexFor(key)}
        onFocus={() => controls.onRowFocus(key)}
        onClick={() => open(phase.id)}
      >
        {content}
      </button>
    </div>
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
  const Icon = PHASE_ICONS[phase.kind];
  const total = Math.max(phase.counts.total, rows.total);
  const done = Math.min(phase.counts.done, total);
  const countsLine = phaseCountsLine(phase);
  // "Read the trials, Research, In progress, 1 of 3 subagents done": named
  // explicitly, for the reason a subagent row is.
  const label = `${phase.title}, ${copy.phaseKind[phase.kind]}, ${copy.phaseStatus[phase.status]}, ${summaryCopy.todoCount(done, total)}${copy.phaseAgentsSuffix}`;
  const heading = (
    <>
      <span className="br-summary__glyph" aria-hidden="true">
        <Icon />
      </span>
      <span className="br-agents__phase-title" aria-hidden="true">
        {phase.title}
      </span>
      <span className="br-summary__count" aria-hidden="true">
        {summaryCopy.todoCount(done, total)}
      </span>
    </>
  );
  return (
    <li className="br-agents__phase" data-status={phase.status} data-phase-id={phase.id}>
      {rows.total > 0 ? (
        <div data-row-item="">
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
        </div>
      ) : (
        <div className="br-agents__phase-toggle" data-static="" role="group" aria-label={label}>
          {heading}
        </div>
      )}
      <Progress
        size="thin"
        className="br-summary__progress br-agents__phase-progress"
        value={done}
        max={total > 0 ? total : 1}
        label={copy.phaseProgressLabel(phase.title)}
      />
      {countsLine ? <p className="br-agents__phase-counts">{countsLine}</p> : null}
      <GateChip phase={phase} controls={controls} />
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
 * name or role, then model, setup and run time). Five or more get dense 28px
 * rows grouped by state, Running, Needs attention and Done, each group
 * collapsible and showing at most 12 rows before "Show all N". With a plan, the
 * subagents sit under their phases, each phase a disclosure with its own
 * progress, and any in no phase under "Other subagents".
 *
 * Every row is a button that opens the subagent as a tab. Hovering it, or
 * reaching it with the keyboard, shows a card with its full setup. The whole
 * area is one tab stop; the arrow keys, Home and End move between rows.
 */
export function SummarySubagents({
  data,
  error,
  refresh,
  onOpen,
  onOpenGate,
}: SummarySubagentsData) {
  const [openState, setOpenState] = useState<Record<string, boolean>>({});
  const [showAll, setShowAll] = useState<Record<string, boolean>>({});
  const [keyboardCard, setKeyboardCard] = useState<string | null>(null);
  const arrowMove = useRef(false);

  const view = buildView(data, openState, showAll, !!onOpenGate);
  const roving = useRovingRows(view.keys, view.preferredKey);

  // A row can be remounted under the person's focus. A poll that moves a child
  // to another state group (it finished), or a fifth spawn that turns the flat
  // list into groups, draws the row under a different parent, and React keys
  // match only siblings, so its old button is removed and focus falls to the
  // body. After every render that could redraw rows (new data, a disclosure),
  // put focus back on the same row's new button, and its card with it when the
  // card was open. Only for a row whose element was REMOVED while focused:
  // focus the person moved elsewhere is left alone.
  const focusedRow = useRef<FocusedRow | null>(null);
  const committedCard = useRef<string | null>(null);
  useLayoutEffect(() => {
    const held = focusedRow.current;
    if (held && !held.node.isConnected) {
      const active = document.activeElement;
      const lost = !active || active === document.body;
      const next = lost ? rowElement(roving.containerRef.current, held.key) : null;
      if (next) {
        if (committedCard.current === held.key) setKeyboardCard(held.key);
        next.focus({ preventScroll: true });
      } else {
        focusedRow.current = null;
      }
    }
    committedCard.current = keyboardCard;
  }, [data, openState, showAll, keyboardCard, roving.containerRef]);

  if (!data || (!view.phases && !view.list)) return null;

  const phaseTitles = new Map((data.plan?.phases ?? []).map((phase) => [phase.id, phase.title]));
  const controls: RowControls = {
    tabIndexFor: roving.tabIndexFor,
    onRowFocus: (key) => {
      roving.onRowFocus(key);
      // A move with the arrow keys is programmatic focus, which a tooltip
      // declines on its own (`ui/Tooltip.tsx`); the person asked for it, so the
      // row's card opens anyway.
      if (arrowMove.current) {
        arrowMove.current = false;
        setKeyboardCard(key.startsWith('agent:') ? key : null);
      }
    },
    keyboardCard,
    onCardClosed: (key) => setKeyboardCard((current) => (current === key ? null : current)),
    onOpen,
    onOpenGate,
    phaseTitles,
  };
  const toggle = (key: string, open: boolean) =>
    setOpenState((current) => ({ ...current, [key]: open }));
  const toggleAll = (scope: string) =>
    setShowAll((current) => ({ ...current, [scope]: !current[scope] }));
  const onKeyDown = (event: KeyboardEvent<HTMLElement>) => {
    arrowMove.current = true;
    roving.onKeyDown(event);
    arrowMove.current = false;
  };
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
      setKeyboardCard(null);
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
      setKeyboardCard(null);
    });
  };

  const plan = data.plan;
  return (
    <div
      ref={roving.containerRef}
      className="br-agents"
      data-layout={view.layout}
      onKeyDown={onKeyDown}
      onFocus={onFocus}
      onBlur={onBlur}
    >
      {plan && view.phases && (
        <section aria-label={copy.phases} className="br-summary__section br-agents__section">
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
          {plan.title ? <p className="br-agents__plan-title">{plan.title}</p> : null}
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
      {error && (
        <div role="alert" className="br-summary__error br-agents__error">
          <span>{copy.refreshFailed}</span>
          <button type="button" className="br-agents__retry" onClick={refresh}>
            {summaryCopy.retry}
          </button>
        </div>
      )}
    </div>
  );
}
