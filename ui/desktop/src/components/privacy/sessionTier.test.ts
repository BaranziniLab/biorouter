import { describe, expect, it } from 'vitest';
import { mergeSessionTiers, raiseTier, sessionTiersDiffer } from './sessionTier';

/**
 * The one rule every surface that draws a tier depends on: two readings of the
 * same chat combine with `max`, never with "whichever is fresher".
 *
 * The safe direction is asserted directly rather than left as a property of the
 * caller — a merge that let a stale `public` win is finding M8 in one line, and
 * a merge that invented `private` for an unknown chat would be the same lie
 * pointing the other way.
 */
describe('raiseTier — max over public < private', () => {
  it('raises public to private', () => {
    expect(raiseTier('public', 'private')).toBe('private');
  });

  it('never lowers private back to public, whichever side the public is on', () => {
    expect(raiseTier('private', 'public')).toBe('private');
    expect(raiseTier('public', 'private')).toBe('private');
  });

  it('keeps private when the other side has no opinion', () => {
    expect(raiseTier('private', undefined)).toBe('private');
    expect(raiseTier(undefined, 'private')).toBe('private');
  });

  it('leaves an id nobody has read unmarked, rather than calling it public', () => {
    expect(raiseTier(undefined, undefined)).toBeUndefined();
  });

  it('keeps public when that is all either side has', () => {
    expect(raiseTier('public', 'public')).toBe('public');
    expect(raiseTier(undefined, 'public')).toBe('public');
  });
});

describe('mergeSessionTiers — folding a live map over a cached one', () => {
  it('takes private from the LIVE source over a stale cached public — finding M8', () => {
    const cached = { chat: 'public' } as const;
    const live = { chat: 'private' } as const;
    expect(mergeSessionTiers(cached, live)).toEqual({ chat: 'private' });
  });

  it('takes private from the CACHED source when the live store has not seen it', () => {
    // A tab never opened in this window has no store, so the list is the only
    // source it has — and it must still be marked.
    expect(mergeSessionTiers({ chat: 'private' }, {})).toEqual({ chat: 'private' });
  });

  it('is order-independent, so neither source is privileged', () => {
    const cached = { a: 'public', b: 'private' } as const;
    const live = { a: 'private', b: 'public' } as const;
    expect(mergeSessionTiers(cached, live)).toEqual(mergeSessionTiers(live, cached));
    expect(mergeSessionTiers(cached, live)).toEqual({ a: 'private', b: 'private' });
  });

  it('omits an id no source has an opinion about', () => {
    expect(mergeSessionTiers({}, {})).toEqual({});
    expect(mergeSessionTiers(undefined, null)).toEqual({});
  });

  it('carries ids that appear in only one source', () => {
    expect(mergeSessionTiers({ a: 'public' }, { b: 'private' })).toEqual({
      a: 'public',
      b: 'private',
    });
  });
});

describe('sessionTiersDiffer — the identity-stability test', () => {
  it('is false for equal maps, so nothing re-renders', () => {
    expect(sessionTiersDiffer({ a: 'private' }, { a: 'private' })).toBe(false);
    expect(sessionTiersDiffer({}, {})).toBe(false);
  });

  it('is true when a tier moved', () => {
    expect(sessionTiersDiffer({ a: 'public' }, { a: 'private' })).toBe(true);
  });

  it('is true when the SET of ids changed, both ways', () => {
    expect(sessionTiersDiffer({}, { a: 'public' })).toBe(true);
    expect(sessionTiersDiffer({ a: 'public' }, {})).toBe(true);
  });

  it('is true for two same-sized maps that name different chats', () => {
    // The cheap test — comparing lengths — passes here, so the per-key walk is
    // what makes this correct rather than merely fast.
    expect(sessionTiersDiffer({ a: 'private' }, { b: 'private' })).toBe(true);
  });
});
