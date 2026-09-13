import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  announceSessionRowChanged,
  subscribeSessionRowChanges,
  type SessionRowFacts,
} from './sessionRowSync';

/**
 * Item 11 of the 1.90.4 hold (2026-09-13). Declassifying a chat no longer moves
 * its `updated_at`, and before that change a second window never learned of a
 * declassification at all (measured: History open in two windows, window B kept
 * the private badge for the full 30 seconds watched). This channel is what
 * tells it.
 *
 * Under vitest's jsdom environment `BroadcastChannel` is Node's, so a second
 * `new BroadcastChannel(name)` in the test stands in for another window. The
 * receive path is asynchronous twice over — the delivery, then the re-read — so
 * every assertion WAITS for what it expects rather than ticking the loop once;
 * see `sessionBindingSync.test.ts` for how a single `setTimeout(0)` loses that
 * race.
 */
const CHANNEL_NAME = 'biorouter:session-row';

const mocks = vi.hoisted(() => ({
  getSession: vi.fn(),
}));

vi.mock('../api', () => ({
  getSession: mocks.getSession,
}));

vi.mock('./userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
}));

function row(id: string, privacy_tier: 'public' | 'private', privacy_reason: string | null) {
  return { data: { id, privacy_tier, privacy_reason } };
}

describe('sessionRowSync', () => {
  const unsubscribes: Array<() => void> = [];
  let other: BroadcastChannel;

  beforeEach(() => {
    vi.clearAllMocks();
    other = new BroadcastChannel(CHANNEL_NAME);
  });

  afterEach(() => {
    // The listener set is process-global: a failed assertion must not leave a
    // listener behind for the next test to receive.
    while (unsubscribes.length > 0) unsubscribes.pop()!();
    other.close();
  });

  function listen(): SessionRowFacts[] {
    const seen: SessionRowFacts[] = [];
    unsubscribes.push(subscribeSessionRowChanges((facts) => seen.push(facts)));
    return seen;
  }

  it('re-reads a row another window announced and hands the read to every subscriber', async () => {
    mocks.getSession.mockResolvedValue(row('s1', 'public', 'declassified_by_user'));
    const first = listen();
    const second = listen();

    other.postMessage({ sessionId: 's1' });

    const expected = {
      sessionId: 's1',
      privacy_tier: 'public',
      privacy_reason: 'declassified_by_user',
    };
    await vi.waitFor(() => expect(second).toEqual([expected]));
    expect(first).toEqual([expected]);
    // ONE read for the window, shared by both subscribers, carrying the proof a
    // private chat's row needs.
    expect(mocks.getSession).toHaveBeenCalledTimes(1);
    expect(mocks.getSession).toHaveBeenCalledWith({
      path: { session_id: 's1' },
      query: { metadata_only: true },
      headers: { 'X-User-Action': 'test-proof' },
      throwOnError: true,
    });
  });

  it('delivers what the daemon read, never what the message claimed', async () => {
    // A chat declassified and raised straight back by a turn: the message says
    // nothing about the tier, and if it did it would be the stale half.
    mocks.getSession.mockResolvedValue(row('s2', 'private', 'turn:versa_azure'));
    const seen = listen();

    other.postMessage({ sessionId: 's2', privacy_tier: 'public' });

    await vi.waitFor(() => expect(seen).toHaveLength(1));
    expect(seen[0].privacy_tier).toBe('private');
  });

  it('reads nothing for a malformed message', async () => {
    mocks.getSession.mockImplementation(async ({ path }: { path: { session_id: string } }) =>
      row(path.session_id, 'public', null)
    );
    const seen = listen();

    other.postMessage({});
    other.postMessage(null);
    other.postMessage({ sessionId: 7 });
    other.postMessage({ sessionId: '' });
    // Posted last on the same port, so its arrival proves the four above were
    // already processed.
    other.postMessage({ sessionId: 'barrier' });

    await vi.waitFor(() => expect(seen.map((f) => f.sessionId)).toEqual(['barrier']));
    expect(mocks.getSession).toHaveBeenCalledTimes(1);
  });

  it('does not deliver a read that a newer read of the same chat overtook', async () => {
    let finishOlder: ((value: unknown) => void) | undefined;
    mocks.getSession
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finishOlder = resolve;
          })
      )
      .mockResolvedValueOnce(row('s3', 'public', 'declassified_by_user'));
    const seen = listen();

    announceSessionRowChanged('s3');
    await vi.waitFor(() => expect(mocks.getSession).toHaveBeenCalledTimes(1));
    announceSessionRowChanged('s3');
    await vi.waitFor(() => expect(seen).toHaveLength(1));

    // The older answer lands last, holding the older fact.
    finishOlder!(row('s3', 'private', 'turn:versa_azure'));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(seen).toEqual([
      { sessionId: 's3', privacy_tier: 'public', privacy_reason: 'declassified_by_user' },
    ]);
  });

  it('re-reads in the announcing window too, and a failed read changes nothing', async () => {
    mocks.getSession
      .mockRejectedValueOnce(new Error('403'))
      .mockResolvedValueOnce(row('s4', 'public', 'declassified_by_user'));
    const seen = listen();

    announceSessionRowChanged('s4');
    await vi.waitFor(() => expect(mocks.getSession).toHaveBeenCalledTimes(1));
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(seen).toEqual([]);

    announceSessionRowChanged('s4');
    await vi.waitFor(() => expect(seen).toHaveLength(1));
  });
});
