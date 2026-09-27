import { describe, expect, it } from 'vitest';
import { makeSnapshot } from './dialogsTestHarness';
import { UNIQUE_NAMES_CAPABILITY, uniqueNamesSupported } from './workspace';

/**
 * Whether Rename is offered: the broker's own word (`unique_names_v1` in its `hello`, carried as
 * the state frame's `capabilities`) wins, and the name handles are read only when it is unknown.
 */
describe('uniqueNamesSupported', () => {
  const withHandles = () => {
    const snapshot = makeSnapshot();
    snapshot.teams[0].handle = 'analysis-lab';
    return snapshot;
  };

  it('offers Rename in a new S2 workspace that has no teams yet', () => {
    const empty = makeSnapshot({ teams: [], channels: [] });
    // Nothing to read a handle from: without the broker's word, Rename is not offered…
    expect(uniqueNamesSupported(empty)).toBe(false);
    expect(uniqueNamesSupported(empty, null)).toBe(false);
    // …and with it, it is.
    expect(uniqueNamesSupported(empty, [UNIQUE_NAMES_CAPABILITY])).toBe(true);
    expect(UNIQUE_NAMES_CAPABILITY).toBe('unique_names_v1');
  });

  it('believes a broker that says it lacks the rules over handles it projects', () => {
    expect(uniqueNamesSupported(withHandles(), ['join_v1'])).toBe(false);
    expect(uniqueNamesSupported(withHandles(), ['join_v1', UNIQUE_NAMES_CAPABILITY])).toBe(true);
  });

  it('reads the handles when the broker’s capabilities are unknown', () => {
    expect(uniqueNamesSupported(withHandles())).toBe(true);
    expect(uniqueNamesSupported(withHandles(), [])).toBe(true);
    expect(uniqueNamesSupported(makeSnapshot(), [])).toBe(false);
  });

  it('offers nothing without a snapshot, whatever the broker says', () => {
    expect(uniqueNamesSupported(null, [UNIQUE_NAMES_CAPABILITY])).toBe(false);
    expect(uniqueNamesSupported(undefined)).toBe(false);
  });
});
