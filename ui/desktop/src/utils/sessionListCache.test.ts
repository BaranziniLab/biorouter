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
import { announceSessionRowChanged } from './sessionRowSync';

const mocks = vi.hoisted(() => ({
  listSessions: vi.fn(),
  updateSessionName: vi.fn(),
  getSession: vi.fn(),
}));

vi.mock('../api', () => ({
  listSessions: mocks.listSessions,
  updateSessionName: mocks.updateSessionName,
  getSession: mocks.getSession,
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

  /**
   * A list response describes the moment it was ISSUED. A rename announced after
   * that is the LATER fact, and replacing the whole array would undo it — the
   * snap-back `sessionNameSync`'s header describes.
   *
   * It stopped being cosmetic when the tab strip started reconciling its titles
   * against this cache: a clobbered name is written onto a tab and persisted
   * there. And the window is the common one — a new chat's first turn issues a
   * list refresh (`refreshSessionBinding` → `notifySessionListChanged`) and,
   * ~800 ms later, announces the name the daemon generated.
   */
  it('keeps a rename announced while the list request was still in flight', async () => {
    let finishRequest: ((value: { data: { sessions: unknown[] } }) => void) | undefined;
    mocks.listSessions.mockReturnValue(
      new Promise((resolve) => {
        finishRequest = resolve;
      })
    );

    const inFlight = refreshSessionList();
    await vi.waitFor(() => expect(mocks.listSessions).toHaveBeenCalledTimes(1));

    // The daemon's auto-name lands while the (slow) list read is still open.
    announceSessionName({
      sessionId: 's1',
      name: 'Penguin prompt test',
      userSetName: false,
      origin: 'llm',
    });

    // …and the response, which was issued before it, still says the old name.
    finishRequest?.({
      data: {
        sessions: [{ id: 's1', name: 'New chat', user_set_name: false, working_dir: '/x' }],
      },
    });
    await inFlight;

    expect(getCachedSessionList()?.[0]).toMatchObject({
      name: 'Penguin prompt test',
      user_set_name: false,
    });
  });

  /** Only for the request it raced. The NEXT read is the later fact and wins. */
  it('lets a later list read overwrite a name it raced once', async () => {
    let finishRequest: ((value: { data: { sessions: unknown[] } }) => void) | undefined;
    mocks.listSessions.mockReturnValue(
      new Promise((resolve) => {
        finishRequest = resolve;
      })
    );
    const inFlight = refreshSessionList();
    await vi.waitFor(() => expect(mocks.listSessions).toHaveBeenCalledTimes(1));
    announceSessionName({ sessionId: 's1', name: 'Raced', userSetName: false, origin: 'llm' });
    finishRequest?.({
      data: { sessions: [{ id: 's1', name: 'Stale', user_set_name: false, working_dir: '/x' }] },
    });
    await inFlight;

    mocks.listSessions.mockResolvedValue({
      data: {
        sessions: [{ id: 's1', name: 'Renamed elsewhere', user_set_name: true, working_dir: '/x' }],
      },
    });
    await refreshSessionList();

    expect(getCachedSessionList()?.[0]).toMatchObject({ name: 'Renamed elsewhere' });
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

  // Item 11 of the 1.90.4 hold (2026-09-13). A declassification no longer
  // re-sorts the chat, so nothing about the LIST changes and no refetch would
  // notice. History's badge and its row menu read this entry, so it is patched
  // where it sits — from the daemon's read, not from the announcement.
  it('re-marks a declassified chat in place, without refetching or reordering', async () => {
    mocks.listSessions.mockResolvedValue({
      data: {
        sessions: [
          { id: 'recent', privacy_tier: 'private', privacy_reason: 'turn:versa_azure' },
          { id: 'old', privacy_tier: 'private', privacy_reason: 'backfill:ollama' },
        ],
      },
    });
    await refreshSessionList();
    mocks.listSessions.mockClear();
    mocks.getSession.mockResolvedValue({
      data: { id: 'old', privacy_tier: 'public', privacy_reason: 'declassified_by_user' },
    });

    announceSessionRowChanged('old');

    await vi.waitFor(() =>
      expect(getCachedSessionList()).toEqual([
        { id: 'recent', privacy_tier: 'private', privacy_reason: 'turn:versa_azure' },
        { id: 'old', privacy_tier: 'public', privacy_reason: 'declassified_by_user' },
      ])
    );
    expect(mocks.listSessions).not.toHaveBeenCalled();
  });

  /**
   * Defect D4 of the 2026-09-13 repair round, the list half. A list request
   * issued BEFORE a turn raised a chat can answer AFTER the raise was read and
   * patched in; adopting the answer drew the chat public again. Neither reading
   * is known to be the later one, so the row shows the higher tier and is read
   * a third time.
   */
  it('a list answer that raced a raise does not draw the chat public again', async () => {
    mocks.listSessions.mockResolvedValueOnce({
      data: { sessions: [{ id: 'raced', privacy_tier: 'public', privacy_reason: null }] },
    });
    await refreshSessionList();

    let answerList: ((value: unknown) => void) | undefined;
    mocks.listSessions.mockReturnValueOnce(
      new Promise((resolve) => {
        answerList = resolve;
      })
    );
    const refresh = refreshSessionList();
    await vi.waitFor(() => expect(mocks.listSessions).toHaveBeenCalledTimes(2));

    // The raise, announced by the chat's store and read while the list is out.
    mocks.getSession.mockResolvedValue({
      data: { id: 'raced', privacy_tier: 'private', privacy_reason: 'turn:versa_azure' },
    });
    announceSessionRowChanged('raced');
    await vi.waitFor(() =>
      expect(getCachedSessionList()?.[0]).toMatchObject({ privacy_tier: 'private' })
    );
    expect(mocks.getSession).toHaveBeenCalledTimes(1);

    // The list answers with what it saw before the raise. The read that
    // settles it is held open, so what the cache shows in the meantime is
    // observable rather than overwritten a microtask later.
    let answerThirdRead: ((value: unknown) => void) | undefined;
    mocks.getSession.mockReturnValueOnce(
      new Promise((resolve) => {
        answerThirdRead = resolve;
      })
    );
    answerList!({
      data: { sessions: [{ id: 'raced', privacy_tier: 'public', privacy_reason: null }] },
    });
    await refresh;

    // The disagreement is settled by a read issued after both…
    await vi.waitFor(() => expect(mocks.getSession).toHaveBeenCalledTimes(2));
    // …and until it lands the chat is not drawn public.
    expect(getCachedSessionList()?.[0]).toMatchObject({
      privacy_tier: 'private',
      privacy_reason: 'turn:versa_azure',
    });
    answerThirdRead!({
      data: { id: 'raced', privacy_tier: 'private', privacy_reason: 'turn:versa_azure' },
    });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(getCachedSessionList()?.[0]).toMatchObject({ privacy_tier: 'private' });
  });

  it('a list answer that raced a declassification is settled by a third read', async () => {
    mocks.listSessions.mockResolvedValueOnce({
      data: { sessions: [{ id: 'lowered', privacy_tier: 'private', privacy_reason: 'turn:x' }] },
    });
    await refreshSessionList();

    let answerList: ((value: unknown) => void) | undefined;
    mocks.listSessions.mockReturnValueOnce(
      new Promise((resolve) => {
        answerList = resolve;
      })
    );
    const refresh = refreshSessionList();
    await vi.waitFor(() => expect(mocks.listSessions).toHaveBeenCalledTimes(2));

    mocks.getSession.mockResolvedValue({
      data: { id: 'lowered', privacy_tier: 'public', privacy_reason: 'declassified_by_user' },
    });
    announceSessionRowChanged('lowered');
    await vi.waitFor(() =>
      expect(getCachedSessionList()?.[0]).toMatchObject({ privacy_tier: 'public' })
    );

    let answerThirdRead: ((value: unknown) => void) | undefined;
    mocks.getSession.mockReturnValueOnce(
      new Promise((resolve) => {
        answerThirdRead = resolve;
      })
    );
    answerList!({
      data: { sessions: [{ id: 'lowered', privacy_tier: 'private', privacy_reason: 'turn:x' }] },
    });
    await refresh;
    // Private until the order is known — never public on a guess…
    await vi.waitFor(() => expect(mocks.getSession).toHaveBeenCalledTimes(2));
    expect(getCachedSessionList()?.[0]).toMatchObject({ privacy_tier: 'private' });
    // …and public once a read issued after both says so.
    answerThirdRead!({
      data: { id: 'lowered', privacy_tier: 'public', privacy_reason: 'declassified_by_user' },
    });
    await vi.waitFor(() =>
      expect(getCachedSessionList()?.[0]).toMatchObject({
        privacy_tier: 'public',
        privacy_reason: 'declassified_by_user',
      })
    );
  });
});
