import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from 'react';
import { AlertTriangle, Check, CircleIcon, Lock, RefreshCw, X } from '../icons/app-icons';
import { Checkbox } from '../ui/Checkbox';
import { Disclosure } from '../ui/disclosure';
import { InfoTip } from '../ui/info-tip';
import { Input } from '../ui/input';
import { Select } from '../ui/Select';
import { PreviewBarActions, PreviewBarMeta, PreviewTip } from '../artifacts/PreviewBar';
import { formatCount } from '../../utils/usageStats';
import {
  DECISION_ORIGINS,
  DECISION_OUTCOMES,
  ORIGIN_FILTER_NAMES,
  OUTCOME_NAMES,
  answerProbability,
  calibrationText,
  decisionModelLabel,
  fetchDecisionDetail,
  fetchDecisionPage,
  fetchDecisionSummary,
  formatLatency,
  formatProbability,
  originLabel,
  outcomeLabel,
  readableAnswer,
  thresholdOnOption,
  thresholdSummary,
  type Decision,
  type DecisionContext,
  type DecisionDetail,
  type DecisionHistoryFilter,
  type DecisionOrigin,
  type DecisionOutcome,
  type DecisionRecord,
  type NamedQuestion,
  type SessionDecisionSummary,
} from '../../utils/sessionDecisions';
import { ProbabilityBar } from './ProbabilityBar';
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
  source: 'Source',
  anySource: 'Any source',
  origin: 'Criterion or call',
  anyOrigin: 'Any',
  turn: 'Turn',
  anyTurn: 'Any turn',
  outcome: 'Outcome',
  anyOutcome: 'Any outcome',
  errorsOnly: 'Errors only',
  minP: 'Min P',
  maxP: 'Max P',
  pRangeInvalid: 'Use numbers from 0 to 1, the lower one first.',
  moreFilters: 'More filters',
  batchOnly: 'This decide call only',
  removeBatch: 'Remove the filter: this decide call only',
  clear: 'Clear filters',
  refresh: 'Refresh',
  showNew: (n: number) => `Show ${formatCount(n)} new`,
  calls: (n: number) => `${formatCount(n)} ${n === 1 ? 'call' : 'calls'}`,
  loading: 'Loading decision calls',
  loadingMore: 'Loading more',
  loadFailed: 'Could not load decision calls.',
  retry: 'Retry',
  empty: 'This chat has made no decision calls yet.',
  emptyFiltered: 'No decision calls match these filters.',
  noSelection: 'Choose a call to see what it saw and how it answered.',
  detailFailed: 'Could not load this call.',
  detailMissing: 'This call is no longer in the log.',
  whatItSaw: 'What it saw',
  contextTruncated: (kb: string) => `Cut to the first 64 KB of ${kb}.`,
  contextLoading: 'Loading the context',
  contextUnavailable: 'The stored context is not available.',
  questions: 'Questions',
  threshold: 'Threshold',
  noThreshold: 'No threshold',
  passed: 'Passed',
  failed: 'Failed',
  answer: 'Answer',
  confidence: 'Concentration',
  confidenceHelp: 'How concentrated the distribution is (0 even, 1 certain). Not P(answer).',
  model: 'Model',
  /** The decision MODEL's tier, not the chat's: a public chat may use a local model. */
  tier: 'Model tier',
  latency: 'Latency',
  tokens: 'Tokens',
  chunk: 'Chunk',
  batch: 'Batch',
  turnId: 'Turn',
  error: 'Error',
  privacyOff: 'Privacy tiers were off when this call ran.',
  raw: 'Raw JSON',
  private: 'Private',
  public: 'Public',
  noAnswer: 'No answer',
};

const copy = DECISION_HISTORY_COPY;

/** How often an open history checks for new calls (one small summary read). */
export const FOLLOW_INTERVAL_MS = 4000;
/** Within this many pixels of the top, new calls reload the list in place. */
const FOLLOW_TOP_SLACK_PX = 24;

type Glyph = typeof CircleIcon;

