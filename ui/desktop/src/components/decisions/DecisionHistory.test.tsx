import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  parseDecisionDetail,
  parseDecisionRecord,
  type DecisionHistoryFilter,
  type DecisionPage,
} from '../../utils/sessionDecisions';
import { PreviewBar, PreviewBarScope } from '../artifacts/PreviewBar';
import DecisionHistory, {
  DECISION_HISTORY_COPY,
  DECISION_PAGE_SIZE,
  DECISION_ROW_HEIGHT,
  FOLLOW_INTERVAL_MS,
  filtersSetNouns,
  listsSeveralModels,
  rowModel,
  rowProbability,
  visibleRange,
} from './DecisionHistory';
import { CALIBRATION_COPY } from './decisionWords';
import { renderToStaticMarkup } from 'react-dom/server';
import { AlertCircle } from '../icons/app-icons';

const mocks = vi.hoisted(() => ({ page: vi.fn(), detail: vi.fn(), summary: vi.fn() }));
vi.mock('../../utils/sessionDecisions', async (original) => ({
  ...(await original<typeof import('../../utils/sessionDecisions')>()),
  fetchDecisionPage: mocks.page,
  fetchDecisionDetail: mocks.detail,
  fetchDecisionSummary: mocks.summary,
}));

const TOTAL = 5000;

function wire(index: number, overrides: Record<string, unknown> = {}) {
  return {
    id: `d${index}`,
    sessionId: 's1',
    createdAt: new Date(Date.UTC(2026, 10, 4, 10, 0, 0) - index * 1000).toISOString(),
    origin: 'tool',
    originId: 'call-1',
    label: 'decide',
    turnId: 't1',
    batchId: 'b1',
    chunkIndex: index,
    chunkId: `note-${String(index + 1).padStart(4, '0')}`,
    model: { provider: 'ollama_decision', model: 'tev1:0.8b' },
    servedModel: null,
    tier: 'private',
    calibrated: true,
    questions: {
      has_ms: {
        instructions: 'Does this note state the patient has multiple sclerosis?',
        kind: { binary: {} },
        threshold: { yes: 0.9 },
      },
    },
    decisions: [
      {
        question: 'has_ms',
        answer: { type: 'binary', value: index % 3 === 0 },
        probabilities: { yes: index % 3 === 0 ? 0.95 : 0.2, no: index % 3 === 0 ? 0.05 : 0.8 },
        confidence: 0.6,
        passed: index % 3 === 0,
        passProbability: index % 3 === 0 ? 0.95 : 0.2,
        source: { kind: 'native' },
        calibrated: true,
        model: { provider: 'ollama_decision', model: 'tev1:0.8b' },
        tier: 'private',
        requestId: `r${index}`,
        latencyMs: 22,
        usage: { inputTokens: 400, outputTokens: 1 },
        raw: {},
      },
    ],
    outcome: index % 3 === 0 ? 'passed' : 'failed',
    contextPreview: `Neuro f/u ${index}`,
    contextBytes: 2048,
    contextTruncated: false,
    error: null,
    latencyMs: 22,
    inputTokens: 400,
    outputTokens: 1,
    privacyEnforced: true,
    ...overrides,
  };
}

/** The daemon's pages over TOTAL rows, newest first, cursor = the next index. */
function pageAt(before: string | null | undefined): DecisionPage {
  const start = before ? Number(before) : 0;
  const end = Math.min(TOTAL, start + DECISION_PAGE_SIZE);
  return {
    records: Array.from({ length: end - start }, (_, i) => parseDecisionRecord(wire(start + i))!),
    nextCursor: end < TOTAL ? String(end) : null,
    total: TOTAL,
  };
}

function draw(filter: DecisionHistoryFilter = {}, filterKey = 1) {
  return render(<DecisionHistory sessionId="s1" filter={filter} filterKey={filterKey} />);
}

/** jsdom lays nothing out: give the list a viewport of ten rows. */
function sizeList(list: HTMLElement, height = DECISION_ROW_HEIGHT * 10) {
  Object.defineProperty(list, 'clientHeight', { configurable: true, value: height });
}

beforeEach(() => {
  mocks.page.mockReset().mockImplementation(async (_s, _f, page) => pageAt(page?.before));
  mocks.summary.mockReset().mockResolvedValue({
    data: { total: TOTAL, aggregates: [], recentTurns: [] },
    missing: false,
  });
  mocks.detail.mockReset().mockImplementation(async (_s: string, id: string) =>
    parseDecisionDetail({
      record: wire(Number(id.slice(1))),
      context: { text: `Full note ${id}: 34F, RRMS dx 2019, EDSS 2.0` },
    })
  );
});

/** A monitor's call: one yes or no question, fired when P(yes) reaches 0.5. */
function monitorWire(id: string, p: number, overrides: Record<string, unknown> = {}) {
  const fired = p >= 0.5;
  return {
    ...wire(0),
    id,
    origin: 'monitor',
    originId: 'no-haiku',
    label: 'No haiku',
    batchId: null,
    chunkIndex: null,
    chunkId: null,
    questions: {
      no_haiku: {
        instructions: 'Has the agent written a haiku?',
        kind: { binary: {} },
        threshold: { yes: 0.5 },
      },
    },
    decisions: [
      {
        ...wire(0).decisions[0],
        question: 'no_haiku',
        answer: { type: 'binary', value: fired },
        probabilities: { yes: p, no: 1 - p },
        passed: fired,
        passProbability: p,
      },
    ],
    outcome: fired ? 'passed' : 'failed',
    ...overrides,
  };
}

/** One call that asked two monitors at once: the daemon joins their names with ` + `. */
function severalMonitorsWire() {
  const base = monitorWire('m2', 0.9);
  return {
    ...base,
    originId: 'shell-seen',
    label: 'Shell seen + Needs a rest',
    questions: {
      shell_seen: {
        instructions: 'Has the agent run a shell command?',
        kind: { binary: {} },
        threshold: { yes: 0.5 },
      },
      needs_rest: {
        instructions: 'Is the agent repeating itself?',
        kind: { binary: {} },
        threshold: { yes: 0.5 },
      },
    },
    decisions: [
      { ...base.decisions[0], question: 'shell_seen', passed: true, passProbability: 0.9 },
      {
        ...base.decisions[0],
        question: 'needs_rest',
        answer: { type: 'binary', value: false },
        probabilities: { yes: 0.1, no: 0.9 },
        passed: false,
        passProbability: 0.1,
      },
    ],
    outcome: 'passed',
  };
}

