import { act, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ModelCostRow } from '../../hooks/useCostTracking';
import { CostTracker } from './CostTracker';

/**
 * What the footer's cost does with what it is handed. BaseChat leans on all of
 * it (V2-R5-5): until a chat's usage ledger answers it passes NONE of the four
 * props, so the footer must draw nothing for that, and must never price the
 * bare counters on its own account. `BaseChat.costLedger.test.tsx` proves the
 * whole path through BaseChat's helpers; this file keeps the component's half
 * of the contract in the component's own suite, so a change here fails here.
 */
const mocks = vi.hoisted(() => ({ pricing: vi.fn() }));
vi.mock('../../utils/pricing', () => ({ fetchModelPricing: mocks.pricing }));
vi.mock('../ModelAndProviderContext', () => ({
  useModelAndProvider: () => ({ currentModel: 'claude-opus-4-5', currentProvider: 'anthropic' }),
}));

/** The current model at $5 per million tokens, either way. */
const RATE = { input_token_cost: 0.000005, output_token_cost: 0.000005, currency: '$' };

const ledgerRow: ModelCostRow = {
  provider: 'anthropic',
  model: 'claude-opus-4-5',
  inputTokens: 100,
  outputTokens: 50,
  cacheReadTokens: null,
  cacheCreationTokens: null,
  totalTokens: 150,
  turns: 1,
  totalCost: 3.5,
  costIsPartial: false,
};

/** Lets the footer's own price lookup land, which is when a counter would be priced. */
async function settlePricing() {
  await waitFor(() => expect(mocks.pricing).toHaveBeenCalled());
  await act(async () => {});
}

beforeEach(() => {
  localStorage.clear();
  mocks.pricing.mockReset().mockResolvedValue(RATE);
});

describe('CostTracker', () => {
  it('draws nothing when handed nothing to price, even for a priced model', async () => {
    render(<CostTracker />);
    await settlePricing();
    expect(screen.queryByTestId('chat-cost')).not.toBeInTheDocument();
  });

  it('draws nothing for an empty ledger and zero counters', async () => {
    render(<CostTracker inputTokens={0} outputTokens={0} sessionCosts={{}} modelCostRows={[]} />);
    await settlePricing();
    expect(screen.queryByTestId('chat-cost')).not.toBeInTheDocument();
  });

  it('prices bare counters at the current rate once there is no ledger row to read', async () => {
    // The fallback BaseChat withholds until the ledger has answered: it is only
    // the right reading when the ledger is known to hold no rows.
    render(<CostTracker inputTokens={2_000_000} outputTokens={200_000} />);
    await waitFor(() => expect(screen.getByTestId('chat-cost')).toHaveTextContent('$11.00'));
  });

  it('reads the ledger rows in preference to the counters', async () => {
    render(
      <CostTracker inputTokens={2_000_000} outputTokens={200_000} modelCostRows={[ledgerRow]} />
    );
    await waitFor(() => expect(screen.getByTestId('chat-cost')).toHaveTextContent('$3.50'));
  });

  it('draws nothing when the person has switched pricing off', async () => {
    localStorage.setItem('show_pricing', 'false');
    render(<CostTracker modelCostRows={[ledgerRow]} />);
    await act(async () => {});
    expect(screen.queryByTestId('chat-cost')).not.toBeInTheDocument();
  });
});
