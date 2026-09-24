import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  DRAFT_STASH_MAX_BODY_BYTES,
  DRAFT_STASH_MAX_ENTRIES,
  LAST_CHANNEL_STORAGE_PREFIX,
  forgetConnectionDrafts,
  forgetLastChannel,
  forgetStashedDraft,
  rememberLastChannel,
  rememberedLastChannel,
  resetDraftStashForTests,
  stashDraft,
  stashedDraft,
  stashedDraftCount,
  takeStashedDraft,
} from './draftStash';
import type { DraftScope } from './observationFailure';

/**
 * The draft stash (live QA round 2, Q2-07) and the last channel (Q2-21). SECURITY-SENSITIVE: the
 * stash keeps an unsent body only, in memory only, only with the verified scope of its own
 * channel, and bounded.
 */

function scope(channelId: string, connectionId = 'conn-1'): DraftScope {
  return {
    connectionId,
    workspaceMode: 'private',
    workspaceInstitution: 'ucsf',
    connectionMode: 'private',
    connectionEpoch: 1,
    connectionInstitution: 'ucsf',
    channel: { id: channelId, classification: 'restricted' },
    sources: new Map(),
  };
}

beforeEach(() => {
  resetDraftStashForTests();
  window.localStorage.clear();
});

describe('the draft stash', () => {
  it('keeps a body with its scope, and hands it back once', () => {
    stashDraft('conn-1', 'general', 'half-written', scope('general'));
    expect(stashedDraft('conn-1', 'general')).toEqual({
      body: 'half-written',
      scope: scope('general'),
    });
    expect(takeStashedDraft('conn-1', 'general')?.body).toBe('half-written');
    expect(takeStashedDraft('conn-1', 'general')).toBeUndefined();
  });

  it('keeps one draft per connection and channel', () => {
    stashDraft('conn-1', 'general', 'one', scope('general'));
    stashDraft('conn-1', 'methods', 'two', scope('methods'));
    stashDraft('conn-2', 'general', 'three', scope('general', 'conn-2'));
    stashDraft('conn-1', 'general', 'one, edited', scope('general'));
    expect(stashedDraftCount()).toBe(3);
    expect(stashedDraft('conn-1', 'general')?.body).toBe('one, edited');
    expect(stashedDraft('conn-2', 'general')?.body).toBe('three');
  });

  it('keeps nothing, and touches nothing kept, for an empty body', () => {
    stashDraft('conn-1', 'general', 'kept', scope('general'));
    stashDraft('conn-1', 'general', '   ', scope('general'));
    expect(stashedDraft('conn-1', 'general')?.body).toBe('kept');
  });

  it('keeps nothing without the verified scope of that very channel and connection', () => {
    stashDraft('conn-1', 'general', 'no scope', null);
    stashDraft('conn-1', 'general', 'another channel’s scope', scope('methods'));
    stashDraft('conn-1', 'general', 'another connection’s scope', scope('general', 'conn-2'));
    stashDraft('', 'general', 'no connection', scope('general', ''));
    stashDraft('conn-1', '', 'no channel', { ...scope(''), channel: null });
    expect(stashedDraftCount()).toBe(0);
  });

  it('never keeps an attachment, a reference or a context channel: only the body and scope', () => {
    stashDraft('conn-1', 'general', 'body', scope('general'));
    expect(Object.keys(stashedDraft('conn-1', 'general') ?? {}).sort()).toEqual(['body', 'scope']);
  });

  it('drops a body over the size bound rather than cutting it, and the older draft with it', () => {
    stashDraft('conn-1', 'general', 'short', scope('general'));
    const tooLong = 'é'.repeat(DRAFT_STASH_MAX_BODY_BYTES / 2 + 1); // two bytes each in UTF-8
    stashDraft('conn-1', 'general', tooLong, scope('general'));
    expect(stashedDraft('conn-1', 'general')).toBeUndefined();

    const fits = 'a'.repeat(DRAFT_STASH_MAX_BODY_BYTES);
    stashDraft('conn-1', 'general', fits, scope('general'));
    expect(stashedDraft('conn-1', 'general')?.body).toHaveLength(DRAFT_STASH_MAX_BODY_BYTES);
  });

  it('keeps at most 50 drafts, dropping the oldest first', () => {
    for (let index = 0; index <= DRAFT_STASH_MAX_ENTRIES; index += 1)
      stashDraft('conn-1', `channel-${index}`, `draft ${index}`, scope(`channel-${index}`));
    expect(stashedDraftCount()).toBe(DRAFT_STASH_MAX_ENTRIES);
    expect(stashedDraft('conn-1', 'channel-0')).toBeUndefined();
    expect(stashedDraft('conn-1', `channel-${DRAFT_STASH_MAX_ENTRIES}`)?.body).toBe(
      `draft ${DRAFT_STASH_MAX_ENTRIES}`
    );
    // Stashing again makes a draft the newest, so it is not the next to go.
    stashDraft('conn-1', 'channel-1', 'draft 1, again', scope('channel-1'));
    stashDraft('conn-1', 'channel-new', 'newest', scope('channel-new'));
    expect(stashedDraft('conn-1', 'channel-1')?.body).toBe('draft 1, again');
    expect(stashedDraft('conn-1', 'channel-2')).toBeUndefined();
  });

  it('forgets one channel, one connection, or the channels a view no longer offers', () => {
    stashDraft('conn-1', 'general', 'a', scope('general'));
    stashDraft('conn-1', 'methods', 'b', scope('methods'));
    stashDraft('conn-1', 'secret', 'c', scope('secret'));
    stashDraft('conn-2', 'general', 'd', scope('general', 'conn-2'));
    stashDraft('conn-10', 'general', 'e', scope('general', 'conn-10'));

    forgetStashedDraft('conn-1', 'general');
    expect(stashedDraft('conn-1', 'general')).toBeUndefined();

    forgetConnectionDrafts('conn-1', (channel) => channel === 'methods');
    expect(stashedDraft('conn-1', 'methods')?.body).toBe('b');
    expect(stashedDraft('conn-1', 'secret')).toBeUndefined();

    forgetConnectionDrafts('conn-1');
    expect(stashedDraft('conn-1', 'methods')).toBeUndefined();
    // Another connection's drafts, even one whose ID starts the same, are untouched.
    expect(stashedDraft('conn-2', 'general')?.body).toBe('d');
    expect(stashedDraft('conn-10', 'general')?.body).toBe('e');
  });

  it('is memory only: nothing reaches storage', () => {
    const setItem = vi.spyOn(window.localStorage, 'setItem');
    stashDraft('conn-1', 'general', 'private words', scope('general'));
    expect(setItem).not.toHaveBeenCalled();
    expect(window.localStorage.length).toBe(0);
    setItem.mockRestore();
  });
});

