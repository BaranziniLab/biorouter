import { describe, expect, it } from 'vitest';
import { WIDE_RUN_EXTRA_CHANNELS } from '../pane/presentation';
import { runSources } from './crewRunStart';

describe('runSources (C20)', () => {
  it('puts the channel it posts in first, then each other channel once', () => {
    expect(runSources('general', ['methods', 'general', 'methods', 'data'])).toEqual([
      'general',
      'methods',
      'data',
    ]);
    expect(runSources('general', [])).toEqual(['general']);
  });

  it('keeps a wide run at 100 sources: the destination and 99 others', () => {
    const others = Array.from({ length: WIDE_RUN_EXTRA_CHANNELS }, (_, index) => `c-${index}`);
    // The destination named again among the others never makes a 101st source.
    const sources = runSources('general', ['general', ...others]);
    expect(sources).toHaveLength(100);
    expect(sources[0]).toBe('general');
  });
});