function gateWire(id: string, p: number) {
  const base = monitorWire(id, p);
  return {
    ...base,
    origin: 'gate',
    originId: 'probe',
    label: 'Probe write test',
    questions: {
      mars: {
        instructions: 'Did the subagent report launching a rocket to Mars?',
        kind: { binary: {} },
        threshold: { yes: 0.9 },
      },
    },
    decisions: [{ ...base.decisions[0], question: 'mars', passed: p >= 0.9 }],
    outcome: p >= 0.9 ? 'passed' : 'failed',
  };
}

/** The list answers `rows` as one page, and each detail by its record. */
function serve(rows: Array<Record<string, unknown>>) {
  mocks.page.mockImplementation(async () => ({
    records: rows.map((row) => parseDecisionRecord(row)!),
    nextCursor: null,
    total: rows.length,
  }));
  mocks.detail.mockImplementation(async (_s: string, id: string) => {
    const row = rows.find((entry) => entry.id === id);
    return row ? parseDecisionDetail({ record: row, context: { text: `Context of ${id}` } }) : null;
  });
}

function rowOf(id: string) {
  return screen
    .getAllByTestId('decision-row')
    .find((row) => row.getAttribute('data-decision-id') === id)!;
}

async function openDetails() {
  const detail = await screen.findByTestId('decision-detail');
  fireEvent.click(within(detail).getByRole('button', { name: DECISION_HISTORY_COPY.details }));
  return screen.findByTestId('decision-details');
}

describe('the window over the list', () => {
  it('draws only the rows in view, plus a margin', () => {
    expect(visibleRange(0, 440, 5000)).toEqual({ start: 0, end: 18 });
    expect(visibleRange(44 * 1000, 440, 5000)).toEqual({ start: 992, end: 1018 });
    expect(visibleRange(44 * 4999, 440, 5000).end).toBe(5000);
    expect(visibleRange(0, 440, 0)).toEqual({ start: 0, end: 0 });
  });
});

describe('the probability a row names (R2)', () => {
  it('is the watched outcome for a monitor or a gate, named by its threshold', () => {
    expect(rowProbability(parseDecisionRecord(monitorWire('m', 0.01))!)).toEqual({
      p: 0.01,
      text: 'P(yes) 0.01',
    });
    expect(rowProbability(parseDecisionRecord(gateWire('g', 0.12))!)).toEqual({
      p: 0.12,
      text: 'P(yes) 0.12',
    });
  });

  it('is the answer’s own for a decide call: P(no) beside a no, never P(yes)', () => {
    // wire(1) answers no with P(no) 0.80 (P(yes) 0.20), outcome failed.
    expect(rowProbability(parseDecisionRecord(wire(1))!)).toEqual({ p: 0.8, text: 'P(no) 0.80' });
    expect(rowProbability(parseDecisionRecord(wire(0))!)).toEqual({
      p: 0.95,
      text: 'P(yes) 0.95',
    });
  });

  it('is nothing for a call that asked several questions', () => {
    expect(rowProbability(parseDecisionRecord(severalMonitorsWire())!)).toBeNull();
  });
});

describe('the rows (R1)', () => {
  it('says Fired or Quiet for a monitor, by shape too, and never Failed or a cross', async () => {
    serve([monitorWire('m1', 0.98), monitorWire('m0', 0.01)]);
    draw();
    await screen.findAllByTestId('decision-row');
    const fired = rowOf('m1');
    const quiet = rowOf('m0');
    expect(within(fired).getByTestId('decision-row-verdict')).toHaveTextContent('Fired');
    expect(fired.querySelector('[data-glyph]')).toHaveAttribute('data-glyph', 'monitorFired');
    expect(within(quiet).getByTestId('decision-row-verdict')).toHaveTextContent('Quiet');
    expect(quiet.querySelector('[data-glyph]')).toHaveAttribute('data-glyph', 'monitorQuiet');
    expect(quiet).toHaveAttribute(
      'aria-label',
      expect.stringMatching(/^Quiet, No haiku, P\(yes\) 0\.01, /)
    );
    expect(quiet).toHaveTextContent('P(yes) 0.01');
    for (const row of [fired, quiet]) expect(row).not.toHaveTextContent('Failed');
    // The model and the latency left the rows (they are under Details).
    expect(quiet).not.toHaveTextContent('tev1');
    expect(quiet).not.toHaveTextContent('22 ms');
  });

  it('titles a call that asked several monitors "2 monitors", the names on line 2, never with +', async () => {
    serve([severalMonitorsWire()]);
    draw();
    const row = await screen.findByTestId('decision-row');
    expect(row).toHaveTextContent('2 monitors');
    expect(row).toHaveTextContent('Shell seen, Needs a rest');
    expect(row).not.toHaveTextContent('+');
    expect(within(row).getByTestId('decision-row-verdict')).toHaveTextContent('1 of 2 fired');
    // Several questions: no bar and no number.
    expect(row.querySelector('.br-pbar')).toBeNull();
    expect(row).toHaveAttribute(
      'aria-label',
      expect.stringMatching(/^1 of 2 fired, 2 monitors: Shell seen, Needs a rest, /)
    );
  });

  it('says Passed or Not passed for a gate', async () => {
    serve([gateWire('g1', 0.95), gateWire('g0', 0.03)]);
    draw();
    await screen.findAllByTestId('decision-row');
    expect(within(rowOf('g1')).getByTestId('decision-row-verdict')).toHaveTextContent('Passed');
    expect(within(rowOf('g0')).getByTestId('decision-row-verdict')).toHaveTextContent('Not passed');
    expect(rowOf('g0').querySelector('[data-glyph]')).toHaveAttribute(
      'data-glyph',
      'gateNotPassed'
    );
    expect(rowOf('g0')).toHaveTextContent('Probe write test');
  });

  it('shows a decide call’s answer, not a verdict: a no that missed its threshold is "No"', async () => {
    serve([wire(1)]);
    draw();
    const row = await screen.findByTestId('decision-row');
    expect(within(row).getByTestId('decision-row-verdict')).toHaveTextContent(/^No$/);
    expect(row.querySelector('[data-glyph]')).toHaveAttribute('data-glyph', 'answer');
    expect(row).toHaveTextContent('note-0002');
    expect(row).toHaveTextContent('P(no) 0.80');
    expect(row).not.toHaveTextContent('Failed');
    expect(row).not.toHaveTextContent('P(yes)');
  });
});

