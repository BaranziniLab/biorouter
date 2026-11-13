import { describe, expect, it, vi } from 'vitest';
import type { ModelUsageRow } from '../api';
import { buildModelCostRows, serverPricedCost } from './useCostTracking';

/**
 * The daemon prices each request on its own (cache buckets at the cache rate, a request over a
 * long-context threshold at the long-context rate), so the card shows its figure when the row
 * carries one and prices the row here only when it does not.
 */
const row = (overrides: Partial<ModelUsageRow> = {}): ModelUsageRow => ({
  provider: 'versa_azure',
  modelId: 'gpt-5.6-sol',
  inputTokens: 300_000,
  outputTokens: 10_000,
  cacheReadTokens: 0,
  cacheCreationTokens: 0,
  totalTokens: 310_000,
  turns: 1,
  ...overrides,
});

/** $4 / $20 per million, flat: what the client would charge without the long-context tier. */
const flat = vi.fn(async () => ({ input_token_cost: 0.000004, output_token_cost: 0.00002 }));

describe('the cost card prefers the daemon’s price', () => {
  it('uses the row’s own cost and partial flag, without asking the pricing table', async () => {
    flat.mockClear();
    const [priced] = await buildModelCostRows([row({ cost: 2.7, costIsPartial: true })], flat);
    expect(priced.totalCost).toBe(2.7);
    expect(priced.costIsPartial).toBe(true);
    expect(flat).not.toHaveBeenCalled();
  });

  it('falls back to the client price when the row carries none', async () => {
    flat.mockClear();
    const [unpriced] = await buildModelCostRows([row({ cost: null })], flat);
    // 300k × $4/M + 10k × $20/M.
    expect(unpriced.totalCost).toBeCloseTo(1.4, 6);
    expect(flat).toHaveBeenCalledTimes(1);
    const [legacy] = await buildModelCostRows([row()], flat);
    expect(legacy.totalCost).toBeCloseTo(1.4, 6);
  });

  it('reads the fields defensively', () => {
    expect(serverPricedCost(null)).toBeNull();
    expect(serverPricedCost({ cost: 'two' })).toBeNull();
    expect(serverPricedCost({ cost: Number.NaN })).toBeNull();
    expect(serverPricedCost({ cost: -1 })).toBeNull();
    expect(serverPricedCost({ cost: 0 })).toEqual({ cost: 0, partial: false });
    expect(serverPricedCost({ cost: 1, costIsPartial: 'yes' })).toEqual({
      cost: 1,
      partial: false,
    });
  });
});
