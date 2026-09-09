import { describe, expect, it, vi } from 'vitest';
import type { SessionMetaDelta } from '../api';
import { subscribeToSessionMeta, SESSION_META_CHANGED_EVENT } from './sessionMetaSubscription';

/**
 * Handoff 04 — the renderer's ear on a session row another PROCESS rewrote.
 *
 * The properties under test are the ones whose absence is a dead renderer or a
 * silently stale composer, not the happy path: the loop must never busy-wait on
 * the daemon, must filter to the chats this window shows, must treat a
 * truncated history as an order to refetch, and must recover from a daemon
 * restart rather than parking forever on a revision that no longer exists.
 */

/**
 * A poll queue: each call returns the next scripted answer, then parks forever
 * so the loop cannot spin past the script and swamp the test. Mirrors
 * `catalogSubscription.test.ts`, which drives the same shape of loop.
 */
function scripted(answers: (SessionMetaDelta | Error)[]) {
  const calls: { since: number; ids: string[] }[] = [];
  let i = 0;
  const poll = vi.fn(async (since: number, ids: string[]) => {
    calls.push({ since, ids: [...ids] });
    if (i < answers.length) {
      const answer = answers[i++];
      if (answer instanceof Error) throw answer;
      return answer;
    }
    await new Promise(() => {});
    return undefined;
  });
  return { poll, calls };
}

function delta(revision: number, ids: string[], truncated = false): SessionMetaDelta {
  return {
    revision,
    truncated,
    changes: ids.map((id, i) => ({
      revision: revision - ids.length + i + 1,
      session_id: id,
      provider_name: 'versa_azure',
      model_name: 'gpt-5.5-2026-04-24',
      privacy_tier: 'private',
      privacy_reason: 'turn:versa_azure',
    })),
  };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 0));

/**
 * The loop keeps a small floor under one iteration (`MIN_INTERVAL_MS`) so a
 * subscription answered without parking cannot become a busy wait on the
 * daemon. These tests are about SEQUENCING, so they hand it a sleep that
 * resolves at once rather than sitting through the floor or encoding its value.
 */
const immediate = () => Promise.resolve();