describe('the model a row names (W2-6)', () => {
  const JOEY = 'decision-models/baseten-joey-2b-latest';
  /** One chunk of a vote, as one of its models answered it. */
  function voteWire(id: string, model: { provider: string; model: string }, tier: string) {
    const row = wire(2, { id, model, tier });
    return { ...row, decisions: row.decisions.map((d) => ({ ...d, model, tier })) };
  }
  const vote = () => [
    voteWire('v-tev', { provider: 'ollama_decision', model: 'tev1:0.8b' }, 'private'),
    voteWire('v-joey', { provider: 'baseten_decision', model: JOEY }, 'public'),
    voteWire('v-clef', { provider: 'ollama_decision', model: 'clef-flash:latest' }, 'private'),
  ];

  afterEach(() => {
    delete document.documentElement.dataset.biorouterSurface;
  });

  it('is the alias and a short place, never a slug or a provider key', () => {
    const [tev, joey] = vote().map((row) => parseDecisionRecord(row)!);
    expect(rowModel(tev)).toBe('tev1:0.8b · this computer');
    expect(rowModel(joey)).toBe('joey-1 · Baseten');
    // A `biorouter serve` browser's place is a phrase: the row keeps the alias alone.
    document.documentElement.dataset.biorouterSurface = 'headless';
    expect(rowModel(tev)).toBe('tev1:0.8b');
    expect(rowModel(parseDecisionRecord(wire(0, { model: null }))!)).toBe('tev1:0.8b');
  });

  it('is drawn only when the list holds calls from more than one model', () => {
    const one = [wire(0), wire(1)].map((row) => parseDecisionRecord(row)!);
    expect(listsSeveralModels(one)).toBe(false);
    expect(listsSeveralModels(vote().map((row) => parseDecisionRecord(row)!))).toBe(true);
  });

  it('tells a vote’s models apart on line 2 and in each row’s name', async () => {
    serve(vote());
    draw();
    await screen.findAllByTestId('decision-row');
    for (const [id, model] of [
      ['v-tev', 'tev1:0.8b · this computer'],
      ['v-joey', 'joey-1 · Baseten'],
      ['v-clef', 'clef-flash:latest · this computer'],
    ]) {
      const row = rowOf(id);
      expect(row.querySelector('.br-dh-row__when')).toHaveTextContent(model);
      expect(row).toHaveAttribute('aria-label', expect.stringMatching(new RegExp(`, ${model}$`)));
    }
    expect(rowOf('v-joey')).not.toHaveTextContent('baseten');
  });
});

