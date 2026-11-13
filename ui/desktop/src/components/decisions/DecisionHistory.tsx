import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
} from 'react';
import { AlertCircle, ChevronDown, RefreshCw, SlidersHorizontal, X } from '../icons/app-icons';
import { Button } from '../ui/button';
import { Disclosure } from '../ui/disclosure';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { FacetChip } from '../ui/facet-chip';
import { InfoTip } from '../ui/info-tip';
import { Input } from '../ui/input';
import { Popover, PopoverContent, PopoverTrigger } from '../ui/popover';
import { SegmentedControl } from '../ui/segmented-control';
import { PreviewBarActions, PreviewBarMeta, PreviewTip } from '../artifacts/PreviewBar';
import { formatCount } from '../../utils/usageStats';
import {
  answerKey,
  answerProbability,
  calibrationText,
  decisionModelLabel,
  fetchDecisionDetail,
  fetchDecisionPage,
  fetchDecisionSummary,
  formatLatency,
  formatProbability,
  outcomeLabel,
  readableAnswer,
  thresholdOnOption,
  type Decision,
  type DecisionContext,
  type DecisionDetail,
  type DecisionHistoryFilter,
  type DecisionOrigin,
  type DecisionRecord,
  type NamedQuestion,
  type SessionDecisionSummary,
} from '../../utils/sessionDecisions';
import { ProbabilityBar } from './ProbabilityBar';
import {
  CALIBRATION_COPY,
  FILTER_CHIP_COPY,
  OUTCOME_FILTER_OPTIONS,
  RECORD_GLYPHS,
  RECORD_GLYPH_TONES,
  decisionModelDisplay,
  decisionModelPlace,
  formatDecisionTime,
  namedProbability,
  outcomeFilterChoice,
  outcomeKeptFor,
  recordGlyphState,
  recordSubtitle,
  recordTitle,
  recordVerdict,
  sourceFilterOptionsFor,
  thresholdLine,
  verdictWord,
  watchedProbability,
} from './decisionWords';
import './decisionsPanel.css';

/** Every list row is this tall (two lines). The list's windowing depends on it. */
export const DECISION_ROW_HEIGHT = 44;
/** Rows drawn above and below the ones in view. */
const OVERSCAN = 8;
/** A page this size; the daemon caps it at 200. */
export const DECISION_PAGE_SIZE = 100;
/** The next page is asked for once the last loaded row is this close to the view. */
const LOAD_AHEAD_ROWS = 20;

export const DECISION_HISTORY_COPY = {
  listLabel: 'Decision calls',
  filtersLabel: 'Filter decision calls',
  source: FILTER_CHIP_COPY.source,
  outcome: FILTER_CHIP_COPY.outcome,
  filters: 'Filters',
  /** The Filters chip once something inside it is set: `Filters: monitor, turn`. */
  filtersSet: (nouns: string[]) => `Filters: ${nouns.join(', ')}`,
  filtersSetName: (nouns: string[]) => `Filters, set: ${nouns.join(', ')}`,
  turn: 'Turn',
  anyTurn: 'Any turn',
  earlierTurn: 'An earlier turn',
  notListed: "Not in this chat's list",
  pRange: 'Probability range',
  pRangeHelp:
    'The probability of the answer a threshold watches, such as P(yes) for a monitor that fires on yes. Calls with no threshold are left out.',
  minP: 'Lowest probability',
  maxP: 'Highest probability',
  pRangeInvalid: 'Use numbers from 0 to 1, the lower one first.',
  to: 'to',
  batchOnly: 'This decide call only',
  removeBatch: 'Remove the filter: This decide call only',
  callOnly: 'This call only',
  removeCall: 'Remove the filter: This call only',
  clear: 'Clear filters',
  refresh: 'Refresh',
  showNew: (n: number) => `Show ${formatCount(n)} new`,
  calls: (n: number) => `${formatCount(n)} ${n === 1 ? 'call' : 'calls'}`,
  callCount: (n: number) => `${formatCount(n)} ${n === 1 ? 'call' : 'calls'}`,
  chunkCount: (n: number) => `${formatCount(n)} ${n === 1 ? 'chunk' : 'chunks'}`,
  loading: 'Loading decision calls',
  loadingMore: 'Loading more',
  loadFailed: 'Could not load decision calls.',
  retry: 'Retry',
  empty: 'This chat has made no decision calls yet.',
  emptyFiltered: 'No decision calls match these filters.',
  noSelection: 'Choose a call to see what it saw and how it answered.',
  detailFailed: 'Could not load this call.',
  detailMissing: 'This call is no longer in the log.',
  details: 'Details',
  show: 'Show',
  whatItSaw: 'What it saw',
  raw: 'Raw JSON',
  contextTruncated: (kb: string) => `Cut to the first 64 KB of ${kb}.`,
  contextLoading: 'Loading the context',
  contextUnavailable: 'The stored context is not available.',
  question: (n: number) => `Question ${n}`,
  /** Kept for the decision chat card's test (DecisionCard.test.tsx reads it). */
  failed: 'Failed',
  certainty: 'Certainty',
  certaintyHelp:
    'How sure the model is across all answers: 0 means every answer is equally likely, 1 means one answer holds all the probability. It is not the answer’s probability.',
  model: 'Model',
  servedBy: 'Served by',
  /** The decision MODEL's tier, not the chat's: a public chat may use a local model. */
  tier: 'Model tier',
  latency: 'Latency',
  tokens: 'Tokens',
  probabilities: 'Probabilities',
  chunk: 'Chunk',
  chunkNumber: (n: number) => `Chunk ${n}`,
  error: 'Error',
  privacyOff: 'Privacy tiers were off when this call ran.',
  private: 'Private',
  public: 'Public',
  noAnswer: 'No answer',
  /** The detail's muted line: the kind of call, never its title again. */
  kind: {
    monitor: 'Monitor',
    monitors: 'Monitors',
    gate: 'Phase gate',
    tool: 'Decide call',
    chat: 'Decision chat',
    adhoc: 'Ad hoc call',
  },
  /** The which-one chip's noun per source, and the Filters chip's word for it. */
  which: {
    monitor: { noun: 'Monitor', any: 'Any monitor', set: 'monitor' },
    gate: { noun: 'Phase gate', any: 'Any phase gate', set: 'gate' },
    tool: { noun: 'Decide call', any: 'Any decide call', set: 'decide call' },
    chat: { noun: 'Decision chat', any: 'Any decision chat', set: 'decision chat' },
    adhoc: { noun: 'Ad hoc call', any: 'Any ad hoc call', set: 'ad hoc call' },
  } satisfies Record<DecisionOrigin, { noun: string; any: string; set: string }>,
};

const copy = DECISION_HISTORY_COPY;

/** How often an open history checks for new calls (one read of its first call). */
export const FOLLOW_INTERVAL_MS = 4000;
/** Within this many pixels of the top, new calls reload the list in place. */
const FOLLOW_TOP_SLACK_PX = 24;

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

const SHORT_TIME = new Intl.DateTimeFormat(undefined, { hour: 'numeric', minute: '2-digit' });
const SHORT_DAY_TIME = new Intl.DateTimeFormat(undefined, {
  month: 'short',
  day: 'numeric',
  hour: 'numeric',
  minute: '2-digit',
});

/** `1:26 PM` today, `Nov 4, 1:26 PM` before: a turn or a decide call in a menu. */
export function formatShortTime(iso: string, now = new Date()): string {
  const date = new Date(iso);
  if (!Number.isFinite(date.getTime())) return '';
  return date.toDateString() === now.toDateString()
    ? SHORT_TIME.format(date)
    : SHORT_DAY_TIME.format(date);
}

