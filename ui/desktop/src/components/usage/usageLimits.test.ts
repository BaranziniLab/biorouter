import { afterEach, describe, expect, it, vi } from 'vitest';
import {
  announceUsageLimitsChanged,
  formatLimitTokens,
  formatLimitUsd,
  limitInputValue,
  parseLimitInput,
  USAGE_LIMITS_CHANGED_EVENT,
} from './usageLimits';

describe('reading a typed limit', () => {
  it.each([
    ['5', 5],
    ['$5', 5],
    ['5.50', 5.5],
    [' 1,250 ', 1250],
    ['.5', 0.5],
    ['0.013', 0.013],
    ['$0.0125', 0.0125],
  ])('reads %j as $%d', (text, value) => {
    expect(parseLimitInput(text, 'usd')).toEqual({ ok: true, value });
  });

  it.each([
    ['250000', 250_000],
    ['250,000', 250_000],
    ['250k', 250_000],
    ['1.5m', 1_500_000],
    ['2M', 2_000_000],
    ['10.4', 10],
  ])('reads %j as %d tokens', (text, value) => {
    expect(parseLimitInput(text, 'tokens')).toEqual({ ok: true, value });
  });

  it.each(['', '0', '-5', 'lots', '5x', '1e3', '$', '0.0'])('refuses %j', (text) => {
    expect(parseLimitInput(text, 'usd').ok).toBe(false);
    expect(parseLimitInput(text, 'tokens').ok).toBe(false);
  });

  it('refuses a dollar amount past four decimals instead of rounding it', () => {
    expect(parseLimitInput('0.01255', 'usd')).toEqual({ ok: false, reason: 'precision' });
    expect(parseLimitInput('0.0125', 'usd')).toEqual({ ok: true, value: 0.0125 });
  });

  it('refuses a token count that rounds to nothing', () => {
    expect(parseLimitInput('0.2', 'tokens').ok).toBe(false);
  });

  it('reads a comma only as a thousands separator, and refuses a decimal comma', () => {
    expect(parseLimitInput('1,000', 'usd')).toEqual({ ok: true, value: 1000 });
    expect(parseLimitInput('$1,250.50', 'usd')).toEqual({ ok: true, value: 1250.5 });
    expect(parseLimitInput('1,000,000', 'usd')).toEqual({ ok: true, value: 1_000_000 });
    expect(parseLimitInput('250,000', 'tokens')).toEqual({ ok: true, value: 250_000 });
    expect(parseLimitInput('1,500k', 'tokens')).toEqual({ ok: true, value: 1_500_000 });
    // A decimal comma is not dropped: "0,50" would otherwise be $50, a hundred times too much.
    for (const text of ['0,50', '5,5', '$0,50', '1,5', '12,34', '1,0000', ',500', '1,000,00']) {
      expect(parseLimitInput(text, 'usd')).toEqual({ ok: false });
    }
    for (const text of ['1,5m', '2,5k', '1,5']) {
      expect(parseLimitInput(text, 'tokens')).toEqual({ ok: false });
    }
  });
});

describe('writing a limit', () => {
  it('formats dollars and tokens as the cards show them', () => {
    expect(formatLimitUsd(5)).toBe('$5.00');
    expect(formatLimitUsd(0.004)).toBe('$0.004');
    expect(formatLimitUsd(0.013)).toBe('$0.013');
    expect(formatLimitTokens(1)).toBe('1 token');
    expect(formatLimitTokens(500_000)).toBe('500,000 tokens');
  });

  it('seeds a field with the stored amount as a person would type it', () => {
    expect(limitInputValue(5, 'usd')).toBe('5');
    expect(limitInputValue(5.256, 'usd')).toBe('5.256');
    expect(limitInputValue(0.013, 'usd')).toBe('0.013');
    expect(limitInputValue(500_000, 'tokens')).toBe('500000');
    expect(limitInputValue(null, 'tokens')).toBe('');
    expect(limitInputValue(0, 'usd')).toBe('');
  });
});

describe('the change announcement', () => {
  afterEach(() => vi.restoreAllMocks());

  it('is one window event carrying the chat and who said it', () => {
    const heard = vi.fn();
    window.addEventListener(USAGE_LIMITS_CHANGED_EVENT, heard);
    const source = {};
    announceUsageLimitsChanged('chat-a', source);
    window.removeEventListener(USAGE_LIMITS_CHANGED_EVENT, heard);
    expect(heard).toHaveBeenCalledTimes(1);
    expect((heard.mock.calls[0][0] as CustomEvent).detail).toEqual({
      sessionId: 'chat-a',
      source,
    });
  });
});
