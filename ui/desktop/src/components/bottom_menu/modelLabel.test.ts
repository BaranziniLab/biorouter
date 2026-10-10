import { describe, expect, it } from 'vitest';
import { friendlyModelName } from './modelLabel';

describe('friendlyModelName', () => {
  it.each([
    ['gpt-5.6-sol-2026-07-09', 'gpt-5.6-sol'],
    ['gpt-5.5-2026-04-24', 'gpt-5.5'],
    ['claude-3-5-sonnet-20241022', 'claude-3-5-sonnet'],
    ['claude-opus-5', 'claude-opus-5'],
    ['gemma4-12b', 'gemma4-12b'],
    ['owner/repo:Q4_K_M', 'owner/repo:Q4_K_M'],
    ['gpt-4-0613', 'gpt-4-0613'],
    ['2026-07-09', '2026-07-09'],
    ['', ''],
  ])('%s -> %s', (id, name) => {
    expect(friendlyModelName(id)).toBe(name);
  });
});