describe('DecisionHistory', () => {
  it('lists the newest page, drawing only the rows in view of thousands', async () => {
    draw();
    await waitFor(() => expect(screen.getAllByTestId('decision-row').length).toBeGreaterThan(0));
    const list = screen.getByTestId('decision-list');
    expect(list).toHaveAttribute('role', 'listbox');
    // 100 rows loaded, only a window of them drawn.
    const drawn = within(list).getAllByTestId('decision-row');
    expect(drawn.length).toBeLessThan(40);
    expect(drawn[0]).toHaveAttribute('aria-setsize', String(TOTAL));
    expect(drawn[0]).toHaveTextContent('note-0001');
    expect(mocks.page).toHaveBeenCalledTimes(1);
    expect(mocks.page.mock.calls[0][2]).toMatchObject({ before: null, limit: DECISION_PAGE_SIZE });
  });

  it('asks for the next page as the list nears its end, and keeps paging', async () => {
    draw();
    await waitFor(() => expect(screen.getAllByTestId('decision-row').length).toBeGreaterThan(0));
    const list = screen.getByTestId('decision-list');
    sizeList(list);
    act(() => {
      list.scrollTop = DECISION_ROW_HEIGHT * 85;
      fireEvent.scroll(list);
    });
    await waitFor(() => expect(mocks.page).toHaveBeenCalledTimes(2));
    expect(mocks.page.mock.calls[1][2]).toMatchObject({ before: '100' });
    await waitFor(() =>
      expect(
        within(list)
          .getAllByTestId('decision-row')
          .some((row) => row.textContent?.includes('note-0101'))
      ).toBe(true)
    );
    act(() => {
      list.scrollTop = DECISION_ROW_HEIGHT * 185;
      fireEvent.scroll(list);
    });
    await waitFor(() => expect(mocks.page).toHaveBeenCalledTimes(3));
    expect(mocks.page.mock.calls[2][2]).toMatchObject({ before: '200' });
    // Still a window, whatever has loaded.
    expect(within(list).getAllByTestId('decision-row').length).toBeLessThan(40);
  });

  it('shows the question in plain words, the answer with its own probability and the threshold up front', async () => {
    draw();
    const detail = await screen.findByTestId('decision-detail');
    expect(detail).toHaveAttribute('data-decision-id', 'd0');
    // The title once, and a muted line with the kind and the time, never the title again.
    expect(within(detail).getAllByText('note-0001')).toHaveLength(1);
    expect(within(detail).getByTestId('decision-detail-sub')).toHaveTextContent(/^Decide call · /);
    const question = within(detail).getByTestId('decision-question');
    expect(within(question).getByTestId('decision-question-text')).toHaveTextContent(
      'Does this note state the patient has multiple sclerosis?'
    );
    expect(within(question).getByTestId('decision-question-answer')).toHaveTextContent(
      'Yes · P(yes) 0.95'
    );
    expect(within(question).getByTestId('decision-question-threshold')).toHaveTextContent(
      'P(yes) 0.95 · threshold 0.90'
    );
    // Never the question's name, never mono, never the old Passed/Failed chip or "≥".
    expect(detail).not.toHaveTextContent('has_ms');
    expect(detail.querySelector('.br-dh-question__name')).toBeNull();
    expect(detail.querySelector('.br-dh-chip')).toBeNull();
    expect(detail).not.toHaveTextContent('≥');
    // An answer-type call shows no verdict on its head line: the answer is item 4.
    expect(within(detail).queryByTestId('decision-detail-verdict')).toBeNull();
  });

  it('says why a call failed in the primary ink with the failure glyph, never in red', async () => {
    const failed = wire(0, {
      outcome: 'error',
      decisions: [],
      error: 'The decision model did not answer in 10 s',
    });
    mocks.page.mockImplementation(async () => ({
      records: [parseDecisionRecord(failed)!],
      nextCursor: null,
      total: 1,
    }));
    mocks.detail.mockImplementation(async () =>
      parseDecisionDetail({ record: failed, context: { text: 'Full note d0' } })
    );
    draw();
    const detail = await screen.findByTestId('decision-detail');
    const error = within(detail).getByTestId('decision-detail-error');
    expect(error).toHaveTextContent('Error: The decision model did not answer in 10 s');
    // Red is rare (design.md 9.13): ink, words and the failure glyph.
    const host = document.createElement('div');
    host.innerHTML = renderToStaticMarkup(<AlertCircle />);
    expect(error.querySelector('svg')?.innerHTML).toBe(host.querySelector('svg')?.innerHTML);
    expect(error.outerHTML).not.toMatch(/danger/);
  });

  it('keeps the bars, model, tier, latency, tokens, certainty, chunk and context under Details', async () => {
    draw();
    const detail = await screen.findByTestId('decision-detail');
    for (const hidden of [
      'tev1',
      'Model tier',
      '22 ms',
      '400 in · 1 out',
      'Certainty',
      'not validated on your data',
      'Chunk 1',
    ]) {
      expect(detail).not.toHaveTextContent(hidden);
    }
    expect(within(detail).queryAllByTestId('decision-option')).toHaveLength(0);
    expect(screen.queryByTestId('decision-context')).toBeNull();

    const details = await openDetails();
    const options = within(details).getAllByTestId('decision-option');
    expect(options.map((option) => option.textContent)).toEqual([
      'YesYes: 0.95, threshold 0.90',
      'NoNo: 0.05',
    ]);
    // The threshold is marked on the yes bar only.
    expect(options[0].querySelector<HTMLElement>('.br-pbar__mark')?.style.left).toBe('90%');
    expect(options[1].querySelector('.br-pbar__mark')).toBeNull();
    // R3: the alias and where it runs, never the provider key.
    expect(details).toHaveTextContent('tev1:0.8b · this computer');
    expect(details).not.toHaveTextContent('ollama_decision');
    // The fact is the decision MODEL's tier: a public chat that used a local
    // model reads "Model tier: Private", never a bare "Tier: Private".
    const tier = within(details).getByText('Model tier');
    expect(tier.tagName).toBe('DT');
    expect(tier.nextElementSibling).toHaveTextContent('Private');
    expect(within(details).queryByText('Tier')).toBeNull();
    expect(details).toHaveTextContent('22 ms');
    expect(details).toHaveTextContent('400 in · 1 out');
    const certainty = within(details).getByText('Certainty').closest('dt')!;
    expect(certainty.nextElementSibling).toHaveTextContent(/^0\.60$/);
    expect(within(certainty).getByRole('button', { name: 'About Certainty' })).toBeInTheDocument();
    expect(details).not.toHaveTextContent('Concentration');
    expect(details).toHaveTextContent("The decision model's own probabilities, not validated");
    expect(details).toHaveTextContent('Chunk 1');
    // Turn and batch ids are not facts any more (R6): only the raw JSON keeps them.
    expect(within(details).queryByText('Batch')).toBeNull();
    await waitFor(() =>
      expect(screen.getByTestId('decision-context')).toHaveTextContent(
        'Full note d0: 34F, RRMS dx 2019, EDSS 2.0'
      )
    );
    // Prose is read in the sans face; only JSON is code.
    expect(screen.getByTestId('decision-context').querySelector('.br-dh-context__text')).toBeNull();
    expect(mocks.detail).toHaveBeenCalledWith('s1', 'd0', expect.any(AbortSignal));
  });

  it('shows the turn and batch ids only in the raw JSON', async () => {
    serve([wire(0, { turnId: 'turn-7f3a9c', batchId: 'batch-1d2e3f' })]);
    draw();
    const detail = await screen.findByTestId('decision-detail');
    const details = await openDetails();
    expect(detail).not.toHaveTextContent('turn-7f3a9c');
    expect(detail).not.toHaveTextContent('batch-1d2e3f');
    await userEvent.click(within(details).getByRole('radio', { name: DECISION_HISTORY_COPY.raw }));
    const raw = await screen.findByTestId('decision-raw');
    expect(raw).toHaveTextContent('turn-7f3a9c');
    expect(raw).toHaveTextContent('batch-1d2e3f');
  });

  it('names a Baseten model by its alias and place, never by its slug or key', async () => {
    const slug = 'decision-models/baseten-joey-2b-latest';
    const baseten = (index: number) => {
      const row = wire(index, {
        model: { provider: 'baseten_decision', model: slug },
        servedModel: 'joey-2b-0.3',
        tier: 'public',
      });
      return { ...row, decisions: row.decisions.map((d) => ({ ...d, model: row.model })) };
    };
    serve([baseten(0)]);
    draw();
    const row = await screen.findByTestId('decision-row');
    expect(row).not.toHaveTextContent('baseten-joey-2b-latest');
    const details = await openDetails();
    expect(details).toHaveTextContent('joey-1 · Baseten');
    expect(details).not.toHaveTextContent(slug);
    expect(details).not.toHaveTextContent('baseten_decision');
    expect(details).toHaveTextContent('Served byjoey-2b-0.3');
  });

  it('says an uncalibrated call is an estimate up front, with the exact sentence under Details', async () => {
    serve([
      wire(0, {
        decisions: [
          {
            ...wire(0).decisions[0],
            source: { kind: 'verbalized', reason: 'the backend returned no logprobs' },
            calibrated: false,
          },
        ],
      }),
    ]);
    mocks.detail.mockResolvedValue(null);
    draw();
    const detail = await screen.findByTestId('decision-detail');
    const estimate = within(detail).getByTestId('decision-estimate');
    expect(estimate).toHaveTextContent(CALIBRATION_COPY.estimate);
    expect(within(estimate).getByRole('button', { name: /About/ })).toBeInTheDocument();
    expect(detail).not.toHaveTextContent('Uncalibrated: the model wrote this number itself');
    const details = await openDetails();
    expect(details).toHaveTextContent(
      'Uncalibrated: the model wrote this number itself (the backend returned no logprobs).'
    );
    await waitFor(() => expect(details).toHaveTextContent('This call is no longer in the log.'));
  });

  it('keeps Details open and the What it saw / Raw JSON choice across rows', async () => {
    draw();
    const details = await openDetails();
    await userEvent.click(within(details).getByRole('radio', { name: DECISION_HISTORY_COPY.raw }));
    expect(await screen.findByTestId('decision-raw')).toBeInTheDocument();
    fireEvent.keyDown(screen.getByTestId('decision-list'), { key: 'ArrowDown' });
    await waitFor(() =>
      expect(screen.getByTestId('decision-detail')).toHaveAttribute('data-decision-id', 'd1')
    );
    expect(screen.getByTestId('decision-details')).toBeInTheDocument();
    expect(screen.getByTestId('decision-raw')).toBeInTheDocument();
  });

  it('moves the selection with the arrow keys and fetches that call', async () => {
    draw();
    await screen.findByTestId('decision-detail');
    const list = screen.getByTestId('decision-list');
    fireEvent.keyDown(list, { key: 'ArrowDown' });
    await waitFor(() =>
      expect(screen.getByTestId('decision-detail')).toHaveAttribute('data-decision-id', 'd1')
    );
    expect(list.getAttribute('aria-activedescendant')).toBeTruthy();
    expect(document.getElementById(list.getAttribute('aria-activedescendant')!)).toHaveAttribute(
      'aria-selected',
      'true'
    );
    await waitFor(() =>
      expect(mocks.detail).toHaveBeenCalledWith('s1', 'd1', expect.any(AbortSignal))
    );
    fireEvent.click(within(list).getAllByTestId('decision-row')[3]);
    await waitFor(() =>
      expect(screen.getByTestId('decision-detail')).toHaveAttribute('data-decision-id', 'd3')
    );
  });

  it('fetches a call again when the person comes back to it before its first read landed', async () => {
    // d0's first read never answers: the person moves on before it would.
    let firstRead = true;
    mocks.detail.mockImplementation(async (_s: string, id: string, signal: AbortSignal) => {
      if (id === 'd0' && firstRead) {
        firstRead = false;
        return new Promise((_resolve, reject) =>
          signal.addEventListener('abort', () => reject(new Error('aborted')))
        );
      }
      return parseDecisionDetail({
        record: wire(Number(id.slice(1))),
        context: { text: `Full note ${id}` },
      });
    });
    draw();
    await waitFor(() =>
      expect(mocks.detail).toHaveBeenCalledWith('s1', 'd0', expect.any(AbortSignal))
    );
    await openDetails();
    const list = screen.getByTestId('decision-list');
    fireEvent.keyDown(list, { key: 'ArrowDown' });
    await waitFor(() =>
      expect(screen.getByTestId('decision-context')).toHaveTextContent('Full note d1')
    );
    fireEvent.keyDown(list, { key: 'ArrowUp' });
    await waitFor(() =>
      expect(screen.getByTestId('decision-context')).toHaveTextContent('Full note d0')
    );
    expect(mocks.detail.mock.calls.filter((call) => call[1] === 'd0')).toHaveLength(2);
  });

  it('shows the batch a decide card opened with, and lets it go with the chip or a new source', async () => {
    const { unmount } = draw({ origin: 'tool', batchId: 'b1' });
    await waitFor(() => expect(mocks.page).toHaveBeenCalled());
    expect(mocks.page.mock.calls[0][1]).toMatchObject({ origin: 'tool', batchId: 'b1' });
    const chip = screen.getByTestId('decision-filter-batch');
    expect(chip).toHaveTextContent('This decide call only');
    expect(chip).toHaveAccessibleName('Remove the filter: This decide call only');
    fireEvent.click(chip);
    await waitFor(() =>
      expect(mocks.page.mock.lastCall?.[1]).toMatchObject({ origin: 'tool', batchId: null })
    );
    expect(screen.queryByTestId('decision-filter-batch')).toBeNull();
    unmount();

    mocks.page.mockClear();
    draw({ origin: 'tool', batchId: 'b1' });
    await waitFor(() => expect(mocks.page).toHaveBeenCalled());
    const source = screen.getByTestId('decision-filter-source');
    expect(source).toHaveTextContent('Source: Decide calls');
    await userEvent.click(source);
    await userEvent.click(screen.getByRole('menuitemradio', { name: 'Any source' }));
    await waitFor(() =>
      expect(mocks.page.mock.lastCall?.[1]).toMatchObject({ origin: null, batchId: null })
    );
    expect(screen.queryByTestId('decision-filter-batch')).toBeNull();
    expect(screen.getByTestId('decision-filter-source')).toHaveTextContent(/^Source$/);
  });

  it('opens with the filter it was given, and applies a new one from a later open', async () => {
    const { rerender } = draw({ origin: 'monitor', originId: 'form' }, 1);
    await waitFor(() => expect(mocks.page).toHaveBeenCalled());
    expect(mocks.page.mock.calls[0][1]).toMatchObject({ origin: 'monitor', originId: 'form' });
    rerender(
      <DecisionHistory
        sessionId="s1"
        filter={{ origin: 'gate', originId: 'verify' }}
        filterKey={2}
      />
    );
    await waitFor(() =>
      expect(mocks.page.mock.lastCall?.[1]).toMatchObject({
        origin: 'gate',
        originId: 'verify',
      })
    );
    fireEvent.click(screen.getByTestId('decision-filter-clear'));
    await waitFor(() => expect(mocks.page.mock.lastCall?.[1]).toMatchObject({ origin: null }));
  });

  /**
   * A decision card's link names its one call. The view asks for that call alone and for its
   * detail at once, selects it, says so in a chip, and lets it go as soon as the person sets a
   * filter of their own.
   */
  it('opens on the one call a link names, selected, and lets it go once the person filters', async () => {
    mocks.page.mockImplementation(async (_s, filter: DecisionHistoryFilter, page) =>
      filter.decisionId
        ? { records: [parseDecisionRecord(wire(7))!], nextCursor: null, total: 1 }
        : pageAt(page?.before)
    );
    draw({ decisionId: 'd7', origin: 'tool', turnId: 't1' });
    await waitFor(() =>
      expect(mocks.detail).toHaveBeenCalledWith('s1', 'd7', expect.any(AbortSignal))
    );
    expect(mocks.detail).not.toHaveBeenCalledWith('s1', 'd0', expect.anything());
    const detail = await screen.findByTestId('decision-detail');
    expect(detail).toHaveAttribute('data-decision-id', 'd7');
    expect(mocks.page.mock.calls[0][1]).toMatchObject({ decisionId: 'd7', origin: 'tool' });
    expect(screen.getAllByTestId('decision-row')).toHaveLength(1);
    expect(screen.getByTestId('decision-filter-call')).toHaveTextContent('This call only');
    expect(screen.getByTestId('decision-filter-clear')).toBeInTheDocument();

    // Outcome: Error or refused is the query "Errors only" used to be.
    await userEvent.click(screen.getByTestId('decision-filter-outcome'));
    await userEvent.click(screen.getByRole('menuitemradio', { name: 'Error or refused' }));
    await waitFor(() =>
      expect(mocks.page.mock.lastCall?.[1]).toMatchObject({
        decisionId: null,
        errorsOnly: true,
        origin: 'tool',
        turnId: 't1',
      })
    );
    expect(screen.queryByTestId('decision-filter-call')).toBeNull();
    expect(screen.getByTestId('decision-filter-outcome')).toHaveTextContent(
      'Outcome: Error or refused'
    );
  });

  it('lets the This call only chip go on its own', async () => {
    draw({ decisionId: 'd7', origin: 'tool' });
    fireEvent.click(await screen.findByTestId('decision-filter-call'));
    await waitFor(() =>
      expect(mocks.page.mock.lastCall?.[1]).toMatchObject({ decisionId: null, origin: 'tool' })
    );
  });

  it('still selects the named call on a daemon that lists more than it', async () => {
    // A daemon from before `decisionId` ignores it and pages the source as usual.
    draw({ decisionId: 'd7', origin: 'tool' });
    const detail = await screen.findByTestId('decision-detail');
    await waitFor(() => expect(detail).toHaveAttribute('data-decision-id', 'd7'));
    expect(screen.getAllByTestId('decision-row').length).toBeGreaterThan(1);
  });

  it('says so when nothing matches, and when a read fails offers Retry', async () => {
    mocks.page.mockResolvedValueOnce({ records: [], nextCursor: null, total: 0 });
    const { unmount } = draw({ outcome: 'error' });
    expect(await screen.findByText('No decision calls match these filters.')).toBeInTheDocument();
    unmount();
    mocks.page.mockRejectedValueOnce(new Error('offline'));
    draw();
    expect(await screen.findByText('Could not load decision calls.')).toBeInTheDocument();
    const retry = screen.getByRole('button', { name: 'Retry' });
    expect(retry).toHaveClass('br-decision-link');
    fireEvent.click(retry);
    await waitFor(() => expect(screen.getAllByTestId('decision-row').length).toBeGreaterThan(0));
  });
});

