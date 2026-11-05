import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  parseDecisionDetail,
  parseDecisionRecord,
  type DecisionHistoryFilter,
  type DecisionPage,
} from '../../utils/sessionDecisions';
import DecisionHistory, {
  DECISION_PAGE_SIZE,
  DECISION_ROW_HEIGHT,
  recordProbability,
  visibleRange,
} from './DecisionHistory';

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

describe('the window over the list', () => {
  it('draws only the rows in view, plus a margin', () => {
    expect(visibleRange(0, 440, 5000)).toEqual({ start: 0, end: 18 });
    expect(visibleRange(44 * 1000, 440, 5000)).toEqual({ start: 992, end: 1018 });
    expect(visibleRange(44 * 4999, 440, 5000).end).toBe(5000);
    expect(visibleRange(0, 440, 0)).toEqual({ start: 0, end: 0 });
  });

  it('reads a row’s bar as its lowest pass probability', () => {
    const record = parseDecisionRecord(wire(3))!;
    expect(recordProbability(record)).toBe(0.95);
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

  it('shows the selected call in full: its exact context, options with bars, threshold and calibration', async () => {
    draw();
    const detail = await screen.findByTestId('decision-detail');
    expect(detail).toHaveAttribute('data-decision-id', 'd0');
    await waitFor(() =>
      expect(screen.getByTestId('decision-context')).toHaveTextContent(
        'Full note d0: 34F, RRMS dx 2019, EDSS 2.0'
      )
    );
    expect(mocks.detail).toHaveBeenCalledWith('s1', 'd0', expect.any(AbortSignal));
    const question = within(detail).getByTestId('decision-question');
    expect(question).toHaveTextContent('Threshold P(yes) ≥ 0.90');
    expect(question).toHaveTextContent('Passed');
    const options = within(question).getAllByTestId('decision-option');
    expect(options.map((option) => option.textContent)).toEqual([
      'YesYes: 0.95, threshold 0.90',
      'NoNo: 0.05',
    ]);
    // The threshold is marked on the yes bar only.
    expect(options[0].querySelector<HTMLElement>('.br-pbar__mark')?.style.left).toBe('90%');
    expect(options[1].querySelector('.br-pbar__mark')).toBeNull();
    expect(question).toHaveTextContent('not validated on your data');
    expect(detail).toHaveTextContent('tev1:0.8b (ollama_decision)');
    expect(detail).toHaveTextContent('Private');
    expect(detail).toHaveTextContent('22 ms');
    expect(detail).toHaveTextContent('400 in · 1 out');
  });

  it('says an uncalibrated call is uncalibrated, in words', async () => {
    mocks.page.mockResolvedValue({
      records: [
        parseDecisionRecord(
          wire(0, {
            decisions: [
              {
                ...wire(0).decisions[0],
                source: { kind: 'verbalized', reason: 'the backend returned no logprobs' },
                calibrated: false,
              },
            ],
          })
        )!,
      ],
      nextCursor: null,
      total: 1,
    });
    mocks.detail.mockResolvedValue(null);
    draw();
    const detail = await screen.findByTestId('decision-detail');
    expect(detail).toHaveTextContent(
      'Uncalibrated: the model wrote this number itself (the backend returned no logprobs).'
    );
    await waitFor(() => expect(detail).toHaveTextContent('This call is no longer in the log.'));
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

  it('says so when nothing matches, and when a read fails offers Retry', async () => {
    mocks.page.mockResolvedValueOnce({ records: [], nextCursor: null, total: 0 });
    const { unmount } = draw({ outcome: 'error' });
    expect(await screen.findByText('No decision calls match these filters.')).toBeInTheDocument();
    unmount();
    mocks.page.mockRejectedValueOnce(new Error('offline'));
    draw();
    expect(await screen.findByText('Could not load decision calls.')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    await waitFor(() => expect(screen.getAllByTestId('decision-row').length).toBeGreaterThan(0));
  });
});