describe('the last channel', () => {
  it('is remembered per connection, in memory and in storage', () => {
    rememberLastChannel('conn-1', 'methods');
    rememberLastChannel('conn-2', 'general');
    expect(rememberedLastChannel('conn-1')).toBe('methods');
    expect(rememberedLastChannel('conn-2')).toBe('general');
    expect(window.localStorage.getItem(`${LAST_CHANNEL_STORAGE_PREFIX}conn-1`)).toBe('methods');
  });

  it('comes back from storage after the app restarts (the memory is empty)', () => {
    window.localStorage.setItem(`${LAST_CHANNEL_STORAGE_PREFIX}conn-1`, 'methods');
    expect(rememberedLastChannel('conn-1')).toBe('methods');
    window.localStorage.setItem(`${LAST_CHANNEL_STORAGE_PREFIX}conn-1`, 'x'.repeat(500));
    expect(rememberedLastChannel('conn-1')).toBeNull();
  });

  it('still serves this app session when storage refuses every access', () => {
    const setItem = vi.spyOn(window.localStorage, 'setItem').mockImplementation(() => {
      throw new Error('denied');
    });
    const getItem = vi.spyOn(window.localStorage, 'getItem').mockImplementation(() => {
      throw new Error('denied');
    });
    rememberLastChannel('conn-1', 'methods');
    expect(rememberedLastChannel('conn-1')).toBe('methods');
    expect(rememberedLastChannel('conn-2')).toBeNull();
    setItem.mockRestore();
    getItem.mockRestore();
  });

  it('is forgotten with its connection', () => {
    rememberLastChannel('conn-1', 'methods');
    forgetLastChannel('conn-1');
    expect(rememberedLastChannel('conn-1')).toBeNull();
    expect(window.localStorage.getItem(`${LAST_CHANNEL_STORAGE_PREFIX}conn-1`)).toBeNull();
  });
});
