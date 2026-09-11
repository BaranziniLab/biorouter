import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  clearSessionListCache,
  getCachedSessionList,
  notifySessionListChanged,
  preloadSessionList,
  refreshSessionList,
  subscribeSessionListChanges,
} from './sessionListCache';
import { announceSessionName } from './sessionNameSync';

const mocks = vi.hoisted(() => ({
  listSessions: vi.fn(),
  updateSessionName: vi.fn(),
}));

vi.mock('../api', () => ({
  listSessions: mocks.listSessions,
  updateSessionName: mocks.updateSessionName,
}));

// The proof the desktop sends. Since issue #56's QA sweep (2026-09-10) a list
// request without it is shown no private chat, so every request here must carry
// it — and it arrives one async hop after the call, which is why the assertions
// below wait for `listSessions` rather than expecting it synchronously.
vi.mock('./userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
}));

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
});

describe('sessionListCache', () => {
  it('deduplicates a prefetch and a view load that overlap', async () => {
    let finishRequest: ((value: { data: { sessions: never[] } }) => void) | undefined;
    mocks.listSessions.mockReturnValue(
      new Promise((resolve) => {
        finishRequest = resolve;
      })
    );

    preloadSessionList();
    const viewLoad = refreshSessionList();

    await vi.waitFor(() => expect(mocks.listSessions).toHaveBeenCalledTimes(1));
    expect(mocks.listSessions).toHaveBeenCalledWith(
      expect.objectContaining({ headers: { 'X-User-Action': 'test-proof' } })
    );
    finishRequest?.({ data: { sessions: [] } });
    await viewLoad;
    expect(getCachedSessionList()).toEqual([]);
  });

  it('does not prefetch again after the list is cached', async () => {
    mocks.listSessions.mockResolvedValue({ data: { sessions: [] } });

    await refreshSessionList();
    preloadSessionList();

    expect(mocks.listSessions).toHaveBeenCalledTimes(1);
  });

  it('patches a cached session name when a rename is announced', async () => {
    mocks.listSessions.mockResolvedValue({
      data: { sessions: [{ id: 's1', name: 'Old name', user_set_name: false, working_dir: '/x' }] },
    });
    await refreshSessionList();

    // A rename rides the name channel; the See-all / Home cache must reflect it
    // so those surfaces do not show an old name beside the tab's new one.
    announceSessionName({ sessionId: 's1', name: 'New name', userSetName: true, origin: 'user' });

    expect(getCachedSessionList()?.[0]).toMatchObject({ name: 'New name', user_set_name: true });
  });

  it('a keyless refresh keeps the flagged identity instead of clobbering it', async () => {
    mocks.listSessions.mockResolvedValue({ data: { sessions: [] } });
    await refreshSessionList(true);
    expect(mocks.listSessions).toHaveBeenLastCalledWith(
      expect.objectContaining({ query: { include_subagents: true } })
    );
    mocks.listSessions.mockClear();
    // Home's loader. It has no opinion, so it must not invalidate History's:
    // whatever it re-reads, it re-reads with the identity History asked for.
    // It re-reads (this function has no cache-hit short-circuit — a membership
    // change depends on that), but it must re-read the flagged identity.
    await refreshSessionList();
    expect(mocks.listSessions).toHaveBeenLastCalledWith(
      expect.objectContaining({ query: { include_subagents: true } })
    );
  });

  // BR-71: a flag change orphans the request already in flight, but nothing
  // cancels it. Two things go wrong if the orphan is not fenced off: its answer
  // overwrites the cache (and emits, pushing the wrong-shaped list into every
  // subscriber), and its `.finally` nulls the in-flight slot the *new* request
  // now owns, destroying the dedupe.
  it('a superseded request cannot clobber the list that replaced it', async () => {
    let finishFirst: ((value: { data: { sessions: unknown[] } }) => void) | undefined;
    let finishSecond: ((value: { data: { sessions: unknown[] } }) => void) | undefined;
    mocks.listSessions
      .mockReturnValueOnce(
        new Promise((resolve) => {
          finishFirst = resolve;
        })
      )
      .mockReturnValueOnce(
        new Promise((resolve) => {
          finishSecond = resolve;
        })
      );

    const first = refreshSessionList();
    const second = refreshSessionList(true);
    await vi.waitFor(() => expect(mocks.listSessions).toHaveBeenCalledTimes(2));
    // Each request asks for the list it was issued for, even the orphan whose
    // flag changed during the proof's async hop.
    expect(mocks.listSessions.mock.calls.map(([options]) => options.query)).toEqual([
      { include_subagents: false },
      { include_subagents: true },
    ]);

    finishSecond?.({ data: { sessions: [{ id: 'with-subagents' }] } });
    await second;
    // The orphan settles LAST — the ordering where the stale list is not a
    // flicker but the terminal state.
    finishFirst?.({ data: { sessions: [{ id: 'stale' }] } });
    await first;

    expect(getCachedSessionList()).toEqual([{ id: 'with-subagents' }]);
  });

  it('a superseded request does not free the dedupe slot the new request owns', async () => {
    let finishFirst: ((value: { data: { sessions: unknown[] } }) => void) | undefined;
    let finishSecond: ((value: { data: { sessions: unknown[] } }) => void) | undefined;
    mocks.listSessions
      .mockReturnValueOnce(
        new Promise((resolve) => {
          finishFirst = resolve;
        })
      )
      .mockReturnValueOnce(
        new Promise((resolve) => {
          finishSecond = resolve;
        })
      );

    const first = refreshSessionList();
    const second = refreshSessionList(true);

    finishFirst?.({ data: { sessions: [{ id: 'stale' }] } });
    await first;

    // Nothing of the orphan's reaches the cache.
    expect(getCachedSessionList()).toBeNull();
    // And a membership change joins the live request instead of starting a third.
    notifySessionListChanged();
    expect(mocks.listSessions).toHaveBeenCalledTimes(2);

    finishSecond?.({ data: { sessions: [{ id: 'with-subagents' }] } });
    await second;
    expect(getCachedSessionList()).toEqual([{ id: 'with-subagents' }]);
  });

  it('notifies list-change subscribers and re-reads on a membership change', async () => {
    mocks.listSessions.mockResolvedValue({ data: { sessions: [] } });
    await refreshSessionList();
    mocks.listSessions.mockClear();

    const listener = vi.fn();
    const unsub = subscribeSessionListChanges(listener);

    // A diverge / create / delete announces membership; every list surface
    // re-reads without waiting for an unrelated turn.
    notifySessionListChanged();

    expect(listener).toHaveBeenCalledTimes(1);
    await vi.waitFor(() => expect(mocks.listSessions).toHaveBeenCalled());
    unsub();
  });
});
