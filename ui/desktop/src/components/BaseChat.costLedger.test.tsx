import { act, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ModelUsageRow, Session } from '../api';
import { footerCostInputs, selectBilledTokens, summaryTokensValue } from './BaseChat';
import { CostTracker } from './bottom_menu/CostTracker';
import { useCostTracking } from '../hooks/useCostTracking';
import { mostCompleteBilledTokens } from '../utils/usageAccounting';

/**
 * Vision V2-R5-5, end to end on the real pieces: the usage hook, BaseChat's two
 * helpers and the footer's `CostTracker`, wired the way BaseChat wires them
 * (`BaseChat.summaryRail.test.ts` pins that wiring at its source, since BaseChat
 * itself cannot be mounted in jsdom).
 *
 * Measured in the running app before the fix, on every open: Demo2 showed
 * "2.2M" and "$11.75" at 961 ms, then "N/A" and no cost at 2056 ms. The ledger
 * had a row with no certified total, and until it answered the counters stood
 * in for it, priced at the current model's rate.
 */
const mocks = vi.hoisted(() => ({
  getSessionUsage: vi.fn(),
  pricing: vi.fn(),
}));
vi.mock('../api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../api')>()),
  getSessionUsage: mocks.getSessionUsage,
}));
vi.mock('../utils/pricing', () => ({ fetchModelPricing: mocks.pricing }));
vi.mock('../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'synthetic-proof' }),
}));
vi.mock('./ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({ currentModel: 'claude-opus-4-5', currentProvider: 'anthropic' }),
}));

/** Demo2's counters: 2.2M billed tokens, which the current rate prices at $11.75. */
const demo2 = {
  id: 'demo2',
  accumulated_input_tokens: 2_000_000,
  accumulated_output_tokens: 200_000,
  accumulated_total_tokens: 2_200_000,
} as unknown as Session;

/**
 * A historical ledger row: no certified total, so the honest figure is N/A, and
 * a model the pricing table does not know, so the honest cost is none.
 */
const uncertified: ModelUsageRow = {
  provider: 'anthropic',
  modelId: 'claude-retired',
  inputTokens: 2_000_000,
  outputTokens: 200_000,
  cacheReadTokens: null,
  cacheCreationTokens: null,
  totalTokens: null,
  turns: 40,
};

function Harness({ session, loading = false }: { session?: Session; loading?: boolean }) {
  const { sessionCosts, modelRows, ledgerSettled } = useCostTracking({ session });
  const footer = footerCostInputs({
    ready: !!session && ledgerSettled,
    tokenState: null,
    session,
    sessionCosts,
    modelRows,
  });
  const tokens = summaryTokensValue({
    loading,
    ledgerSettled,
    billed: mostCompleteBilledTokens(selectBilledTokens(null, session), modelRows),
  });
  return (
    <>
      <span data-testid="tokens">{tokens}</span>
      <CostTracker
        inputTokens={footer.accumulatedInputTokens}
        outputTokens={footer.accumulatedOutputTokens}
        sessionCosts={footer.sessionCosts}
        modelCostRows={footer.modelCostRows}
      />
    </>
  );
}

/** Every reading the two figures take, as the DOM shows it, in order. */
function watchReadings(container: HTMLElement) {
  const readings: string[] = [];
  const read = () => {
    const tokens = container.querySelector('[data-testid="tokens"]')?.textContent ?? '';
    const cost = container.querySelector('[data-testid="chat-cost"]')?.textContent ?? '';
    const reading = `tokens:${tokens} cost:${cost}`;
    if (readings[readings.length - 1] !== reading) readings.push(reading);
  };
  read();
  const observer = new MutationObserver(read);
  observer.observe(container, { subtree: true, childList: true, characterData: true });
  return { readings, stop: () => observer.disconnect() };
}

beforeEach(() => {
  mocks.getSessionUsage.mockReset();
  // The current model at $5 per million either way, so the counters would
  // price at $11.00; the retired model has no price.
  mocks.pricing
    .mockReset()
    .mockImplementation(async (_provider: string, model: string) =>
      model === 'claude-opus-4-5'
        ? { input_token_cost: 0.000005, output_token_cost: 0.000005, currency: '$' }
        : null
    );
});