/** The names a several-monitor call asked under (the daemon joins them with ` + `). */
function labelNames(record: DecisionRecord): string[] {
  return (record.label ?? '')
    .split(' + ')
    .map((name) => name.trim())
    .filter(Boolean);
}

/** A monitor call that asked several monitors at once (`decisionWords`' own rule). */
function isSeveralMonitors(record: DecisionRecord): boolean {
  return (
    record.origin === 'monitor' &&
    Math.max(record.questions.length, record.decisions.length, labelNames(record).length) > 1
  );
}

/** The question a decision answered, by name. */
function questionOf(record: DecisionRecord, decision: Decision): NamedQuestion | null {
  return record.questions.find((question) => question.name === decision.question) ?? null;
}

/**
 * What a ONE-question row draws on its second line's right (R2): for a monitor or a
 * gate the watched outcome's probability, named by the threshold (`P(yes) 0.12`);
 * for a decide, chat or ad hoc call the answer's own probability (`P(no) 0.99`).
 * `null` for a call that asked several questions, or answered nothing.
 */
export function rowProbability(record: DecisionRecord): { p: number; text: string } | null {
  if (record.decisions.length !== 1 || record.questions.length > 1) return null;
  const decision = record.decisions[0];
  const question = questionOf(record, decision);
  const watches = record.origin === 'monitor' || record.origin === 'gate';
  if (watches && question?.threshold && decision.passProbability !== null) {
    const text = watchedProbability(question.threshold, question, decision.passProbability);
    if (text) return { p: decision.passProbability, text };
  }
  const p = answerProbability(decision);
  const key = answerKey(decision.answer);
  if (p === null || key === null) return null;
  const text = namedProbability(outcomeLabel(question, key), p);
  return text ? { p, text } : null;
}

/** The detail's muted line: `Monitor · 1:26:51 PM`, `Decide call · 1:37:24 PM`. */
function kindWord(record: DecisionRecord): string {
  if (record.origin === 'monitor') {
    return isSeveralMonitors(record) ? copy.kind.monitors : copy.kind.monitor;
  }
  return copy.kind[record.origin];
}

/** A place this long or shorter rides on a row; the `biorouter serve` phrase is left to Details. */
const ROW_PLACE_MAX = 16;

/**
 * The model a row names (R3): its alias, with where it runs when that is a short word
 * (`joey-1 · Baseten`, `tev1:0.8b · this computer`), or `null` without a model.
 */
export function rowModel(record: DecisionRecord): string | null {
  const ref = record.model ?? record.decisions[0]?.model ?? null;
  const alias = decisionModelLabel(ref);
  if (!ref || !alias) return null;
  const place = decisionModelPlace(ref.provider, { tier: record.tier });
  return place && place.length <= ROW_PLACE_MAX ? `${alias} · ${place}` : alias;
}

/**
 * Whether the rows came from more than one model (a vote, or monitors on different
 * models). Only then does a row name its model: it is what tells a vote's answers
 * apart, and a chat that asks one model keeps it under Details.
 */
export function listsSeveralModels(records: readonly DecisionRecord[]): boolean {
  let first: string | null = null;
  for (const record of records) {
    const model = rowModel(record);
    if (model === null) continue;
    if (first === null) first = model;
    else if (model !== first) return true;
  }
  return false;
}

/**
 * A row's accessible name: verdict or answer, title, the named probability or the names,
 * time, and the model when the list holds several.
 */