describe('sessionMetaSubscription', () => {
  it('reports only the chats this window has open', async () => {
    // The ring is process-wide, so a delta legitimately names a chat another
    // window is watching. Waking this window for one it does not show would
    // refetch a row it holds no copy of.
    const { poll } = scripted([delta(3, ['mine', 'someone-elses'])]);
    const changed: string[] = [];
    const stop = subscribeToSessionMeta({
      openSessionIds: () => ['mine'],
      onSessionChanged: (id) => changed.push(id),
      poll,
      sleep: immediate,
    });
    await flush();
    await flush();
    stop();

    expect(changed).toEqual(['mine']);
  });

  it('carries the open ids to the daemon and advances the cursor', async () => {
    const { poll, calls } = scripted([delta(4, ['a']), delta(9, ['b'])]);
    const stop = subscribeToSessionMeta({
      openSessionIds: () => ['a', 'b'],
      onSessionChanged: () => {},
      poll,
      sleep: immediate,
    });
    await flush();
    await flush();
    stop();

    expect(calls[0]).toEqual({ since: 0, ids: ['a', 'b'] });
    // The second poll resumes from the revision the first returned, which is
    // what lets the daemon PARK it instead of answering immediately.
    expect(calls[1].since).toBe(4);
  });

  /**
   * ⚠ `truncated` is an order, not a warning. The history handed back is
   * partial, so believing it and moving on is the stale-row bug one layer down.
   */
  it('re-reads every open chat when the history was truncated', async () => {
    const { poll } = scripted([delta(50, [], true)]);
    const changed: string[] = [];
    const stop = subscribeToSessionMeta({
      openSessionIds: () => ['a', 'b', 'c'],
      onSessionChanged: (id) => changed.push(id),
      poll,
      sleep: immediate,
    });
    await flush();
    await flush();
    stop();

    expect(changed.sort()).toEqual(['a', 'b', 'c']);
  });

  /**
   * A daemon restart resets the revision to 0. Nothing was undone — our cursor
   * is simply meaningless, and holding it would park us forever on a number the
   * daemon will take a long time to climb back to.
   */
  it('re-reads everything when the daemon`s revision goes backwards', async () => {
    const { poll } = scripted([delta(12, ['a']), { revision: 1, changes: [] }]);
    const changed: string[] = [];
    const stop = subscribeToSessionMeta({
      openSessionIds: () => ['a', 'b'],
      onSessionChanged: (id) => changed.push(id),
      poll,
      sleep: immediate,
    });
    await flush();
    await flush();
    await flush();
    stop();

    // First the real change, then both chats because the cursor is worthless.
    expect(changed).toEqual(['a', 'a', 'b']);
  });

  it('says nothing when the revision did not move', async () => {
    const { poll } = scripted([{ revision: 0, changes: [] }]);
    const changed: string[] = [];
    const stop = subscribeToSessionMeta({
      openSessionIds: () => ['a'],
      onSessionChanged: (id) => changed.push(id),
      poll,
      sleep: immediate,
    });
    await flush();
    await flush();
    stop();

    expect(changed).toEqual([]);
  });

  /**
   * ⚠ The blast-radius property. A window sitting on Settings has no chats
   * open; the daemon would answer such a poll immediately, so a loop without a
   * local sleep would spin as fast as the loopback carries it and claim all six
   * of Chromium's sockets to the host.
   */
  it('never asks the daemon anything when no chat is open', async () => {
    const { poll } = scripted([{ revision: 0, changes: [] }]);
    let slept = 0;
    const stop = subscribeToSessionMeta({
      openSessionIds: () => [],
      onSessionChanged: () => {},
      poll,
      sleep: async () => {
        slept += 1;
        if (slept > 3) stop();
      },
    });
    await flush();
    await flush();
    stop();

    expect(poll).not.toHaveBeenCalled();
    expect(slept).toBeGreaterThan(0);
  });

  it('backs off instead of spinning when a poll throws', async () => {
    const { poll } = scripted([new Error('daemon restarting'), delta(2, ['a'])]);
    const sleeps: number[] = [];
    const changed: string[] = [];
    const stop = subscribeToSessionMeta({
      openSessionIds: () => ['a'],
      onSessionChanged: (id) => changed.push(id),
      poll,
      sleep: async (ms) => {
        sleeps.push(ms);
      },
      retryDelayMs: 1234,
    });
    await flush();
    await flush();
    await flush();
    stop();

    expect(sleeps[0]).toBe(1234);
    expect(changed).toEqual(['a']);
  });

  it('stops polling once it is torn down', async () => {
    const { poll } = scripted([delta(1, ['a']), delta(2, ['a']), delta(3, ['a'])]);
    const stop = subscribeToSessionMeta({
      openSessionIds: () => ['a'],
      onSessionChanged: () => {},
      poll,
      sleep: immediate,
    });
    await flush();
    stop();
    const after = poll.mock.calls.length;
    await flush();
    await flush();

    expect(poll.mock.calls.length).toBe(after);
  });

  it('announces changed ids on the window, for non-React readers', async () => {
    const { poll } = scripted([delta(3, ['a'])]);
    const seen: string[][] = [];
    const listener = (event: Event) => {
      seen.push((event as CustomEvent<{ sessionIds: string[] }>).detail.sessionIds);
    };
    window.addEventListener(SESSION_META_CHANGED_EVENT, listener);
    const stop = subscribeToSessionMeta({
      openSessionIds: () => ['a'],
      onSessionChanged: () => {},
      poll,
      sleep: immediate,
    });
    await flush();
    await flush();
    stop();
    window.removeEventListener(SESSION_META_CHANGED_EVENT, listener);

    expect(seen).toEqual([['a']]);
  });
});