describe('summaryTokensValue', () => {
  it('is empty while the transcript loads or the ledger is out, then one value', () => {
    expect(summaryTokensValue({ loading: true, ledgerSettled: true, billed: 1 })).toBeNull();
    expect(summaryTokensValue({ loading: false, ledgerSettled: false, billed: 1 })).toBeNull();
    expect(summaryTokensValue({ loading: false, ledgerSettled: true, billed: null })).toBe('N/A');
    expect(summaryTokensValue({ loading: false, ledgerSettled: true, billed: 2_200_000 })).toBe(
      '2.2M'
    );
  });
});

describe('footerCostInputs', () => {
  it('hands the footer nothing to price until the ledger is in', () => {
    const inputs = {
      tokenState: { accumulatedInputTokens: 5, accumulatedOutputTokens: 6 },
      session: demo2,
      sessionCosts: {},
      modelRows: [],
    };
    expect(footerCostInputs({ ...inputs, ready: false })).toEqual({});
    expect(footerCostInputs({ ...inputs, ready: true })).toEqual({
      accumulatedInputTokens: 5,
      accumulatedOutputTokens: 6,
      sessionCosts: {},
      modelCostRows: [],
    });
  });
});

describe('the Tokens value and the footer cost on a chat open (V2-R5-5)', () => {
  it('show nothing while the usage request is pending, then exactly one reading', async () => {
    let answer!: (models: ModelUsageRow[]) => void;
    mocks.getSessionUsage.mockReturnValue(
      new Promise((resolve) => (answer = (models) => resolve({ data: { models } })))
    );
    const { container } = render(<Harness session={demo2} />);
    const watch = watchReadings(container);
    await waitFor(() => expect(mocks.getSessionUsage).toHaveBeenCalled());
    // Long enough for the footer's own pricing read to land, which is when the
    // counters used to be priced.
    await waitFor(() => expect(mocks.pricing).toHaveBeenCalled());
    await act(async () => {});
    expect(screen.getByTestId('tokens')).toBeEmptyDOMElement();
    expect(screen.queryByTestId('chat-cost')).not.toBeInTheDocument();

    await act(async () => answer([uncertified]));
    await waitFor(() => expect(screen.getByTestId('tokens')).toHaveTextContent('N/A'));
    await act(async () => {});
    watch.stop();
    // Empty, then the ledger's honest answer; never "2.2M" or "$11.00" between.
    expect(watch.readings).toEqual(['tokens: cost:', 'tokens:N/A cost:']);
  });

  it('prices the counters when the ledger answers with no rows, and only then', async () => {
    let answer!: (models: ModelUsageRow[]) => void;
    mocks.getSessionUsage.mockReturnValue(
      new Promise((resolve) => (answer = (models) => resolve({ data: { models } })))
    );
    const { container } = render(<Harness session={demo2} />);
    const watch = watchReadings(container);
    await waitFor(() => expect(mocks.pricing).toHaveBeenCalled());
    await act(async () => {});
    expect(screen.queryByTestId('chat-cost')).not.toBeInTheDocument();
    expect(screen.getByTestId('tokens')).toBeEmptyDOMElement();

    await act(async () => answer([]));
    await waitFor(() => expect(screen.getByTestId('chat-cost')).toHaveTextContent('$11.00'));
    watch.stop();
    expect(screen.getByTestId('tokens')).toHaveTextContent('2.2M');
    expect(watch.readings[0]).toBe('tokens: cost:');
    expect(watch.readings[watch.readings.length - 1]).toBe('tokens:2.2M cost:$11.00');
  });

  it('shows no Tokens value while the transcript itself is loading', async () => {
    mocks.getSessionUsage.mockReturnValue(new Promise(() => {}));
    render(<Harness session={undefined} loading />);
    await waitFor(() => expect(mocks.pricing).toHaveBeenCalled());
    await act(async () => {});
    expect(screen.getByTestId('tokens')).toBeEmptyDOMElement();
    expect(screen.queryByTestId('chat-cost')).not.toBeInTheDocument();
    expect(mocks.getSessionUsage).not.toHaveBeenCalled();
  });
});
