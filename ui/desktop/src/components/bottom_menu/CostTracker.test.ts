import { describe, expect, it } from 'vitest';
import {
  aggregateModelRowsCost,
  costEstimateSummary,
  costIsWorthShowing,
  formatCostEstimate,
  formatTooltipMoney,
  sessionTokensSummary,
} from './CostTracker';

describe('formatTooltipMoney', () => {
  it('keeps tooltip costs compact', () => {
    expect(formatTooltipMoney(0)).toBe('$0.00');
    expect(formatTooltipMoney(0.0000042)).toBe('<$0.01');
    expect(formatTooltipMoney(12.345)).toBe('$12.35');
    expect(formatTooltipMoney(null)).toBe('Unavailable');
    expect(formatTooltipMoney(Number.NaN)).toBe('Unavailable');
  });
});

describe('sessionTokensSummary', () => {
  it('shows only input and output on separate lines', () => {
    expect(sessionTokensSummary(12_000_000, 1_300_000)).toBe(
      'Input: 12,000,000 tokens\nOutput: 1,300,000 tokens'
    );
  });
});

describe('cost estimates', () => {
  it('labels mixed known and unknown rows as a conservative subtotal', () => {
    const estimate = aggregateModelRowsCost([
      {
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
      },
      {
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
      },
    ]);

    expect(estimate).toEqual({ amount: 1.25, partial: true });
    expect(formatCostEstimate(estimate)).toBe('$1.25');
    expect(costEstimateSummary(estimate)).toBe(
      'Estimated total: $1.25\nConservative estimate based on available token and pricing data.'
    );
  });

  it('keeps the total exact beside a local decision model, which has no meter', () => {
    const chat = {
      provider: 'anthropic',
      model: 'claude-fable-5',
      inputTokens: 100,
      outputTokens: 20,
      cacheReadTokens: 0,
      cacheCreationTokens: 0,
      totalTokens: 120,
      turns: 1,
      totalCost: 0.42,
      costIsPartial: false,
    };
    const local = {
      ...chat,
      provider: 'ollama_decision',
      model: 'tev1:0.8b',
      turns: 20,
      totalCost: null,
    };
    const estimate = aggregateModelRowsCost([chat, local]);
    expect(estimate).toEqual({ amount: 0.42, partial: false });
    expect(costEstimateSummary(estimate)).toBe('Total cost: $0.42');

    // A local decision model alone costs $0, not an unknown amount.
    expect(aggregateModelRowsCost([local])).toEqual({ amount: 0, partial: false });

    // Baseten bills its decision models, so an unpriced one leaves it partial.
    const hosted = { ...local, provider: 'baseten_decision', model: 'decision-models/x' };
    expect(aggregateModelRowsCost([chat, hosted])).toEqual({ amount: 0.42, partial: true });
  });

  it('keeps an entirely unknown total unavailable', () => {
    const estimate = aggregateModelRowsCost([
      {
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
      },
    ]);

    expect(estimate).toEqual({ amount: null, partial: true });
    expect(formatCostEstimate(estimate)).toBe('Unavailable');
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
