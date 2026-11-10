import { describe, expect, it } from 'vitest';
import { MODEL_NOT_RECORDED, homeUsageCopy } from './copy';
import { MODEL_NOT_RECORDED as COST_CARD_MODEL_NOT_RECORDED } from '../bottom_menu/copy';
import { modelLabel as costCardModelLabel } from '../bottom_menu/ModelBreakdownTable';
import { modelLabel as settingsUsageModelLabel } from '../settings/usage/UsagePanel';

/**
 * One bucket, one name. Home's usage card, Settings > App > Usage and the composer's cost card
 * all list usage saved without a model (the report's and the ledger's `null` model). They once
 * called it "Not recorded", "unknown" and then "Model not recorded", which reads as three
 * buckets. Each surface must say exactly `MODEL_NOT_RECORDED`, declared once in this module.
 */
describe('the name for usage saved without a model', () => {
  it('is sentence case and is declared once, in the usage copy', () => {
    expect(MODEL_NOT_RECORDED).toBe('Not recorded');
    // The footer's copy re-exports it rather than declaring its own.
    expect(COST_CARD_MODEL_NOT_RECORDED).toBe(MODEL_NOT_RECORDED);
  });

  it("is Home's usage card's name for the bucket", () => {
    expect(homeUsageCopy.notRecorded).toBe(MODEL_NOT_RECORDED);
  });

  it('is what Settings > App > Usage labels a row with no model', () => {
    expect(settingsUsageModelLabel({ modelId: null, provider: null })).toBe(MODEL_NOT_RECORDED);
    expect(settingsUsageModelLabel({ modelId: null, provider: 'zai' })).toBe(MODEL_NOT_RECORDED);
  });

  it("is what the cost card's By model list labels a row with no model", () => {
    expect(costCardModelLabel({ provider: undefined, model: undefined })).toBe(MODEL_NOT_RECORDED);
    expect(costCardModelLabel({ provider: 'openai', model: undefined })).toBe(MODEL_NOT_RECORDED);
  });
});
