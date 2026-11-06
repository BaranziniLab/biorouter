import { describe, expect, it } from 'vitest';
import type { PricingData } from '../../api';
import type { ModelCostRow } from '../../hooks/useCostTracking';
import { billedTokens, mostCompleteBilledTokens } from '../../utils/usageAccounting';
import { summaryTokensValue } from '../BaseChat';
import { COST_CARD_COPY } from './copy';
import {
  aggregateModelRowsCost,
  costIsWorthShowing,
  costSummary,
  formatCostEstimate,
  formatTooltipMoney,
  recordedTokenSum,
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
    expect(formatCostEstimate(estimate)).toBe('$0.42');

    // A local decision model alone costs $0, not an unknown amount.
    expect(aggregateModelRowsCost([local])).toEqual({ amount: 0, partial: false });

    // Baseten bills its decision models, so an unpriced one leaves it partial.
    const hosted = { ...local, provider: 'baseten_decision', model: 'decision-models/x' };
    expect(aggregateModelRowsCost([chat, hosted])).toEqual({ amount: 0.42, partial: true });
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
      cacheReadTokens: 0,
      cacheCreationTokens: 0,
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
      // The map's writer set no cache buckets: not recorded, not zero.
      cacheReadTokens: null,
      cacheCreationTokens: null,
      models: [],
    });
  });

  it('then the legacy map, with the cache buckets its rows carry', () => {
    expect(
      costSummary({
        ...none,
        sessionCosts: {
          'anthropic/a': {
            inputTokens: 100,
            outputTokens: 20,
            cacheReadTokens: 4_000,
            cacheCreationTokens: 600,
            totalCost: 1,
          },
          'anthropic/b': { inputTokens: 10, outputTokens: 2, cacheReadTokens: 50, totalCost: 0.5 },
        },
      })
    ).toMatchObject({
      inputTokens: 110,
      outputTokens: 22,
      cacheReadTokens: 4_050,
      cacheCreationTokens: 600,
    });
  });

  it('then the counters at the current rate, always as an estimate', () => {
    expect(costSummary({ inputTokens: 2_000_000, outputTokens: 200_000, pricing: RATE })).toEqual({
      estimate: { amount: 11, partial: true },
      currency: '€',
      inputTokens: 2_000_000,
      outputTokens: 200_000,
      // The counters never split the cache out, so this path says nothing about it.
      cacheReadTokens: null,
      cacheCreationTokens: null,
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

/**
 * The owner's chat (versa_azure gpt-5.5, 2026-11): the card read "22,788 input · 382 output"
 * while the summary's Tokens value read 89k and Settings > App > Usage listed "Cache read 65,408".
 * The ledger's four buckets are disjoint and add up to the billed total, so the card lists the
 * cache too.
 */
const ownersChat: ModelCostRow = {
  provider: 'versa_azure',
  model: 'gpt-5.5-2026-04-24',
  inputTokens: 22_788,
  outputTokens: 382,
  cacheReadTokens: 65_408,
  cacheCreationTokens: 0,
  totalTokens: 88_578,
  turns: 4,
  totalCost: 0.14,
  costIsPartial: false,
};

/** A model with prompt caching both ways (Anthropic), so both cache buckets hold tokens. */
const cachedBothWays: ModelCostRow = {
  provider: 'anthropic',
  model: 'claude-opus-4-5',
  inputTokens: 1_200,
  outputTokens: 800,
  cacheReadTokens: 40_000,
  cacheCreationTokens: 9_000,
  totalTokens: 51_000,
  turns: 3,
  totalCost: 0.42,
  costIsPartial: false,
};

/** A detail's lines as they read: the no-break spaces that hold each figure to its word are spaces. */
const plain = (lines: string[]) => lines.map((line) => line.replace(/\u00a0/g, ' '));

/** Every grouped figure in a detail's lines, money left out. */
function tokenFigures(lines: string[]): number[] {
  return plain(lines)
    .join(' · ')
    .split(' · ')
    .filter((part) => !part.startsWith('$') && part !== 'Unavailable')
    .map((part) => Number(part.split(' ')[0].replace(/,/g, '')));
}

const sum = (figures: number[]) => figures.reduce((total, figure) => total + figure, 0);

describe('recordedTokenSum', () => {
  it('sums the rows that recorded the bucket, and is null when none did', () => {
    expect(recordedTokenSum([65_408, 0, null, undefined])).toBe(65_408);
    expect(recordedTokenSum([0, 0])).toBe(0);
    expect(recordedTokenSum([null, undefined])).toBeNull();
    expect(recordedTokenSum([])).toBeNull();
    // A value the ledger could not have written adds nothing.
    expect(recordedTokenSum([Number.NaN, -5, Number.POSITIVE_INFINITY, 7])).toBe(7);
  });
});

describe('the cost card’s token lines', () => {
  it('lists input and output alone when the cache holds nothing or was not recorded', () => {
    const tokens = { inputTokens: 24_715, outputTokens: 648 };
    expect(plain(COST_CARD_COPY.tokens(tokens))).toEqual(['24,715 input · 648 output']);
    expect(
      plain(COST_CARD_COPY.tokens({ ...tokens, cacheReadTokens: 0, cacheCreationTokens: null }))
    ).toEqual(['24,715 input · 648 output']);
    expect(plain(COST_CARD_COPY.modelDetail('$0.14', tokens))).toEqual([
      '$0.14 · 24,715 in · 648 out',
    ]);
  });

  it('keeps one cache bucket on the same line, after output, as Settings > App > Usage orders them', () => {
    expect(plain(COST_CARD_COPY.tokens(ownersChat))).toEqual([
      '22,788 input · 382 output · 65,408 cache read',
    ]);
    expect(plain(COST_CARD_COPY.modelDetail('$0.14', ownersChat))).toEqual([
      '$0.14 · 22,788 in · 382 out · 65,408 cache read',
    ]);
    expect(
      plain(
        COST_CARD_COPY.tokens({ inputTokens: 900, outputTokens: 40, cacheCreationTokens: 3_000 })
      )
    ).toEqual(['900 input · 40 output · 3,000 cache write']);
  });

  it('breaks a line that is too long only after a "·", never inside a figure and its word', () => {
    // An unpriced model's row with a big cache runs past the 294px column in Chromium; a plain
    // space let it break between "cache" and "read".
    const [line] = COST_CARD_COPY.modelDetail('Unavailable', {
      inputTokens: 20_000,
      outputTokens: 1_500,
      cacheReadTokens: 300_000,
    });
    // The places a browser may break are the ordinary spaces: one after each "·".
    expect(line.split(' ')).toEqual([
      'Unavailable\u00a0·',
      '20,000\u00a0in\u00a0·',
      '1,500\u00a0out\u00a0·',
      '300,000\u00a0cache\u00a0read',
    ]);
    expect(COST_CARD_COPY.tokens(cachedBothWays).map((part) => part.split(' '))).toEqual([
      ['1,200\u00a0input\u00a0·', '800\u00a0output'],
      ['40,000\u00a0cache\u00a0read\u00a0·', '9,000\u00a0cache\u00a0write'],
    ]);
  });

  it('gives both cache buckets a second line of their own', () => {
    expect(plain(COST_CARD_COPY.tokens(cachedBothWays))).toEqual([
      '1,200 input · 800 output',
      '40,000 cache read · 9,000 cache write',
    ]);
    expect(plain(COST_CARD_COPY.modelDetail('$0.42', cachedBothWays))).toEqual([
      '$0.42 · 1,200 in · 800 out',
      '40,000 cache read · 9,000 cache write',
    ]);
  });
});

describe('the card’s tokens reconcile with the summary’s Tokens value and Settings > App > Usage', () => {
  it('adds up, in the owner’s chat, to the 89k the summary shows', () => {
    const rows = [ownersChat];
    const summary = costSummary({
      modelCostRows: rows,
      inputTokens: 0,
      outputTokens: 0,
      pricing: null,
    });
    expect(summary).not.toBeNull();
    const figures = tokenFigures(COST_CARD_COPY.tokens(summary!));
    // The summary's Tokens value, from the same rows the way BaseChat reads them.
    const billed = mostCompleteBilledTokens(null, rows);
    expect(sum(figures)).toBe(billed);
    expect(summaryTokensValue({ loading: false, ledgerSettled: true, billed })).toBe('89k');
  });

  it('adds up across models, and each model’s row adds up to its billed total', () => {
    const rows = [ownersChat, cachedBothWays];
    const summary = costSummary({
      modelCostRows: rows,
      inputTokens: 0,
      outputTokens: 0,
      pricing: null,
    });
    expect(summary).toMatchObject({ cacheReadTokens: 105_408, cacheCreationTokens: 9_000 });
    expect(sum(tokenFigures(COST_CARD_COPY.tokens(summary!)))).toBe(
      mostCompleteBilledTokens(null, rows)
    );
    for (const row of rows) {
      // `billedTokens` is the Billed figure Settings > App > Usage shows for a model.
      expect(
        sum(tokenFigures(COST_CARD_COPY.modelDetail(formatTooltipMoney(row.totalCost), row)))
      ).toBe(billedTokens(row));
    }
  });

  it('lists what was recorded when a row predates cache accounting, and the total is an estimate', () => {
    // History from before cache accounting: the bucket and the certified total are both unknown.
    const old: ModelCostRow = {
      ...priced,
      cacheReadTokens: null,
      cacheCreationTokens: null,
      totalTokens: null,
      totalCost: 0.5,
      costIsPartial: true,
    };
    const summary = costSummary({
      modelCostRows: [ownersChat, old],
      inputTokens: 0,
      outputTokens: 0,
      pricing: null,
    });
    expect(summary).toMatchObject({
      estimate: { partial: true },
      cacheReadTokens: 65_408,
      cacheCreationTokens: 0,
    });
    expect(plain(COST_CARD_COPY.tokens(summary!))).toEqual([
      '22,888 input · 402 output · 65,408 cache read',
    ]);
    // The old row lists only what it recorded.
    expect(plain(COST_CARD_COPY.modelDetail('$0.50', old))).toEqual(['$0.50 · 100 in · 20 out']);
  });
});