describe('the filters (spec 1.2)', () => {
  const summaryWith = (
    aggregates: Array<Record<string, unknown>>,
    recentTurns: Array<Record<string, unknown>> = []
  ) =>
    mocks.summary.mockResolvedValue({
      data: { total: TOTAL, aggregates, recentTurns },
      missing: false,
    });

  const aggregate = (origin: string, originId: string, label: string, count = 5) => ({
    origin,
    originId,
    label,
    count,
    passed: 1,
    failed: count - 1,
    errors: 0,
    lastAt: '2026-11-04T10:00:00Z',
    lastAnswer: 'no',
    lastPassProbability: 0.1,
    model: null,
  });

  async function menuRows(testId: string) {
    await userEvent.click(screen.getByTestId(testId));
    const rows = screen.getAllByRole('menuitemradio').map((row) => row.textContent);
    await userEvent.keyboard('{Escape}');
    return rows;
  }

  it('offers only the sources this chat has, and only the outcome words its list shows', async () => {
    summaryWith([
      aggregate('monitor', 'no-haiku', 'No haiku'),
      aggregate('gate', 'probe', 'Probe'),
    ]);
    const { unmount } = draw({ origin: 'monitor' });
    await waitFor(() => expect(mocks.summary).toHaveBeenCalled());
    await waitFor(async () =>
      expect(await menuRows('decision-filter-source')).toEqual([
        'Any source',
        'Monitors',
        'Phase gates',
      ])
    );
    expect(await menuRows('decision-filter-outcome')).toEqual([
      'Any outcome',
      'Fired',
      'Quiet',
      'Error or refused',
    ]);
    unmount();
    draw({ origin: 'gate' });
    expect(await menuRows('decision-filter-outcome')).toEqual([
      'Any outcome',
      'Passed',
      'Not passed',
      'Error or refused',
    ]);
  });

  it('offers no threshold word to a decide call, and Error or refused to every source', async () => {
    const { unmount } = draw({ origin: 'tool' });
    expect(await menuRows('decision-filter-outcome')).toEqual(['Any outcome', 'Error or refused']);
    unmount();
    draw();
    expect(await menuRows('decision-filter-outcome')).toEqual(['Any outcome', 'Error or refused']);
  });

  it('applies Fired as the wire’s passed, and resets Outcome for a source without that word', async () => {
    summaryWith([
      aggregate('monitor', 'no-haiku', 'No haiku'),
      aggregate('tool', 'call-1', 'decide'),
    ]);
    draw({ origin: 'monitor' });
    await userEvent.click(screen.getByTestId('decision-filter-outcome'));
    await userEvent.click(screen.getByRole('menuitemradio', { name: 'Fired' }));
    await waitFor(() =>
      expect(mocks.page.mock.lastCall?.[1]).toMatchObject({ origin: 'monitor', outcome: 'passed' })
    );
    expect(screen.getByTestId('decision-filter-outcome')).toHaveTextContent('Outcome: Fired');
    await userEvent.click(screen.getByTestId('decision-filter-source'));
    await userEvent.click(screen.getByRole('menuitemradio', { name: 'Decide calls' }));
    await waitFor(() =>
      expect(mocks.page.mock.lastCall?.[1]).toMatchObject({ origin: 'tool', outcome: null })
    );
    expect(screen.getByTestId('decision-filter-outcome')).toHaveTextContent(/^Outcome$/);
  });

  it('still says an outcome a link applied that the source lists no row for', async () => {
    draw({ origin: 'tool', outcome: 'refused' });
    expect(screen.getByTestId('decision-filter-outcome')).toHaveTextContent('Outcome: Refused');
    expect(await menuRows('decision-filter-outcome')).toEqual([
      'Any outcome',
      'Error or refused',
      'Refused',
    ]);
  });

  it('names what the Filters chip holds, in field order', async () => {
    expect(filtersSetNouns({ origin: 'monitor', originId: 'form', turnId: 't1' })).toEqual([
      'monitor',
      'turn',
    ]);
    expect(filtersSetNouns({ origin: 'tool', originId: 'call-1', minP: 0.2 })).toEqual([
      'decide call',
      'probability',
    ]);
    expect(filtersSetNouns({})).toEqual([]);
    draw({ origin: 'monitor', originId: 'form', turnId: 't1' });
    await screen.findAllByTestId('decision-row');
    const chip = screen.getByTestId('decision-filter-more');
    expect(chip).toHaveTextContent('Filters: monitor, turn');
    expect(chip).toHaveAccessibleName('Filters, set: monitor, turn');
    expect(chip).toHaveAttribute('data-engaged', 'true');
  });

  it('picks which monitor and which turn inside the Filters popover, by name and time, never by id', async () => {
    summaryWith(
      [
        aggregate('monitor', 'no-haiku', 'No haiku', 5),
        aggregate('monitor', 'shell', 'Done with shell', 3),
      ],
      [
        {
          turnId: 'turn-9f8e7d',
          origin: 'monitor',
          originId: 'no-haiku',
          label: 'No haiku',
          count: 10,
          passed: 0,
          failed: 10,
          errors: 0,
          firstAt: '2026-11-04T10:00:00Z',
          lastAt: '2026-11-04T10:01:00Z',
          lastPassProbability: 0.1,
        },
      ]
    );
    draw({ origin: 'monitor' });
    await waitFor(() => expect(mocks.summary).toHaveBeenCalled());
    await userEvent.click(screen.getByTestId('decision-filter-more'));
    const popover = await screen.findByTestId('decision-filters-more');
    await waitFor(() =>
      expect(within(popover).getByTestId('decision-filter-origin')).toBeInTheDocument()
    );
    await userEvent.click(within(popover).getByTestId('decision-filter-origin'));
    expect(screen.getByRole('menuitemradio', { name: /Done with shell/ })).toHaveTextContent(
      'Done with shell3 calls'
    );
    await userEvent.click(screen.getByRole('menuitemradio', { name: /Done with shell/ }));
    await waitFor(() =>
      expect(mocks.page.mock.lastCall?.[1]).toMatchObject({ origin: 'monitor', originId: 'shell' })
    );
    // Picking applies at once and leaves the popover open.
    expect(screen.getByTestId('decision-filters-more')).toBeInTheDocument();
    expect(screen.getByTestId('decision-filter-origin')).toHaveTextContent(
      'Monitor: Done with shell'
    );
    await userEvent.click(screen.getByTestId('decision-filter-turn'));
    const turn = screen.getAllByRole('menuitemradio')[1];
    expect(turn).toHaveTextContent('10 calls');
    expect(turn).not.toHaveTextContent('turn-9f8e7d');
    await userEvent.click(turn);
    await waitFor(() =>
      expect(mocks.page.mock.lastCall?.[1]).toMatchObject({ turnId: 'turn-9f8e7d' })
    );
    expect(screen.getByTestId('decision-filter-more')).toHaveTextContent('Filters: monitor, turn');
  });

  it('names an unknown turn or monitor in words, never by its id (R6)', async () => {
    draw({ origin: 'monitor', originId: 'gone-monitor-id', turnId: 'turn-5a6b7c' });
    await userEvent.click(screen.getByTestId('decision-filter-more'));
    const popover = await screen.findByTestId('decision-filters-more');
    expect(within(popover).getByTestId('decision-filter-origin')).toHaveTextContent(
      "Monitor: Not in this chat's list"
    );
    expect(within(popover).getByTestId('decision-filter-turn')).toHaveTextContent(
      'Turn: An earlier turn'
    );
    expect(document.body).not.toHaveTextContent('gone-monitor-id');
    expect(document.body).not.toHaveTextContent('turn-5a6b7c');
  });

  it('applies the probability range as it is typed, not on blur (G-6)', async () => {
    draw();
    await waitFor(() => expect(mocks.page).toHaveBeenCalled());
    await userEvent.click(screen.getByTestId('decision-filter-more'));
    const lowest = await screen.findByRole('textbox', { name: 'Lowest probability' });
    await userEvent.type(lowest, '0.5');
    await waitFor(() => expect(mocks.page.mock.lastCall?.[1]).toMatchObject({ minP: 0.5 }));
    // "0." on the way to "0.5" applied 0 and left the field as typed.
    expect(lowest).toHaveValue('0.5');
    expect(lowest).toHaveFocus();
    const highest = screen.getByRole('textbox', { name: 'Highest probability' });
    await userEvent.type(highest, '0.2');
    expect(await screen.findByText(DECISION_HISTORY_COPY.pRangeInvalid)).toBeInTheDocument();
    // An invalid range applies nothing.
    expect(mocks.page.mock.lastCall?.[1]).toMatchObject({ minP: 0.5, maxP: null });
    await userEvent.clear(highest);
    await userEvent.type(highest, '0.9');
    await waitFor(() =>
      expect(mocks.page.mock.lastCall?.[1]).toMatchObject({ minP: 0.5, maxP: 0.9 })
    );
    expect(screen.getByTestId('decision-filter-more')).toHaveTextContent('Filters: probability');
  });
});