/** A call's outcome, by SHAPE (never by colour alone). */
export const OUTCOME_ICONS: Record<DecisionOutcome, Glyph> = {
  passed: Check,
  failed: X,
  decided: CircleIcon,
  error: AlertTriangle,
  refused: Lock,
};

// ---------------------------------------------------------------------------
// Pure helpers
// ---------------------------------------------------------------------------

/** The probability a row's bar draws: the lowest pass probability, else P(first answer). */
export function recordProbability(record: DecisionRecord): number | null {
  const gated = record.decisions
    .map((decision) => decision.passProbability)
    .filter((p): p is number => p !== null);
  if (gated.length > 0) return Math.min(...gated);
  const first = record.decisions[0];
  return first ? answerProbability(first) : null;
}

/** What a row leads with: the chunk it read, else its origin. */
export function recordTitle(record: DecisionRecord): string {
  if (record.chunkId) return record.chunkId;
  if (record.chunkIndex !== null && record.origin === 'tool')
    return `Chunk ${record.chunkIndex + 1}`;
  return originLabel(record.origin, record.label);
}

/** The first question's answer, `+2` when more were asked. */
export function recordAnswer(record: DecisionRecord): string {
  if (record.outcome === 'error' || record.outcome === 'refused') {
    return OUTCOME_NAMES[record.outcome];
  }
  const first = record.decisions[0];
  if (!first) return copy.noAnswer;
  const question = record.questions.find((entry) => entry.name === first.question);
  const more = record.decisions.length - 1;
  return `${readableAnswer(first, question)}${more > 0 ? ` +${more}` : ''}`;
}

const TIME = new Intl.DateTimeFormat(undefined, {
  hour: 'numeric',
  minute: '2-digit',
  second: '2-digit',
});
const DAY_TIME = new Intl.DateTimeFormat(undefined, {
  month: 'short',
  day: 'numeric',
  hour: 'numeric',
  minute: '2-digit',
});