function rowLabel(record: DecisionRecord, showModel = false): string {
  const title = isSeveralMonitors(record)
    ? `${recordTitle(record)}: ${labelNames(record).join(', ')}`
    : recordTitle(record);
  return [
    recordVerdict(record),
    title,
    rowProbability(record)?.text ?? null,
    formatDecisionTime(record.createdAt),
    showModel ? rowModel(record) : null,
  ]
    .filter(Boolean)
    .join(', ');
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

function sameFilter(a: DecisionHistoryFilter, b: DecisionHistoryFilter): boolean {
  return JSON.stringify(normalizeFilter(a)) === JSON.stringify(normalizeFilter(b));
}

function normalizeFilter(filter: DecisionHistoryFilter): DecisionHistoryFilter {
  return {
    decisionId: filter.decisionId ?? null,
    origin: filter.origin ?? null,
    originId: filter.originId ?? null,
    turnId: filter.turnId ?? null,
    batchId: filter.batchId ?? null,
    outcome: filter.outcome ?? null,
    errorsOnly: filter.errorsOnly === true,
    minP: typeof filter.minP === 'number' ? filter.minP : null,
    maxP: typeof filter.maxP === 'number' ? filter.maxP : null,
  };
}

function filterIsSet(filter: DecisionHistoryFilter): boolean {
  return !sameFilter(filter, {});
}

/** The rows a window over `count` rows draws, by index. */
export function visibleRange(
  scrollTop: number,
  viewport: number,
  count: number,
  rowHeight = DECISION_ROW_HEIGHT,
  overscan = OVERSCAN
): { start: number; end: number } {
  if (count === 0) return { start: 0, end: 0 };
  const first = Math.floor(Math.max(0, scrollTop) / rowHeight);
  const last = Math.ceil((Math.max(0, scrollTop) + Math.max(viewport, rowHeight)) / rowHeight);
  return { start: Math.max(0, first - overscan), end: Math.min(count, last + overscan) };
}

// ---------------------------------------------------------------------------
// The list, loaded a page at a time
// ---------------------------------------------------------------------------

/** What the list's first page read: the follow tick compares a fresh read with it. */
interface ListHead {
  total: number;
  newestId: string | null;
}

interface ListState {
  key: string;
  records: DecisionRecord[];
  nextCursor: string | null;
  total: number;
  loading: boolean;
  error: boolean;
  /** At least one page has answered for this filter. */
  loaded: boolean;
  /** The first page's own total and newest call, kept while later pages load. */
  head: ListHead | null;
}

function emptyList(key: string): ListState {
  return {
    key,
    records: [],
    nextCursor: null,
    total: 0,
    loading: true,
    error: false,
    loaded: false,
    head: null,
  };
}

function useDecisionPages(sessionId: string, filter: DecisionHistoryFilter, reload: number) {
  const key = JSON.stringify([sessionId, normalizeFilter(filter), reload]);
  const [state, setState] = useState<ListState>(() => emptyList(key));
  const controllerRef = useRef<AbortController | null>(null);
  const inFlight = useRef(false);
  const stateRef = useRef(state);
  stateRef.current = state;

  const load = useCallback(
    async (cursor: string | null, forKey: string) => {
      if (inFlight.current && cursor !== null) return;
      inFlight.current = true;
      const controller = controllerRef.current ?? new AbortController();
      controllerRef.current = controller;
      setState((previous) =>
        previous.key === forKey ? { ...previous, loading: true, error: false } : previous
      );
      try {
        const page = await fetchDecisionPage(
          sessionId,
          filter,
          { before: cursor, limit: DECISION_PAGE_SIZE },
          controller.signal
        );
        if (controller.signal.aborted) return;
        setState((previous) => {
          if (previous.key !== forKey) return previous;
          const seen = new Set(previous.records.map((entry) => entry.id));
          const added = page.records.filter((entry) => !seen.has(entry.id));
          return {
            key: forKey,
            records: cursor === null ? page.records : [...previous.records, ...added],
            nextCursor: page.nextCursor,
            total: Math.max(
              page.total,
              (cursor === null ? 0 : previous.records.length) + added.length
            ),
            loading: false,
            error: false,
            loaded: true,
            head:
              cursor === null
                ? { total: page.total, newestId: page.records[0]?.id ?? null }
                : previous.head,
          };
        });
      } catch {
        if (controller.signal.aborted) return;
        setState((previous) =>
          previous.key === forKey ? { ...previous, loading: false, error: true } : previous
        );
      } finally {
        if (!controller.signal.aborted) inFlight.current = false;
      }
    },
    // `filter` is read through `key`, which changes whenever it does.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [sessionId, key]
  );

  useEffect(() => {
    controllerRef.current?.abort();
    controllerRef.current = new AbortController();
    inFlight.current = false;
    setState(emptyList(key));
    void load(null, key);
    return () => controllerRef.current?.abort();
  }, [key, load]);

  const loadMore = useCallback(() => {
    const current = stateRef.current;
    if (current.key !== key || current.loading || current.error || !current.nextCursor) return;
    void load(current.nextCursor, key);
  }, [key, load]);

  const retry = useCallback(() => {
    const current = stateRef.current;
    void load(current.records.length > 0 ? current.nextCursor : null, key);
  }, [key, load]);

  return { state: state.key === key ? state : emptyList(key), loadMore, retry };
}

// ---------------------------------------------------------------------------
// Filters (history spec 1.2): Source and Outcome up front, the rest behind Filters
// ---------------------------------------------------------------------------

function parseP(text: string): number | null | 'invalid' {
  const trimmed = text.trim();
  if (!trimmed) return null;
  const value = Number(trimmed);
  return Number.isFinite(value) && value >= 0 && value <= 1 ? value : 'invalid';
}

/** The chip's trailing chevron, turned while its surface is open. */
function Chevron({ open }: { open?: boolean }) {
  return (
    <ChevronDown
      aria-hidden="true"
      className="br-dh-facet__chevron"
      data-open={open || undefined}
    />
  );
}

interface MenuRow {
  value: string;
  label: string;
  hint?: string;
}

/**
 * A facet chip that opens a DropdownMenu with one radio group: the chosen row carries
 * the menu's trailing check. Radix menus are portalled DismissableLayer branches, so a
 * pick inside one nested in the Filters popover is not a press outside the popover, and
 * Escape closes only the topmost layer.
 */
function FacetMenu({
  label,
  name,
  engaged,
  value,
  rows,
  onPick,
  testId,
  wide,
}: {
  label: string;
  name?: string;
  engaged: boolean;
  value: string;
  rows: MenuRow[];
  onPick: (value: string) => void;
  testId: string;
  wide?: boolean;
}) {
  const [open, setOpen] = useState(false);
  const chipRef = useRef<HTMLButtonElement>(null);
  const escaped = useRef(false);
  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <DropdownMenuTrigger asChild>
        <FacetChip
          ref={chipRef}
          engaged={engaged}
          aria-label={name}
          className={wide ? 'br-dh-facet br-dh-facet--wide' : 'br-dh-facet'}
          data-testid={testId}
        >
          <span className="br-dh-facet__label">{label}</span>
          <Chevron open={open} />
        </FacetChip>
      </DropdownMenuTrigger>
      <DropdownMenuContent
        align="start"
        data-testid={`${testId}-menu`}
        onEscapeKeyDown={() => {
          escaped.current = true;
        }}
        // Escape always lands back on the chip, however the menu was opened: a menu opened
        // with the pointer never focused its trigger, and nested in the Filters popover the
        // default return could leave focus on the body, outside the popover still open.
        onCloseAutoFocus={(event) => {
          if (!escaped.current) return;
          escaped.current = false;
          event.preventDefault();
          chipRef.current?.focus();
        }}
      >
        <DropdownMenuRadioGroup value={value} onValueChange={onPick}>
          {rows.map((row) => (
            <DropdownMenuRadioItem key={row.value} value={row.value} hint={row.hint}>
              <span className="min-w-0 flex-1 truncate">{row.label}</span>
            </DropdownMenuRadioItem>
          ))}
        </DropdownMenuRadioGroup>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

const ANY = 'any';

/** The origins the chat has calls from, by the summary's aggregates. */
function presentOrigins(summary: SessionDecisionSummary | null): DecisionOrigin[] {
  return [...new Set((summary?.aggregates ?? []).map((aggregate) => aggregate.origin))];
}

function SourceFacet({
  filter,
  summary,
  onChange,
}: {
  filter: DecisionHistoryFilter;
  summary: SessionDecisionSummary | null;
  onChange: (next: DecisionHistoryFilter) => void;
}) {
  const options = sourceFilterOptionsFor(presentOrigins(summary), filter.origin ?? null);
  const current = options.find((option) => option.origin === (filter.origin ?? null));
  const engaged = !!filter.origin;
  return (
    <FacetMenu
      label={
        engaged && current ? FILTER_CHIP_COPY.engaged(copy.source, current.label) : copy.source
      }
      engaged={engaged}
      value={filter.origin ?? ANY}
      rows={options.map((option) => ({ value: option.origin ?? ANY, label: option.label }))}
      testId="decision-filter-source"
      onPick={(value) => {
        const origin = value === ANY ? null : (value as DecisionOrigin);
        if (origin === (filter.origin ?? null)) return;
        const keep = outcomeKeptFor(origin, filter);
        // A batch is one decide call, and the which-one pick belongs to the old source.
        onChange({
          ...filter,
          origin,
          originId: null,
          batchId: null,
          ...(keep ? {} : { outcome: null, errorsOnly: false }),
        });
      }}
    />
  );
}

const UNLISTED = 'unlisted';

function OutcomeFacet({
  filter,
  onChange,
}: {
  filter: DecisionHistoryFilter;
  onChange: (next: DecisionHistoryFilter) => void;
}) {
  const origin = filter.origin ?? null;
  const options = OUTCOME_FILTER_OPTIONS(origin);
  const choice = outcomeFilterChoice(origin, filter);
  const rows: MenuRow[] = options.map((option) => ({ value: option.key, label: option.label }));
  // An outcome a link brought that this source lists no row for: still said, still checked.
  if (!choice.listed) rows.push({ value: UNLISTED, label: choice.label });
  const value = choice.listed ? (choice.key ?? ANY) : UNLISTED;
  const engaged = value !== ANY;
  return (
    <FacetMenu
      label={engaged ? FILTER_CHIP_COPY.engaged(copy.outcome, choice.label) : copy.outcome}
      engaged={engaged}
      value={value}
      rows={rows}
      testId="decision-filter-outcome"
      onPick={(picked) => {
        const option = options.find((entry) => entry.key === picked);
        if (option) onChange({ ...filter, ...option.filter });
      }}
    />
  );
}

/** The nouns the Filters chip names, in field order: `monitor`, `turn`, `probability`. */
export function filtersSetNouns(filter: DecisionHistoryFilter): string[] {
  const nouns: string[] = [];
  if (filter.originId) nouns.push(filter.origin ? copy.which[filter.origin].set : 'source');
  if (filter.turnId) nouns.push('turn');
  if (filter.minP != null || filter.maxP != null) nouns.push('probability');
  return nouns;
}

function turnRows(summary: SessionDecisionSummary | null) {
  const turns = new Map<string, { at: string; count: number }>();
  for (const turn of summary?.recentTurns ?? []) {
    const entry = turns.get(turn.turnId) ?? { at: turn.firstAt, count: 0 };
    entry.count += turn.count;
    if (turn.firstAt && (!entry.at || Date.parse(turn.firstAt) < Date.parse(entry.at))) {
      entry.at = turn.firstAt;
    }
    turns.set(turn.turnId, entry);
  }
  return turns;
}

/**
 * What the Filters popover holds: which monitor, gate or decide call; which turn; the
 * probability range. Mounted only while the popover is open.
 */
function MoreFilters({
  filter,
  summary,
  onChange,
}: {
  filter: DecisionHistoryFilter;
  summary: SessionDecisionSummary | null;
  onChange: (next: DecisionHistoryFilter) => void;
}) {
  const base = useId();
  const [minText, setMinText] = useState(filter.minP != null ? String(filter.minP) : '');
  const [maxText, setMaxText] = useState(filter.maxP != null ? String(filter.maxP) : '');
  // A range applied from outside (Clear filters, a link) resets a field, unless the field
  // already reads that value: "0." typed on the way to "0.5" applies 0 and must stay "0.".
  const [seen, setSeen] = useState({ min: filter.minP ?? null, max: filter.maxP ?? null });
  if (seen.min !== (filter.minP ?? null) || seen.max !== (filter.maxP ?? null)) {
    setSeen({ min: filter.minP ?? null, max: filter.maxP ?? null });
    if (parseP(minText) !== (filter.minP ?? null)) {
      setMinText(filter.minP != null ? String(filter.minP) : '');
    }
    if (parseP(maxText) !== (filter.maxP ?? null)) {
      setMaxText(filter.maxP != null ? String(filter.maxP) : '');
    }
  }
  const min = parseP(minText);
  const max = parseP(maxText);
  const rangeInvalid =
    min === 'invalid' || max === 'invalid' || (min !== null && max !== null && min > max);

  // G-6: the range applies as it is typed, whenever both fields read a valid range.
  const applyRange = (nextMin: string, nextMax: string) => {
    const lo = parseP(nextMin);
    const hi = parseP(nextMax);
    if (lo === 'invalid' || hi === 'invalid' || (lo !== null && hi !== null && lo > hi)) return;
    if (lo === (filter.minP ?? null) && hi === (filter.maxP ?? null)) return;
    onChange({ ...filter, minP: lo, maxP: hi });
  };

  const origin = filter.origin ?? null;
  const which = origin ? copy.which[origin] : null;
  const criteria = (summary?.aggregates ?? []).filter(
    (aggregate) => aggregate.origin === origin && aggregate.originId
  );
  const showWhich = !!which && (criteria.length >= 2 || !!filter.originId);
  const whichRows: MenuRow[] = which
    ? [
        { value: ANY, label: which.any },
        ...criteria.map((aggregate) => ({
          value: aggregate.originId!,
          label:
            aggregate.origin === 'tool'
              ? formatShortTime(aggregate.lastAt) || which.noun
              : aggregate.label || which.noun,
          hint:
            aggregate.origin === 'tool'
              ? copy.chunkCount(aggregate.count)
              : copy.callCount(aggregate.count),
        })),
      ]
    : [];
  const pickedCriterion = criteria.find((aggregate) => aggregate.originId === filter.originId);
  if (filter.originId && !pickedCriterion) {
    whichRows.push({ value: filter.originId, label: copy.notListed });
  }
  const whichLabel = !which
    ? ''
    : !filter.originId
      ? which.noun
      : FILTER_CHIP_COPY.engaged(
          which.noun,
          whichRows.find((row) => row.value === filter.originId)?.label ?? copy.notListed
        );

  const turns = turnRows(summary);
  const turnOptions: MenuRow[] = [
    { value: ANY, label: copy.anyTurn },
    ...[...turns.entries()].map(([turnId, entry]) => ({
      value: turnId,
      label: formatShortTime(entry.at) || copy.earlierTurn,
      hint: copy.callCount(entry.count),
    })),
  ];
  if (filter.turnId && !turns.has(filter.turnId)) {
    turnOptions.push({ value: filter.turnId, label: copy.earlierTurn });
  }
  const turnLabel = filter.turnId
    ? FILTER_CHIP_COPY.engaged(
        copy.turn,
        turnOptions.find((row) => row.value === filter.turnId)?.label ?? copy.earlierTurn
      )
    : copy.turn;

  return (
    <div className="br-dh-more" data-testid="decision-filters-more">
      {showWhich && which ? (
        <FacetMenu
          wide
          label={whichLabel}
          engaged={!!filter.originId}
          value={filter.originId ?? ANY}
          rows={whichRows}
          testId="decision-filter-origin"
          onPick={(value) =>
            onChange({ ...filter, originId: value === ANY ? null : value, batchId: null })
          }
        />
      ) : null}
      <FacetMenu
        wide
        label={turnLabel}
        engaged={!!filter.turnId}
        value={filter.turnId ?? ANY}
        rows={turnOptions}
        testId="decision-filter-turn"
        onPick={(value) => onChange({ ...filter, turnId: value === ANY ? null : value })}
      />
      <div className="br-dh-range">
        <span className="br-dh-range__label" id={`${base}-range`}>
          {copy.pRange}
          <InfoTip label={copy.pRange} help={copy.pRangeHelp} />
        </span>
        <span className="br-dh-range__fields" role="group" aria-labelledby={`${base}-range`}>
          <Input
            aria-label={copy.minP}
            aria-describedby={rangeInvalid ? `${base}-range-error` : undefined}
            aria-invalid={rangeInvalid || undefined}
            inputMode="decimal"
            placeholder="0"
            value={minText}
            data-testid="decision-filter-min-p"
            onChange={(event) => {
              setMinText(event.target.value);
              applyRange(event.target.value, maxText);
            }}
            onKeyDown={(event) => {
              if (event.key === 'Enter') applyRange(minText, maxText);
            }}
          />
          <span aria-hidden="true">{copy.to}</span>
          <Input
            aria-label={copy.maxP}
            aria-describedby={rangeInvalid ? `${base}-range-error` : undefined}
            aria-invalid={rangeInvalid || undefined}
            inputMode="decimal"
            placeholder="1"
            value={maxText}
            data-testid="decision-filter-max-p"
            onChange={(event) => {
              setMaxText(event.target.value);
              applyRange(minText, event.target.value);
            }}
            onKeyDown={(event) => {
              if (event.key === 'Enter') applyRange(minText, maxText);
            }}
          />
        </span>
        {rangeInvalid ? (
          <p className="br-dh-range__error" id={`${base}-range-error`} role="alert">
            {copy.pRangeInvalid}
          </p>
        ) : null}
      </div>
    </div>
  );
}

function FiltersFacet({
  filter,
  summary,
  onChange,
}: {
  filter: DecisionHistoryFilter;
  summary: SessionDecisionSummary | null;
  onChange: (next: DecisionHistoryFilter) => void;
}) {
  const [open, setOpen] = useState(false);
  const nouns = filtersSetNouns(filter);
  return (
    <Popover open={open} onOpenChange={setOpen}>
      <PopoverTrigger asChild>
        <FacetChip
          engaged={nouns.length > 0}
          aria-label={nouns.length > 0 ? copy.filtersSetName(nouns) : copy.filters}
          // Kept while its popover is open (R8): `.br-surface-trigger` in main.css.
          className="br-dh-facet br-dh-facet--filters br-surface-trigger"
          data-testid="decision-filter-more"
        >
          <SlidersHorizontal aria-hidden="true" className="br-dh-facet__glyph" />
          <span className="br-dh-facet__label">
            {nouns.length > 0 ? copy.filtersSet(nouns) : copy.filters}
          </span>
          <Chevron open={open} />
        </FacetChip>
      </PopoverTrigger>
      <PopoverContent align="start" className="flex w-64 flex-col items-stretch gap-1 p-2">
        <MoreFilters filter={filter} summary={summary} onChange={onChange} />
      </PopoverContent>
    </Popover>
  );
}

/** A filter no chip names (a card's one batch or one call), engaged, with its own remove. */
function RemovableChip({
  label,
  name,
  onRemove,
  testId,
}: {
  label: string;
  name: string;
  onRemove: () => void;
  testId: string;
}) {
  return (
    <FacetChip
      engaged
      aria-label={name}
      className="br-dh-facet"
      data-testid={testId}
      onClick={onRemove}
    >
      <span className="br-dh-facet__label">{label}</span>
      <X aria-hidden="true" className="br-dh-facet__remove" />
    </FacetChip>
  );
}

function Filters({
  filter,
  summary,
  onChange,
  onClear,
}: {
  filter: DecisionHistoryFilter;
  summary: SessionDecisionSummary | null;
  onChange: (next: DecisionHistoryFilter) => void;
  onClear: () => void;
}) {
  return (
    <div
      className="br-dh__filters"
      role="group"
      aria-label={copy.filtersLabel}
      data-testid="decision-filters"
    >
      <SourceFacet filter={filter} summary={summary} onChange={onChange} />
      <OutcomeFacet filter={filter} onChange={onChange} />
      <FiltersFacet filter={filter} summary={summary} onChange={onChange} />
      {filter.batchId ? (
        <RemovableChip
          label={copy.batchOnly}
          name={copy.removeBatch}
          testId="decision-filter-batch"
          onRemove={() => onChange({ ...filter, batchId: null })}
        />
      ) : null}
      {filter.decisionId ? (
        <RemovableChip
          label={copy.callOnly}
          name={copy.removeCall}
          testId="decision-filter-call"
          onRemove={() => onChange({ ...filter, decisionId: null })}
        />
      ) : null}
      {filterIsSet(filter) ? (
        <Button
          type="button"
          variant="link"
          size="xs"
          className="br-decision-link br-dh__clear"
          data-testid="decision-filter-clear"
          onClick={onClear}
        >
          {copy.clear}
        </Button>
      ) : null}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Rows (R1: verdict words and glyph shapes; R2: the probability named)
// ---------------------------------------------------------------------------

function RecordGlyph({ record }: { record: DecisionRecord }) {
  const state = recordGlyphState(record);
  const Icon = RECORD_GLYPHS[state];
  return (
    <span
      className="br-dh-row__glyph"
      aria-hidden="true"
      data-tone={RECORD_GLYPH_TONES[state]}
      data-glyph={state}
    >
      <Icon />
    </span>
  );
}

function DecisionRow({
  record,
  index,
  total,
  selected,
  showModel,
  id,
  onSelect,
}: {
  record: DecisionRecord;
  index: number;
  total: number;
  selected: boolean;
  /** Name the model on line 2: the list holds calls from more than one. */
  showModel: boolean;
  id: string;
  onSelect: (id: string) => void;
}) {
  const probability = rowProbability(record);
  const model = showModel ? rowModel(record) : null;
  return (
    <div
      id={id}
      role="option"
      aria-selected={selected}
      aria-posinset={index + 1}
      aria-setsize={total}
      aria-label={rowLabel(record, showModel)}
      className="br-dh-row"
      data-outcome={record.outcome}
      data-selected={selected ? '' : undefined}
      data-testid="decision-row"
      data-decision-id={record.id}
      style={{ top: index * DECISION_ROW_HEIGHT }}
      onClick={() => onSelect(record.id)}
    >
      <RecordGlyph record={record} />
      <span className="br-dh-row__text" aria-hidden="true">
        <span className="br-dh-row__line">
          <span className="br-dh-row__title">{recordTitle(record)}</span>
          <span className="br-dh-row__answer" data-testid="decision-row-verdict">
            {recordVerdict(record)}
          </span>
        </span>
        <span className="br-dh-row__line br-dh-row__meta">
          <span className="br-dh-row__when">
            {model ? `${recordSubtitle(record)} · ${model}` : recordSubtitle(record)}
          </span>
          {probability ? (
            <span className="br-dh-row__p">
              <ProbabilityBar p={probability.p} />
              <span className="br-dh-row__p-value">{probability.text}</span>
            </span>
          ) : null}
        </span>
      </span>
    </div>
  );
}

// ---------------------------------------------------------------------------
// The detail pane
// ---------------------------------------------------------------------------

function ContextView({ context }: { context: DecisionContext }) {
  if (context.type === 'text') {
    return <p className="br-dh-context__prose">{context.text}</p>;
  }
  if (context.type === 'json') {
    // JSON is code: the mono layer.
    return (
      <pre className="br-dh-context__text">
        {typeof context.value === 'string' ? context.value : JSON.stringify(context.value, null, 2)}
      </pre>
    );
  }
  return (
    <ol className="br-dh-context__messages">
      {context.messages.map((message, index) => (
        <li key={index} className="br-dh-context__message">
          <span className="br-dh-context__role">{message.role}</span>
          <span className="br-dh-context__content">{message.content}</span>
        </li>
      ))}
    </ol>
  );
}

/** The option keys in the question's order, then any the decision named that it does not. */
function optionKeys(question: NamedQuestion | null, decision: Decision): string[] {
  const keys: string[] = [];
  if (question?.kind.type === 'binary') keys.push('yes', 'no');
  else if (question?.kind.type === 'choice') keys.push(...question.kind.options.map((o) => o.name));
  else if (question?.kind.type === 'score')
    keys.push(...question.kind.levels.map((_, i) => String(i)));
  for (const entry of decision.probabilities) if (!keys.includes(entry.key)) keys.push(entry.key);
  return keys;
}

/** One question's option bars, the threshold marked on the option it watches. */
function OptionBars({
  question,
  decision,
  heading,
}: {
  question: NamedQuestion | null;
  decision: Decision;
  heading: string | null;
}) {
  const threshold = question?.threshold ?? null;
  const chosen = answerKey(decision.answer);
  return (
    <div className="br-dh-options-group">
      {heading ? <p className="br-dh-options__heading">{heading}</p> : null}
      <ul className="br-dh-options" aria-label={heading ?? copy.probabilities}>
        {optionKeys(question, decision).map((key) => {
          const p = decision.probabilities.find((entry) => entry.key === key)?.p ?? 0;
          const mark = thresholdOnOption(threshold, key);
          const label = outcomeLabel(question, key);
          return (
            <li
              key={key}
              className="br-dh-option"
              data-chosen={key === chosen ? '' : undefined}
              data-testid="decision-option"
            >
              <span className="br-dh-option__label">{label}</span>
              <ProbabilityBar p={p} threshold={mark} className="br-pbar--wide" />
              <span className="br-dh-option__p">
                <span className="sr-only">{`${label}: `}</span>
                {formatProbability(p)}
                {mark !== null ? (
                  <span className="sr-only">{`, threshold ${formatProbability(mark)}`}</span>
                ) : null}
              </span>
            </li>
          );
        })}
      </ul>
    </div>
  );
}

interface QuestionEntry {
  question: NamedQuestion | null;
  decision: Decision | null;
}

/** The questions in their order, then decisions no listed question names (a newer daemon's). */
function questionEntries(record: DecisionRecord): QuestionEntry[] {
  const names = new Set(record.questions.map((question) => question.name));
  return [
    ...record.questions.map((question) => ({
      question,
      decision: record.decisions.find((decision) => decision.question === question.name) ?? null,
    })),
    ...record.decisions
      .filter((decision) => !names.has(decision.question))
      .map((decision) => ({ question: null, decision })),
  ];
}

/** The question in plain words: its text, else the monitor's or phase's name, else `Question 2`. */
function questionText(record: DecisionRecord, entry: QuestionEntry, index: number): string {
  const text = entry.question?.instructions?.trim();
  if (text) return text;
  const named =
    (record.origin === 'monitor' && !isSeveralMonitors(record)) || record.origin === 'gate';
  if (named && record.label?.trim()) return record.label.trim();
  return copy.question(index + 1);
}

function QuestionBlock({
  record,
  entry,
  index,
  several,
}: {
  record: DecisionRecord;
  entry: QuestionEntry;
  index: number;
  several: boolean;
}) {
  const { question, decision } = entry;
  const names = labelNames(record);
  // A several-question monitor or gate call: the block opens on whose question it is.
  const owner =
    several && record.origin === 'monitor' && names.length === record.questions.length
      ? (names[index] ?? null)
      : null;
  const verdict =
    several &&
    decision &&
    decision.passed !== null &&
    (record.origin === 'monitor' || record.origin === 'gate')
      ? verdictWord(record.origin, decision.passed ? 'passed' : 'failed')
      : null;
  const key = decision ? answerKey(decision.answer) : null;
  const answerP = decision ? answerProbability(decision) : null;
  const threshold = thresholdLine(question?.threshold, question, decision?.passProbability);
  return (
    <section className="br-dh-question" data-testid="decision-question">
      {owner || verdict ? (
        <p className="br-dh-question__owner">
          <span className="br-dh-question__owner-name">{owner}</span>
          {verdict ? <span className="br-dh-question__owner-verdict">{verdict}</span> : null}
        </p>
      ) : null}
      <p className="br-dh-question__text" data-testid="decision-question-text">
        {questionText(record, entry, index)}
      </p>
      {decision && decision.answer ? (
        <p className="br-dh-question__answer" data-testid="decision-question-answer">
          <strong className="font-semibold">{readableAnswer(decision, question)}</strong>
          {key !== null && answerP !== null
            ? ` · ${namedProbability(outcomeLabel(question, key), answerP)}`
            : ''}
        </p>
      ) : null}
      {threshold ? (
        <p className="br-dh-question__threshold" data-testid="decision-question-threshold">
          {threshold}
        </p>
      ) : null}
    </section>
  );
}

function Fact({ label, children }: { label: ReactNode; children: ReactNode }) {
  return (
    <div className="br-decision-fact">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  );
}

type DetailState =
  | { status: 'loading' }
  | { status: 'ready'; detail: DecisionDetail }
  | { status: 'missing' }
  | { status: 'error' };

export type DetailView = 'saw' | 'raw';

function DecisionDetailPane({
  record,
  detail,
  onRetry,
  detailsOpen,
  onDetailsOpenChange,
  view,
  onViewChange,
}: {
  record: DecisionRecord;
  detail: DetailState | undefined;
  onRetry: () => void;
  detailsOpen: boolean;
  onDetailsOpenChange: (open: boolean) => void;
  view: DetailView;
  onViewChange: (view: DetailView) => void;
}) {
  const shown = detail?.status === 'ready' ? detail.detail.record : record;
  const entries = questionEntries(shown);
  const several = entries.length > 1;
  const headVerdict =
    shown.origin === 'monitor' ||
    shown.origin === 'gate' ||
    shown.outcome === 'error' ||
    shown.outcome === 'refused'
      ? recordVerdict(shown)
      : null;
  const uncalibrated =
    shown.calibrated === false || shown.decisions.some((decision) => !decision.calibrated);
  const model = decisionModelDisplay(shown.model, shown.tier);
  const tokens =
    shown.inputTokens !== null || shown.outputTokens !== null
      ? `${formatCount(shown.inputTokens ?? 0)} in · ${formatCount(shown.outputTokens ?? 0)} out`
      : null;
  const certainties = shown.decisions
    .map((decision) => decision.confidence)
    .filter((value): value is number => value !== null);
  const calibrations = [
    ...new Set(
      shown.decisions.map((decision) => calibrationText(decision.source, decision.calibrated))
    ),
  ];
  const chunk =
    shown.origin === 'tool'
      ? shown.chunkIndex !== null
        ? copy.chunkNumber(shown.chunkIndex + 1)
        : shown.chunkId
      : null;
  const rawJson =
    detail?.status === 'ready'
      ? JSON.stringify(detail.detail.raw ?? detail.detail.record, null, 2)
      : JSON.stringify(shown, null, 2);

  return (
    <div className="br-dh-detail__body" data-testid="decision-detail" data-decision-id={shown.id}>
      <div className="br-dh-detail__head">
        <RecordGlyph record={shown} />
        <h3 className="br-dh-detail__title">{recordTitle(shown)}</h3>
        {headVerdict ? (
          <span className="br-dh-detail__outcome" data-testid="decision-detail-verdict">
            {headVerdict}
          </span>
        ) : null}
      </div>
      <p className="br-dh-detail__sub" data-testid="decision-detail-sub">
        {[kindWord(shown), formatDecisionTime(shown.createdAt)].filter(Boolean).join(' · ')}
      </p>
      {shown.error ? (
        <p className="br-dh-detail__error" role="note" data-testid="decision-detail-error">
          <AlertCircle aria-hidden="true" />
          <span>
            {copy.error}: {shown.error}
          </span>
        </p>
      ) : null}
      {!shown.privacyEnforced ? <p className="br-dh-detail__note">{copy.privacyOff}</p> : null}

      <div className="br-dh-questions">
        {entries.map((entry, index) => (
          <QuestionBlock
            key={entry.question?.name ?? entry.decision?.question ?? index}
            record={shown}
            entry={entry}
            index={index}
            several={several}
          />
        ))}
      </div>

      {uncalibrated && shown.decisions.length > 0 ? (
        <p className="br-dh-detail__note br-dh-detail__estimate" data-testid="decision-estimate">
          {CALIBRATION_COPY.estimate}
          <InfoTip label={copy.probabilities} help={CALIBRATION_COPY.estimateTip} />
        </p>
      ) : null}

      <Disclosure
        label={copy.details}
        open={detailsOpen}
        onOpenChange={onDetailsOpenChange}
        className="br-dh-details"
      >
        <div className="br-dh-details__body" data-testid="decision-details">
          {entries.map((entry, index) =>
            entry.decision ? (
              <OptionBars
                key={entry.question?.name ?? entry.decision.question}
                question={entry.question}
                decision={entry.decision}
                heading={several ? questionText(shown, entry, index) : null}
              />
            ) : null
          )}
          <dl className="br-decision-facts">
            {model ? <Fact label={copy.model}>{model}</Fact> : null}
            {shown.servedModel && shown.servedModel !== shown.model?.model ? (
              <Fact label={copy.servedBy}>{shown.servedModel}</Fact>
            ) : null}
            {shown.tier ? (
              <Fact label={copy.tier}>{shown.tier === 'private' ? copy.private : copy.public}</Fact>
            ) : null}
            {formatLatency(shown.latencyMs) ? (
              <Fact label={copy.latency}>{formatLatency(shown.latencyMs)}</Fact>
            ) : null}
            {tokens ? <Fact label={copy.tokens}>{tokens}</Fact> : null}
            {certainties.length > 0 ? (
              <Fact
                label={
                  <span className="br-decision-fact__label">
                    {copy.certainty}
                    <InfoTip label={copy.certainty} help={copy.certaintyHelp} />
                  </span>
                }
              >
                {certainties.map((value) => formatProbability(value)).join(' · ')}
              </Fact>
            ) : null}
            {calibrations.length > 0 ? (
              <Fact label={copy.probabilities}>{calibrations.join(' ')}</Fact>
            ) : null}
            {chunk ? <Fact label={copy.chunk}>{chunk}</Fact> : null}
          </dl>

          <div className="br-dh-saw">
            <div className="br-dh-saw__bar">
              <SegmentedControl
                aria-label={copy.show}
                value={view}
                onValueChange={(next) => onViewChange(next as DetailView)}
                options={[
                  { value: 'saw', label: copy.whatItSaw },
                  { value: 'raw', label: copy.raw },
                ]}
                data-testid="decision-detail-view"
              />
              {shown.contextBytes > 0 ? (
                <span className="br-dh-saw__size">{formatBytes(shown.contextBytes)}</span>
              ) : null}
            </div>
            {view === 'raw' ? (
              <div className="br-dh-context biorouter-focus-region" tabIndex={0}>
                <pre className="br-dh-context__text" data-testid="decision-raw">
                  {rawJson}
                </pre>
              </div>
            ) : (
              <>
                <div
                  className="br-dh-context biorouter-focus-region"
                  tabIndex={0}
                  data-testid="decision-context"
                >
                  {detail === undefined || detail.status === 'loading' ? (
                    <p className="br-dh-context__prose" data-preview="">
                      {shown.contextPreview}
                    </p>
                  ) : detail.status === 'ready' && detail.detail.context ? (
                    <ContextView context={detail.detail.context} />
                  ) : null}
                </div>
                {shown.contextTruncated ? (
                  <p className="br-dh-detail__note">
                    {copy.contextTruncated(formatBytes(shown.contextBytes))}
                  </p>
                ) : null}
                {detail === undefined || detail.status === 'loading' ? (
                  <p className="br-dh-detail__note" aria-live="polite">
                    {copy.contextLoading}
                  </p>
                ) : detail.status === 'error' ? (
                  <p className="br-dh-detail__note" role="alert">
                    {copy.detailFailed}{' '}
                    <Button
                      type="button"
                      variant="link"
                      size="xs"
                      className="br-decision-link"
                      onClick={onRetry}
                    >
                      {copy.retry}
                    </Button>
                  </p>
                ) : detail.status === 'missing' ? (
                  <p className="br-dh-detail__note">{copy.detailMissing}</p>
                ) : !detail.detail.context ? (
                  <p className="br-dh-detail__note">{copy.contextUnavailable}</p>
                ) : null}
              </>
            )}
          </div>
        </div>
      </Disclosure>
    </div>
  );
}

// ---------------------------------------------------------------------------
// The view
// ---------------------------------------------------------------------------

export interface DecisionHistoryProps {
  sessionId: string;
  /** The filter the view opens with (a summary row's source, a decide call's batch). */
  filter: DecisionHistoryFilter;
  /** Changes on every open, so the same filter applied again still applies. */
  filterKey: number;
}

/**
 * A chat's decision history: the ONE place individual decision-model calls are
 * listed (owner rule, decision calls can number in the thousands). Opened as a
 * tab of the artifact side panel (kind `decisions`), from the chat summary's
 * Decisions rows, a `decide` tool card or a turn's decision marker.
 *
 * One filter line (Source and Outcome up front; which-one, turn and the
 * probability range behind Filters), then a windowed list, newest first, that
 * asks for the next page as it nears the end, so a chat with thousands of calls
 * draws only the rows in view. A `decide` call's rows stay together in chunk
 * order, each showing the time its first row was stored (the daemon's
 * `PAGE_ORDER` in `decision/log.rs`). Each row says Fired or Quiet for a monitor,
 * Passed or Not passed for a gate, and the answer for every other call, with the
 * probability named (R1, R2); its second line names the model only when the
 * loaded rows came from more than one (`listsSeveralModels`, R3), and otherwise
 * the model stays under Details. Beside it (or under it, in a narrow panel) the
 * selected call: the question in plain words, the answer and its own
 * probability, the threshold line, and everything else (bars, model, tier,
 * latency, tokens, certainty, what it saw, raw JSON) under Details (R7). Reads
 * only, so it works the same in a `biorouter serve` browser.
 */
export default function DecisionHistory({
  sessionId,
  filter: initial,
  filterKey,
}: DecisionHistoryProps) {
  const [filter, setFilter] = useState<DecisionHistoryFilter>(() => normalizeFilter(initial));
  const [appliedKey, setAppliedKey] = useState(filterKey);
  const [reload, setReload] = useState(0);
  // Selection: the call a link named (a decision card's), else the newest row once
  // the list lands, kept while it stays listed.
  const [selectedId, setSelectedId] = useState<string | null>(initial.decisionId ?? null);
  // Details and the What it saw / Raw JSON choice hold for the tab's life, across rows.
  const [detailsOpen, setDetailsOpen] = useState(false);
  const [detailView, setDetailView] = useState<DetailView>('saw');
  if (appliedKey !== filterKey) {
    setAppliedKey(filterKey);
    setFilter(normalizeFilter(initial));
    if (initial.decisionId) setSelectedId(initial.decisionId);
  }

  const { state, loadMore, retry } = useDecisionPages(sessionId, filter, reload);
  const [summary, setSummary] = useState<SessionDecisionSummary | null>(null);
  useEffect(() => {
    const controller = new AbortController();
    fetchDecisionSummary(sessionId, controller.signal)
      .then(({ data }) => {
        if (!controller.signal.aborted) setSummary(data);
      })
      .catch(() => {});
    return () => controller.abort();
  }, [sessionId, reload, filterKey]);

  // Follow a running chat: monitors and decide batches keep adding calls while the
  // view is open. Every FOLLOW_INTERVAL_MS while the window is visible, one small read
  // of the list's own first call (limit 1, the same filter) is compared with what the
  // list's first page read: a change reloads the list in place when it is at the top,
  // and new calls are offered as "Show N new" when the person has scrolled into older
  // ones, so the rows under the reader never move. The comparison is with the list's
  // own read, never with a summary read beside it: a call that landed between those two
  // reads was counted by the summary and missing from the list, and the view kept
  // saying "2 calls" until Refresh (W3-7). It also counts only calls the filter shows.
  const [newCalls, setNewCalls] = useState(0);
  const atTopRef = useRef(true);
  const listedRef = useRef<{ key: string; filter: DecisionHistoryFilter; head: ListHead } | null>(
    null
  );
  listedRef.current =
    state.loaded && state.head ? { key: state.key, filter, head: state.head } : null;
  // A new filter or a reload starts counting again.
  useEffect(() => setNewCalls(0), [state.key]);
  useEffect(() => {
    let controller: AbortController | null = null;
    const tick = () => {
      if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return;
      const listed = listedRef.current;
      if (!listed) return;
      controller?.abort();
      const current = new AbortController();
      controller = current;
      fetchDecisionPage(sessionId, listed.filter, { before: null, limit: 1 }, current.signal)
        .then((page) => {
          if (current.signal.aborted || listedRef.current?.key !== listed.key) return;
          const added = page.total - listed.head.total;
          const changed = added !== 0 || (page.records[0]?.id ?? null) !== listed.head.newestId;
          if (!changed) return;
          if (atTopRef.current) {
            setNewCalls(0);
            setReload((value) => value + 1);
          } else if (added > 0) {
            setNewCalls(added);
          }
        })
        .catch(() => {});
    };
    const timer = setInterval(tick, FOLLOW_INTERVAL_MS);
    return () => {
      clearInterval(timer);
      controller?.abort();
    };
  }, [sessionId]);

  const records = state.records;
  const selectedIndex = selectedId ? records.findIndex((entry) => entry.id === selectedId) : -1;
  useEffect(() => {
    if (!state.loaded) return;
    if (selectedId && records.some((entry) => entry.id === selectedId)) return;
    setSelectedId(records[0]?.id ?? null);
  }, [state.loaded, records, selectedId]);

  // Details, fetched per selected call and kept for the view's life.
  const [details, setDetails] = useState<Record<string, DetailState>>({});
  const [detailRetry, setDetailRetry] = useState(0);
  useEffect(() => {
    if (!selectedId) return;
    // Only a settled answer is kept. A 'loading' entry here belongs to a fetch an
    // earlier selection started and then aborted when the person moved on, so
    // nothing will ever settle it: fetch again.
    const existing = details[selectedId];
    if (existing && (existing.status === 'ready' || existing.status === 'missing')) return;
    const controller = new AbortController();
    setDetails((current) => ({ ...current, [selectedId]: { status: 'loading' } }));
    fetchDecisionDetail(sessionId, selectedId, controller.signal)
      .then((detail) => {
        if (controller.signal.aborted) return;
        setDetails((current) => ({
          ...current,
          [selectedId]: detail ? { status: 'ready', detail } : { status: 'missing' },
        }));
      })
      .catch(() => {
        if (controller.signal.aborted) return;
        setDetails((current) => ({ ...current, [selectedId]: { status: 'error' } }));
      });
    return () => controller.abort();
    // `details` is read for its current entry only; a landed detail must not re-run this.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedId, sessionId, detailRetry]);

  // The window over the list.
  const listRef = useRef<HTMLDivElement | null>(null);
  const [viewport, setViewport] = useState({ top: 0, height: 600 });
  const frame = useRef(0);
  const measure = useCallback(() => {
    const element = listRef.current;
    if (!element) return;
    setViewport((current) =>
      current.top === element.scrollTop && current.height === element.clientHeight
        ? current
        : { top: element.scrollTop, height: element.clientHeight || current.height }
    );
  }, []);
  useLayoutEffect(() => {
    measure();
    const element = listRef.current;
    if (!element || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => measure());
    observer.observe(element);
    return () => observer.disconnect();
  }, [measure]);
  const hasFrames = typeof requestAnimationFrame === 'function';
  useEffect(
    () => () => {
      if (hasFrames) cancelAnimationFrame(frame.current);
    },
    [hasFrames]
  );
  // One measure per frame, however fast the wheel.
  const onScroll = () => {
    if (!hasFrames) return measure();
    cancelAnimationFrame(frame.current);
    frame.current = requestAnimationFrame(measure);
  };
  atTopRef.current = viewport.top < FOLLOW_TOP_SLACK_PX;
  const range = visibleRange(viewport.top, viewport.height, records.length);
  useEffect(() => {
    if (state.nextCursor && range.end >= records.length - LOAD_AHEAD_ROWS) loadMore();
  }, [range.end, records.length, state.nextCursor, loadMore]);
  // A new filter starts at the top.
  const listKey = state.key;
  useLayoutEffect(() => {
    if (listRef.current) listRef.current.scrollTop = 0;
    measure();
  }, [listKey, measure]);

  const baseId = useId();
  const optionId = (index: number) => `${baseId}-option-${index}`;
  const indexes: number[] = [];
  for (let index = range.start; index < range.end; index += 1) indexes.push(index);
  // The selected row stays drawn wherever the window is, so the list's active
  // descendant always names an element.
  if (selectedIndex >= 0 && (selectedIndex < range.start || selectedIndex >= range.end)) {
    indexes.push(selectedIndex);
  }

  const select = (id: string) => setSelectedId(id);
  const scrollIntoView = (index: number) => {
    const element = listRef.current;
    if (!element) return;
    const top = index * DECISION_ROW_HEIGHT;
    const bottom = top + DECISION_ROW_HEIGHT;
    if (top < element.scrollTop) element.scrollTop = top;
    else if (bottom > element.scrollTop + element.clientHeight) {
      element.scrollTop = bottom - element.clientHeight;
    }
    measure();
  };
  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (records.length === 0) return;
    const page = Math.max(
      1,
      Math.floor((listRef.current?.clientHeight ?? 400) / DECISION_ROW_HEIGHT) - 1
    );
    const from = selectedIndex < 0 ? -1 : selectedIndex;
    let next: number | null = null;
    if (event.key === 'ArrowDown') next = Math.min(records.length - 1, from + 1);
    else if (event.key === 'ArrowUp') next = Math.max(0, from - 1);
    else if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = records.length - 1;
    else if (event.key === 'PageDown') next = Math.min(records.length - 1, from + page);
    else if (event.key === 'PageUp') next = Math.max(0, from - page);
    if (next === null) return;
    event.preventDefault();
    setSelectedId(records[next].id);
    scrollIntoView(next);
  };

  const total = Math.max(state.total, records.length);
  const showModel = useMemo(() => listsSeveralModels(records), [records]);
  // The rows the narrow list hugs: those loaded, one more while a page waits, and one
  // for a state line ("Loading", "No decision calls match") when there are none.
  const listRows = records.length + (state.nextCursor ? 1 : 0);
  const filtered = filterIsSet(filter);
  const selected = selectedIndex >= 0 ? records[selectedIndex] : null;

  return (
    <div className="br-dh" data-testid="decision-history">
      <PreviewBarMeta>
        <span data-testid="decision-history-count">{state.loaded ? copy.calls(total) : ''}</span>
      </PreviewBarMeta>
      <PreviewBarActions>
        {newCalls > 0 && (
          <button
            type="button"
            className="br-dh-new-calls"
            data-testid="decision-history-new-calls"
            onClick={() => {
              setNewCalls(0);
              setReload((value) => value + 1);
            }}
          >
            {copy.showNew(newCalls)}
          </button>
        )}
        <PreviewTip content={copy.refresh}>
          <button
            type="button"
            className="br-dh-icon-button"
            aria-label={copy.refresh}
            data-testid="decision-history-refresh"
            onClick={() => setReload((value) => value + 1)}
          >
            <RefreshCw aria-hidden="true" />
          </button>
        </PreviewTip>
      </PreviewBarActions>
      <Filters
        filter={filter}
        summary={summary}
        // A filter the person sets is browsing: the one call a link named lets go.
        onChange={(next) => setFilter(normalizeFilter({ ...next, decisionId: null }))}
        onClear={() => setFilter(normalizeFilter({}))}
      />
      <div className="br-dh__split">
        <div
          className="br-dh__list-pane"
          // A-5: in a narrow panel the list hugs its rows (decisionsPanel.css).
          style={{ '--br-dh-rows': Math.max(1, listRows) } as CSSProperties}
        >
          <div
            ref={listRef}
            className="br-dh-list biorouter-focus-region"
            role="listbox"
            tabIndex={0}
            aria-label={copy.listLabel}
            aria-busy={state.loading || undefined}
            aria-activedescendant={selectedIndex >= 0 ? optionId(selectedIndex) : undefined}
            data-testid="decision-list"
            onScroll={onScroll}
            onKeyDown={onKeyDown}
          >
            <div
              className="br-dh-list__space"
              style={{
                height: (records.length + (state.nextCursor ? 1 : 0)) * DECISION_ROW_HEIGHT,
              }}
            >
              {indexes.map((index) => (
                <DecisionRow
                  key={records[index].id}
                  record={records[index]}
                  index={index}
                  total={total}
                  id={optionId(index)}
                  selected={index === selectedIndex}
                  showModel={showModel}
                  onSelect={select}
                />
              ))}
              {state.nextCursor ? (
                <div
                  className="br-dh-row br-dh-row--more"
                  aria-hidden="true"
                  style={{ top: records.length * DECISION_ROW_HEIGHT }}
                >
                  {copy.loadingMore}
                </div>
              ) : null}
            </div>
          </div>
          {!state.loaded && state.loading ? (
            <p className="br-dh__state" aria-live="polite">
              {copy.loading}
            </p>
          ) : null}
          {state.error ? (
            <p className="br-dh__state" role="alert">
              {copy.loadFailed}{' '}
              <Button
                type="button"
                variant="link"
                size="xs"
                className="br-decision-link"
                onClick={retry}
              >
                {copy.retry}
              </Button>
            </p>
          ) : null}
          {state.loaded && !state.error && records.length === 0 ? (
            <p className="br-dh__state">{filtered ? copy.emptyFiltered : copy.empty}</p>
          ) : null}
        </div>
        <div className="br-dh__detail" aria-live="off">
          {selected ? (
            <DecisionDetailPane
              record={selected}
              detail={details[selected.id]}
              detailsOpen={detailsOpen}
              onDetailsOpenChange={setDetailsOpen}
              view={detailView}
              onViewChange={setDetailView}
              onRetry={() => {
                setDetails((current) => {
                  const next = { ...current };
                  delete next[selected.id];
                  return next;
                });
                setDetailRetry((value) => value + 1);
              }}
            />
          ) : state.loaded && records.length > 0 ? (
            <p className="br-dh__state">{copy.noSelection}</p>
          ) : null}
        </div>
      </div>
    </div>
  );
}
