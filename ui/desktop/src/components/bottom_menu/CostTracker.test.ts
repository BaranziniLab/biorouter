import { describe, expect, it } from 'vitest';
import type { PricingData } from '../../api';
import type { ModelCostRow } from '../../hooks/useCostTracking';
import {
  aggregateModelRowsCost,
  costIsWorthShowing,
  costSummary,
  formatCostEstimate,
  formatTooltipMoney,
} from './CostTracker';

const priced: ModelCostRow = {
  provider: 'openai',
  model: 'priced',
  inputTokens: 100,
  outputTokens: 20,
  cacheReadTokens: 0,
  cacheCreationTokens: 0,
  totalTokens: 120,
  turns: 1,
  totalCost: 1.25,
  costIsPartial: false,
};

const unpriced: ModelCostRow = {
  provider: 'unknown',
  model: 'unpriced',
  inputTokens: 50,
  outputTokens: 10,
  cacheReadTokens: 0,
  cacheCreationTokens: 0,
  totalTokens: 60,
  turns: 1,
  totalCost: null,
  costIsPartial: false,
};

/** $5 per million tokens, either way. */
const RATE: PricingData = {
  provider: 'anthropic',
  model: 'claude-opus-4-5',
  input_token_cost: 0.000005,
  output_token_cost: 0.000005,
  currency: '€',
};

describe('formatTooltipMoney', () => {
  it('keeps card costs compact', () => {
    expect(formatTooltipMoney(0)).toBe('$0.00');
    expect(formatTooltipMoney(0.0000042)).toBe('<$0.01');
    expect(formatTooltipMoney(12.345)).toBe('$12.35');
    expect(formatTooltipMoney(null)).toBe('Unavailable');
    expect(formatTooltipMoney(Number.NaN)).toBe('Unavailable');
  });
});

describe('cost estimates', () => {
  it('labels mixed known and unknown rows as a conservative subtotal', () => {
    const estimate = aggregateModelRowsCost([priced, unpriced]);
    expect(estimate).toEqual({ amount: 1.25, partial: true });
    expect(formatCostEstimate(estimate)).toBe('$1.25');
  });

  it('keeps an entirely unknown total unavailable', () => {
    const unknownBucket: ModelCostRow = {
      provider: undefined,
      model: undefined,
      inputTokens: 50,
      outputTokens: 10,
      cacheReadTokens: 5,
      cacheCreationTokens: 0,
      totalTokens: 65,
      turns: 1,
      totalCost: null,
      costIsPartial: false,
    };
    const estimate = aggregateModelRowsCost([unknownBucket]);

    expect(estimate).toEqual({ amount: null, partial: true });
    expect(formatCostEstimate(estimate)).toBe('Unavailable');
  });

  it('sums the legacy map’s rows the same way', () => {
    expect(
      aggregateModelRowsCost([{ totalCost: 1, costIsPartial: true }, { totalCost: 0.5 }])
    ).toEqual({ amount: 1.5, partial: true });
  });
});

describe('costIsWorthShowing (spec 3.7: a cost appears only above $0)', () => {
  it.each([
    [{ amount: 0, partial: false }, false],
    [{ amount: null, partial: true }, false],
    [{ amount: Number.NaN, partial: false }, false],
    [{ amount: 0.004, partial: true }, true],
    [{ amount: 1.25, partial: false }, true],
  ])('%o -> %s', (estimate, shown) => {
    expect(costIsWorthShowing(estimate)).toBe(shown);
  });
});

describe('costSummary: one card from each of the three data paths', () => {
  const none = { inputTokens: 0, outputTokens: 0, pricing: null };

  it('reads the ledger rows first, with their tokens summed and a row per model', () => {
    expect(
      costSummary({
        ...none,
        modelCostRows: [priced, unpriced],
        sessionCosts: { 'x/y': { inputTokens: 9, outputTokens: 9, totalCost: 9 } },
        inputTokens: 2_000_000,
        pricing: RATE,
      })
    ).toEqual({
      estimate: { amount: 1.25, partial: true },
      currency: '$',
      inputTokens: 150,
      outputTokens: 30,
      models: [priced, unpriced],
    });
  });

  it('then the legacy map, which has no rows to list', () => {
    expect(
      costSummary({
        ...none,
        sessionCosts: { 'openai/priced': { inputTokens: 100, outputTokens: 20, totalCost: 1.25 } },
        pricing: RATE,
      })
    ).toEqual({
      estimate: { amount: 1.25, partial: false },
      currency: '$',
      inputTokens: 100,
      outputTokens: 20,
      models: [],
    });
  });

  it('then the counters at the current rate, always as an estimate', () => {
    expect(costSummary({ inputTokens: 2_000_000, outputTokens: 200_000, pricing: RATE })).toEqual({
      estimate: { amount: 11, partial: true },
      currency: '€',
      inputTokens: 2_000_000,
      outputTokens: 200_000,
      models: [],
    });
  });

  it('draws nothing for a path whose total is not worth showing, and never falls through', () => {
    // A ledger that prices the chat at nothing is the answer; the counters are not priced.
    expect(
      costSummary({
        modelCostRows: [unpriced],
        inputTokens: 2_000_000,
        outputTokens: 200_000,
        pricing: RATE,
      })
    ).toBeNull();
    expect(
      costSummary({
        sessionCosts: { 'a/b': { inputTokens: 1, outputTokens: 1, totalCost: 0 } },
        inputTokens: 2_000_000,
        outputTokens: 200_000,
        pricing: RATE,
      })
    ).toBeNull();
    expect(costSummary({ ...none, pricing: RATE })).toBeNull();
    // No price yet (loading), or none for this model.
    expect(
      costSummary({ inputTokens: 2_000_000, outputTokens: 200_000, pricing: null })
    ).toBeNull();
  });
});