describe('following a running chat', () => {
  // The follow interval is captured and run by hand. Faking timers would also stop
  // jsdom's animation frames, which the list measures its scroll position with.
  let followTick: (() => void) | null = null;
  beforeEach(() => {
    followTick = null;
    const realSetInterval = globalThis.setInterval;
    vi.spyOn(globalThis, 'setInterval').mockImplementation(((
      handler: () => void,
      ms?: number,
      ...rest: unknown[]
    ) => {
      if (ms === FOLLOW_INTERVAL_MS) {
        followTick = handler;
        return 0 as unknown as ReturnType<typeof setInterval>;
      }
      return realSetInterval(handler, ms, ...rest);
    }) as typeof setInterval);
  });
  afterEach(() => {
    vi.restoreAllMocks();
  });

  async function tick() {
    expect(followTick).not.toBeNull();
    await act(async () => {
      followTick?.();
    });
  }

  /** `by` calls land at the top: every page read from now on lists and counts them. */
  function grow(by: number) {
    const fresh = Array.from(
      { length: by },
      (_, i) =>
        parseDecisionRecord(
          wire(0, {
            id: `new${by - i}`,
            createdAt: new Date(Date.UTC(2026, 10, 4, 10, 5, by - i)).toISOString(),
          })
        )!
    );
    mocks.page.mockImplementation(async (_s, _f, page) => {
      const base = pageAt(page?.before);
      return {
        ...base,
        total: TOTAL + by,
        records: page?.before ? base.records : [...fresh, ...base.records],
      };
    });
  }

  function drawInBar(filter: DecisionHistoryFilter = {}) {
    return render(
      <PreviewBarScope>
        <PreviewBar name="Decisions" />
        <DecisionHistory sessionId="s1" filter={filter} filterKey={1} />
      </PreviewBarScope>
    );
  }

  /** The follow tick's read: the list's own first call, under its own filter. */
  function tickReads() {
    return mocks.page.mock.calls.filter((call) => call[2]?.limit === 1);
  }
  function listReads() {
    return mocks.page.mock.calls.filter((call) => call[2]?.limit === DECISION_PAGE_SIZE);
  }

  it('reloads in place when new calls land while the list is at the top', async () => {
    drawInBar();
    await waitFor(() => expect(screen.getAllByTestId('decision-row').length).toBeGreaterThan(0));
    expect(listReads()).toHaveLength(1);

    // Nothing new: the tick reads the list's first call and leaves the list alone.
    await tick();
    await waitFor(() => expect(tickReads()).toHaveLength(1));
    expect(tickReads()[0][2]).toMatchObject({ before: null, limit: 1 });
    expect(listReads()).toHaveLength(1);

    grow(3);
    await tick();
    await waitFor(() => expect(listReads()).toHaveLength(2));
    expect(listReads()[1][2]).toMatchObject({ before: null });
    await waitFor(() =>
      expect(screen.getByTestId('decision-history-count')).toHaveTextContent('5,003 calls')
    );
    expect(screen.queryByTestId('decision-history-new-calls')).toBeNull();

    // The reload read the new first page, so the same calls do not reload again.
    await tick();
    await waitFor(() => expect(tickReads()).toHaveLength(3));
    expect(listReads()).toHaveLength(2);
  });

  it('offers "Show N new" instead of moving the rows under a reader who has scrolled', async () => {
    drawInBar();
    await waitFor(() => expect(screen.getAllByTestId('decision-row').length).toBeGreaterThan(0));
    const list = screen.getByTestId('decision-list');
    sizeList(list);
    act(() => {
      list.scrollTop = DECISION_ROW_HEIGHT * 40;
      fireEvent.scroll(list);
    });
    await waitFor(() =>
      expect(
        within(list)
          .getAllByTestId('decision-row')
          .some((row) => row.textContent?.includes('note-0045'))
      ).toBe(true)
    );

    grow(3);
    await tick();
    const button = await screen.findByTestId('decision-history-new-calls');
    expect(button).toHaveTextContent('Show 3 new');
    expect(listReads()).toHaveLength(1);

    fireEvent.click(button);
    await waitFor(() => expect(listReads()).toHaveLength(2));
    expect(listReads()[1][2]).toMatchObject({ before: null });
    expect(screen.queryByTestId('decision-history-new-calls')).toBeNull();
  });

  // W3-7: the list read two gate attempts, and the third landed before the summary
  // beside it was read. Counting against the summary, the view said "2 calls" for good.
  it('follows a call that landed between the list’s read and the summary’s', async () => {
    const attempts = [gateWire('g2', 0.1), gateWire('g1', 0.1)];
    serve(attempts);
    mocks.summary.mockResolvedValue({
      data: { total: 3, aggregates: [], recentTurns: [] },
      missing: false,
    });
    const filter: DecisionHistoryFilter = { origin: 'gate', originId: 'probe' };
    drawInBar(filter);
    await waitFor(() =>
      expect(screen.getByTestId('decision-history-count')).toHaveTextContent('2 calls')
    );
    await waitFor(() => expect(mocks.summary).toHaveBeenCalled());

    serve([gateWire('g3', 0.2), ...attempts]);
    await tick();
    await waitFor(() =>
      expect(screen.getByTestId('decision-history-count')).toHaveTextContent('3 calls')
    );
    expect(screen.getAllByTestId('decision-row')).toHaveLength(3);
    // The tick asked under the list's own filter, for one call.
    expect(tickReads()[0][1]).toMatchObject(filter);
  });

  it('reloads when the first call changed though the count did not', async () => {
    serve([gateWire('g2', 0.1), gateWire('g1', 0.1)]);
    drawInBar();
    await waitFor(() => expect(screen.getAllByTestId('decision-row')).toHaveLength(2));
    const reads = mocks.page.mock.calls.length;

    serve([gateWire('g3', 0.2), gateWire('g2', 0.1)]);
    await tick();
    await waitFor(() => expect(rowOf('g3')).toBeTruthy());
    expect(mocks.page.mock.calls.length).toBe(reads + 2);
    expect(screen.getByTestId('decision-history-count')).toHaveTextContent('2 calls');
  });
});