/** Seconds today (calls come fast), the day and minute before. */
export function formatDecisionTime(iso: string, now = new Date()): string {
  const date = new Date(iso);
  if (!Number.isFinite(date.getTime())) return '';
  return date.toDateString() === now.toDateString() ? TIME.format(date) : DAY_TIME.format(date);
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

interface ListState {
  key: string;
  records: DecisionRecord[];
  nextCursor: string | null;
  total: number;
  loading: boolean;
  error: boolean;
  /** At least one page has answered for this filter. */
  loaded: boolean;
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
// Filters
// ---------------------------------------------------------------------------

interface SelectOption<T> {
  value: T;
  label: string;
}

const menuPortalTarget = typeof document === 'undefined' ? undefined : document.body;

function FilterSelect<T extends string>({
  id,
  label,
  value,
  options,
  onChange,
  testId,
}: {
  id: string;
  label: string;
  value: T | null;
  options: SelectOption<T | null>[];
  onChange: (value: T | null) => void;
  testId: string;
}) {
  const selected = options.find((option) => option.value === value) ?? options[0];
  return (
    <div className="br-dh-filter" data-testid={testId}>
      <label htmlFor={id} className="br-dh-filter__label">
        {label}
      </label>
      <Select
        inputId={id}
        options={options}
        value={selected}
        isSearchable={false}
        menuPlacement="auto"
        menuPortalTarget={menuPortalTarget}
        onChange={(next: unknown) =>
          onChange((next as SelectOption<T | null> | null)?.value ?? null)
        }
      />
    </div>
  );
}

function parseP(text: string): number | null | 'invalid' {
  const trimmed = text.trim();
  if (!trimmed) return null;
  const value = Number(trimmed);
  return Number.isFinite(value) && value >= 0 && value <= 1 ? value : 'invalid';
}

function Filters({
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
  // A filter applied from outside (a summary row) resets the range fields.
  const applied = `${filter.minP ?? ''}|${filter.maxP ?? ''}`;
  const [seenApplied, setSeenApplied] = useState(applied);
  if (seenApplied !== applied) {
    setSeenApplied(applied);
    setMinText(filter.minP != null ? String(filter.minP) : '');
    setMaxText(filter.maxP != null ? String(filter.maxP) : '');
  }
  const min = parseP(minText);
  const max = parseP(maxText);
  const rangeInvalid =
    min === 'invalid' || max === 'invalid' || (min !== null && max !== null && min > max);

  const originOptions: SelectOption<DecisionOrigin | null>[] = [
    { value: null, label: copy.anySource },
    ...DECISION_ORIGINS.map((origin) => ({ value: origin, label: ORIGIN_FILTER_NAMES[origin] })),
  ];
  const outcomeOptions: SelectOption<DecisionOutcome | null>[] = [
    { value: null, label: copy.anyOutcome },
    ...DECISION_OUTCOMES.map((outcome) => ({ value: outcome, label: OUTCOME_NAMES[outcome] })),
  ];
  // The criteria of the chosen source the summary knows (a monitor by name, a
  // phase gate by its phase, a decide call by when it ran).
  const criteria = (summary?.aggregates ?? []).filter(
    (aggregate) => aggregate.origin === filter.origin && aggregate.originId
  );
  const criterionOptions: SelectOption<string | null>[] = [
    { value: null, label: copy.anyOrigin },
    ...criteria.map((aggregate) => ({
      value: aggregate.originId!,
      label:
        aggregate.origin === 'tool'
          ? `${originLabel('tool')} · ${formatDecisionTime(aggregate.lastAt)} (${formatCount(aggregate.count)})`
          : `${originLabel(aggregate.origin, aggregate.label)} (${formatCount(aggregate.count)})`,
    })),
  ];
  if (filter.originId && !criteria.some((aggregate) => aggregate.originId === filter.originId)) {
    criterionOptions.push({ value: filter.originId, label: filter.originId });
  }
  const turns = new Map<string, { at: string; count: number }>();
  for (const turn of summary?.recentTurns ?? []) {
    const entry = turns.get(turn.turnId) ?? { at: turn.firstAt, count: 0 };
    entry.count += turn.count;
    if (turn.firstAt && (!entry.at || Date.parse(turn.firstAt) < Date.parse(entry.at))) {
      entry.at = turn.firstAt;
    }
    turns.set(turn.turnId, entry);
  }
  const turnOptions: SelectOption<string | null>[] = [
    { value: null, label: copy.anyTurn },
    ...[...turns.entries()].map(([turnId, entry]) => ({
      value: turnId,
      label: `${formatDecisionTime(entry.at) || turnId} (${formatCount(entry.count)})`,
    })),
  ];
  if (filter.turnId && !turns.has(filter.turnId)) {
    turnOptions.push({ value: filter.turnId, label: filter.turnId });
  }

  const commitRange = (nextMin: string, nextMax: string) => {
    const lo = parseP(nextMin);
    const hi = parseP(nextMax);
    if (lo === 'invalid' || hi === 'invalid' || (lo !== null && hi !== null && lo > hi)) return;
    if (lo === (filter.minP ?? null) && hi === (filter.maxP ?? null)) return;
    onChange({ ...filter, minP: lo, maxP: hi });
  };

  const moreSet =
    !!filter.turnId || !!filter.errorsOnly || filter.minP != null || filter.maxP != null;

  return (
    <div className="br-dh-filters" role="group" aria-label={copy.filtersLabel}>
      <div className="br-dh-filters__row">
        <FilterSelect
          id={`${base}-source`}
          label={copy.source}
          value={filter.origin ?? null}
          options={originOptions}
          testId="decision-filter-source"
          // A batch is one decide call: another source cannot hold it.
          onChange={(origin) => onChange({ ...filter, origin, originId: null, batchId: null })}
        />
        {filter.origin && criterionOptions.length > 1 ? (
          <FilterSelect
            id={`${base}-origin`}
            label={copy.origin}
            value={filter.originId ?? null}
            options={criterionOptions}
            testId="decision-filter-origin"
            onChange={(originId) => onChange({ ...filter, originId, batchId: null })}
          />
        ) : null}
        {filter.batchId ? (
          // No select names a batch, so the filter a decide card opened with is said here,
          // where it can also be removed.
          <button
            type="button"
            className="br-dh-chip"
            aria-label={copy.removeBatch}
            data-testid="decision-filter-batch"
            onClick={() => onChange({ ...filter, batchId: null })}
          >
            <span>{copy.batchOnly}</span>
            <X aria-hidden="true" />
          </button>
        ) : null}
        <FilterSelect
          id={`${base}-outcome`}
          label={copy.outcome}
          value={filter.outcome ?? null}
          options={outcomeOptions}
          testId="decision-filter-outcome"
          onChange={(outcome) => onChange({ ...filter, outcome })}
        />
      </div>
      <Disclosure label={copy.moreFilters} defaultOpen={moreSet} className="br-dh-filters__more">
        <div className="br-dh-filters__row">
          <FilterSelect
            id={`${base}-turn`}
            label={copy.turn}
            value={filter.turnId ?? null}
            options={turnOptions}
            testId="decision-filter-turn"
            onChange={(turnId) => onChange({ ...filter, turnId })}
          />
          <div className="br-dh-filter br-dh-filter--range">
            <span className="br-dh-filter__label" id={`${base}-range`}>
              P
            </span>
            <div className="br-dh-range">
              <Input
                aria-label={copy.minP}
                aria-describedby={rangeInvalid ? `${base}-range-error` : undefined}
                aria-invalid={rangeInvalid || undefined}
                inputMode="decimal"
                placeholder="0"
                value={minText}
                data-testid="decision-filter-min-p"
                onChange={(event) => setMinText(event.target.value)}
                onBlur={() => commitRange(minText, maxText)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') commitRange(minText, maxText);
                }}
              />
              <span aria-hidden="true">to</span>
              <Input
                aria-label={copy.maxP}
                aria-describedby={rangeInvalid ? `${base}-range-error` : undefined}
                aria-invalid={rangeInvalid || undefined}
                inputMode="decimal"
                placeholder="1"
                value={maxText}
                data-testid="decision-filter-max-p"
                onChange={(event) => setMaxText(event.target.value)}
                onBlur={() => commitRange(minText, maxText)}
                onKeyDown={(event) => {
                  if (event.key === 'Enter') commitRange(minText, maxText);
                }}
              />
            </div>
          </div>
          <label className="br-dh-check">
            <Checkbox
              checked={filter.errorsOnly === true}
              data-testid="decision-filter-errors"
              onChange={(event) => onChange({ ...filter, errorsOnly: event.target.checked })}
            />
            <span>{copy.errorsOnly}</span>
          </label>
        </div>
        {rangeInvalid ? (
          <p className="br-dh-filters__error" id={`${base}-range-error`} role="alert">
            {copy.pRangeInvalid}
          </p>
        ) : null}
      </Disclosure>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Rows and the bar
// ---------------------------------------------------------------------------

function rowLabel(record: DecisionRecord, p: number | null): string {
  return [
    OUTCOME_NAMES[record.outcome],
    recordTitle(record),
    recordAnswer(record),
    p !== null ? `P ${formatProbability(p)}` : null,
    formatDecisionTime(record.createdAt),
    formatLatency(record.latencyMs),
  ]
    .filter(Boolean)
    .join(', ');
}

function DecisionRow({
  record,
  index,
  total,
  selected,
  id,
  onSelect,
}: {
  record: DecisionRecord;
  index: number;
  total: number;
  selected: boolean;
  id: string;
  onSelect: (id: string) => void;
}) {
  const Icon = OUTCOME_ICONS[record.outcome];
  const p = recordProbability(record);
  const model = decisionModelLabel(record.model);
  const latency = formatLatency(record.latencyMs);
  return (
    <div
      id={id}
      role="option"
      aria-selected={selected}
      aria-posinset={index + 1}
      aria-setsize={total}
      aria-label={rowLabel(record, p)}
      className="br-dh-row"
      data-outcome={record.outcome}
      data-selected={selected ? '' : undefined}
      data-testid="decision-row"
      data-decision-id={record.id}
      style={{ top: index * DECISION_ROW_HEIGHT }}
      onClick={() => onSelect(record.id)}
    >
      <span className="br-dh-row__glyph" aria-hidden="true">
        <Icon />
      </span>
      <span className="br-dh-row__text" aria-hidden="true">
        <span className="br-dh-row__line">
          <span className="br-dh-row__title">{recordTitle(record)}</span>
          <span className="br-dh-row__answer">{recordAnswer(record)}</span>
        </span>
        <span className="br-dh-row__line br-dh-row__meta">
          <span className="br-dh-row__when">
            {[formatDecisionTime(record.createdAt), model, latency].filter(Boolean).join(' · ')}
          </span>
          <span className="br-dh-row__p">
            <ProbabilityBar p={p} />
            <span className="br-dh-row__p-value">{formatProbability(p)}</span>
          </span>
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
    return <pre className="br-dh-context__text">{context.text}</pre>;
  }
  if (context.type === 'json') {
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

function QuestionBlock({
  question,
  decision,
}: {
  question: NamedQuestion | null;
  decision: Decision | null;
}) {
  const name = question?.name ?? decision?.question ?? '';
  const threshold = question?.threshold ?? null;
  // The options in the question's order, with any the decision named that the
  // question does not (a newer daemon's) after them.
  const keys: string[] = [];
  if (question?.kind.type === 'binary') keys.push('yes', 'no');
  else if (question?.kind.type === 'choice') keys.push(...question.kind.options.map((o) => o.name));
  else if (question?.kind.type === 'score')
    keys.push(...question.kind.levels.map((_, i) => String(i)));
  for (const entry of decision?.probabilities ?? [])
    if (!keys.includes(entry.key)) keys.push(entry.key);
  const chosen = decision ? (decision.answer ? readableAnswer(decision, question) : null) : null;
  const answerKeyName =
    decision?.answer?.type === 'binary'
      ? decision.answer.value
        ? 'yes'
        : 'no'
      : decision?.answer?.type === 'choice'
        ? decision.answer.option
        : decision?.answer?.type === 'score'
          ? String(decision.answer.level)
          : null;
  return (
    <section className="br-dh-question" data-testid="decision-question" data-question={name}>
      <div className="br-dh-question__head">
        <span className="br-dh-question__name">{name}</span>
        {decision?.passed === true ? (
          <span className="br-dh-chip" data-state="passed">
            <Check aria-hidden="true" />
            {copy.passed}
          </span>
        ) : decision?.passed === false ? (
          <span className="br-dh-chip" data-state="failed">
            <X aria-hidden="true" />
            {copy.failed}
          </span>
        ) : null}
      </div>
      {question?.instructions ? (
        <p className="br-dh-question__instructions">{question.instructions}</p>
      ) : null}
      <p className="br-dh-question__threshold">
        {threshold
          ? `${copy.threshold} ${thresholdSummary(threshold, question)}`
          : copy.noThreshold}
        {decision?.passProbability !== null && decision?.passProbability !== undefined
          ? ` · P ${formatProbability(decision.passProbability)}`
          : ''}
      </p>
      {chosen !== null ? (
        <p className="br-dh-question__answer">
          {copy.answer}: <strong className="font-semibold">{chosen}</strong>
          {decision && answerProbability(decision) !== null
            ? ` · P ${formatProbability(answerProbability(decision))}`
            : ''}
          {decision?.confidence !== null && decision?.confidence !== undefined ? (
            <>
              {` · ${copy.confidence} ${formatProbability(decision.confidence)}`}
              <InfoTip label={copy.confidence} help={copy.confidenceHelp} />
            </>
          ) : null}
        </p>
      ) : null}
      {decision ? (
        <ul className="br-dh-options" aria-label={`${name} options`}>
          {keys.map((key) => {
            const p = decision.probabilities.find((entry) => entry.key === key)?.p ?? 0;
            const mark = thresholdOnOption(threshold, key);
            const label = outcomeLabel(question, key);
            return (
              <li
                key={key}
                className="br-dh-option"
                data-chosen={key === answerKeyName ? '' : undefined}
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
      ) : null}
      {decision ? (
        <p
          className="br-dh-question__calibration"
          data-calibrated={decision.calibrated ? '' : undefined}
        >
          {calibrationText(decision.source, decision.calibrated)}
        </p>
      ) : null}
    </section>
  );
}

function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="br-dh-fact">
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

function DecisionDetailPane({
  record,
  detail,
  onRetry,
}: {
  record: DecisionRecord;
  detail: DetailState | undefined;
  onRetry: () => void;
}) {
  const shown = detail?.status === 'ready' ? detail.detail.record : record;
  const Icon = OUTCOME_ICONS[shown.outcome];
  const model = shown.model ? `${decisionModelLabel(shown.model)} (${shown.model.provider})` : null;
  const tokens =
    shown.inputTokens !== null || shown.outputTokens !== null
      ? `${formatCount(shown.inputTokens ?? 0)} in · ${formatCount(shown.outputTokens ?? 0)} out`
      : null;
  const questionNames = new Set(shown.questions.map((question) => question.name));
  const orphanDecisions = shown.decisions.filter(
    (decision) => !questionNames.has(decision.question)
  );
  return (
    <div className="br-dh-detail__body" data-testid="decision-detail" data-decision-id={shown.id}>
      <div className="br-dh-detail__head">
        <span className="br-dh-row__glyph" aria-hidden="true">
          <Icon />
        </span>
        <h3 className="br-dh-detail__title">{recordTitle(shown)}</h3>
        <span className="br-dh-detail__outcome" data-outcome={shown.outcome}>
          {OUTCOME_NAMES[shown.outcome]}
        </span>
      </div>
      <p className="br-dh-detail__sub">
        {[originLabel(shown.origin, shown.label), formatDecisionTime(shown.createdAt)]
          .filter(Boolean)
          .join(' · ')}
      </p>
      {shown.error ? (
        <p className="br-dh-detail__error" role="note">
          {copy.error}: {shown.error}
        </p>
      ) : null}
      {!shown.privacyEnforced ? <p className="br-dh-detail__note">{copy.privacyOff}</p> : null}

      <h4 className="br-dh-detail__section">{copy.questions}</h4>
      {shown.questions.map((question) => (
        <QuestionBlock
          key={question.name}
          question={question}
          decision={shown.decisions.find((decision) => decision.question === question.name) ?? null}
        />
      ))}
      {orphanDecisions.map((decision) => (
        <QuestionBlock key={decision.question} question={null} decision={decision} />
      ))}

      <dl className="br-dh-facts">
        {model ? <Fact label={copy.model}>{model}</Fact> : null}
        {shown.servedModel && shown.servedModel !== shown.model?.model ? (
          <Fact label="Served by">{shown.servedModel}</Fact>
        ) : null}
        {shown.tier ? (
          <Fact label={copy.tier}>{shown.tier === 'private' ? copy.private : copy.public}</Fact>
        ) : null}
        {formatLatency(shown.latencyMs) ? (
          <Fact label={copy.latency}>{formatLatency(shown.latencyMs)}</Fact>
        ) : null}
        {tokens ? <Fact label={copy.tokens}>{tokens}</Fact> : null}
        {shown.chunkId || shown.chunkIndex !== null ? (
          <Fact label={copy.chunk}>
            {[shown.chunkId, shown.chunkIndex !== null ? `#${shown.chunkIndex + 1}` : null]
              .filter(Boolean)
              .join(' ')}
          </Fact>
        ) : null}
        {shown.batchId ? <Fact label={copy.batch}>{shown.batchId}</Fact> : null}
        {shown.turnId ? <Fact label={copy.turnId}>{shown.turnId}</Fact> : null}
      </dl>

      <h4 className="br-dh-detail__section">
        {copy.whatItSaw}
        {shown.contextBytes > 0 ? (
          <span className="br-dh-detail__section-meta">{formatBytes(shown.contextBytes)}</span>
        ) : null}
      </h4>
      {shown.contextTruncated ? (
        <p className="br-dh-detail__note">
          {copy.contextTruncated(formatBytes(shown.contextBytes))}
        </p>
      ) : null}
      <div className="br-dh-context" data-testid="decision-context">
        {detail === undefined || detail.status === 'loading' ? (
          <>
            <pre className="br-dh-context__text" data-preview="">
              {shown.contextPreview}
            </pre>
            <p className="br-dh-detail__note" aria-live="polite">
              {copy.contextLoading}
            </p>
          </>
        ) : detail.status === 'ready' && detail.detail.context ? (
          <ContextView context={detail.detail.context} />
        ) : detail.status === 'error' ? (
          <p className="br-dh-detail__note" role="alert">
            {copy.detailFailed}{' '}
            <button type="button" className="br-dh-link" onClick={onRetry}>
              {copy.retry}
            </button>
          </p>
        ) : detail.status === 'missing' ? (
          <p className="br-dh-detail__note">{copy.detailMissing}</p>
        ) : (
          <p className="br-dh-detail__note">{copy.contextUnavailable}</p>
        )}
      </div>

      {detail?.status === 'ready' ? (
        <Disclosure label={copy.raw} className="br-dh-raw">
          <pre className="br-dh-context__text" data-testid="decision-raw">
            {JSON.stringify(detail.detail.raw ?? detail.detail.record, null, 2)}
          </pre>
        </Disclosure>
      ) : null}
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
 * Filters above (source, criterion, outcome; more: turn, probability range,
 * errors only), then a windowed list, newest first, that asks for the next page
 * as it nears the end, so a chat with thousands of calls draws only the rows in
 * view. Beside it (or under it, in a narrow panel) the selected call in full:
 * the exact context it was sent, its questions, each option's probability with
 * the threshold marked, whether it passed, whether its probabilities are
 * calibrated (said in words), the model and tier, latency and tokens, and the
 * raw JSON. Reads only, so it works the same in a `biorouter serve` browser.
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
  // view is open. A cheap summary read every FOLLOW_INTERVAL_MS while the window is
  // visible; new calls reload the list in place when it is at the top, and are
  // offered as "Show N new" when the person has scrolled into older calls, so the
  // rows under the reader never move.
  const seenTotal = useRef<number | null>(null);
  const [newCalls, setNewCalls] = useState(0);
  const atTopRef = useRef(true);
  useEffect(() => {
    if (summary) seenTotal.current = summary.total;
  }, [summary]);
  useEffect(() => {
    let controller: AbortController | null = null;
    const tick = () => {
      if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return;
      controller?.abort();
      const current = new AbortController();
      controller = current;
      fetchDecisionSummary(sessionId, current.signal)
        .then(({ data }) => {
          if (current.signal.aborted || !data || seenTotal.current === null) return;
          const added = data.total - seenTotal.current;
          if (added <= 0) return;
          if (atTopRef.current) {
            setNewCalls(0);
            setReload((value) => value + 1);
          } else {
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
      <div className="br-dh__filters">
        {/* Keyed by the open, so a filter applied from outside also opens More
            filters when it set one of them. */}
        <Filters
          key={appliedKey}
          filter={filter}
          summary={summary}
          // A filter the person sets is browsing: the one call a link named lets go.
          onChange={(next) => setFilter(normalizeFilter({ ...next, decisionId: null }))}
        />
        {filtered ? (
          <button
            type="button"
            className="br-dh-link br-dh__clear"
            data-testid="decision-filter-clear"
            onClick={() => setFilter(normalizeFilter({}))}
          >
            {copy.clear}
          </button>
        ) : null}
      </div>
      <div className="br-dh__split">
        <div className="br-dh__list-pane">
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
              <button type="button" className="br-dh-link" onClick={retry}>
                {copy.retry}
              </button>
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
