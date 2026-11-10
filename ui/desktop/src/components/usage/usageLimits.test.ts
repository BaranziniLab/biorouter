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

  it('refuses a token count that rounds to nothing', () => {
    expect(parseLimitInput('0.2', 'tokens').ok).toBe(false);
  });
});

describe('writing a limit', () => {
  it('formats dollars and tokens as the cards show them', () => {
    expect(formatLimitUsd(5)).toBe('$5.00');
    expect(formatLimitUsd(0.004)).toBe('<$0.01');
    expect(formatLimitTokens(1)).toBe('1 token');
    expect(formatLimitTokens(500_000)).toBe('500,000 tokens');
  });

  it('seeds a field with the stored amount as a person would type it', () => {
    expect(limitInputValue(5, 'usd')).toBe('5');
    expect(limitInputValue(5.256, 'usd')).toBe('5.26');
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
