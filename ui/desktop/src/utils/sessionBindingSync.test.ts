import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  announceAppModelSelection,
  announceSessionBinding,
  subscribeAppModelSelectionChanges,
  subscribeSessionBindingChanges,
  type SessionBindingChange,
} from './sessionBindingSync';

/**
 * Handoff 04 — a per-chat model switch made in one window has to reach the
 * others.
 *
 * Both windows are the same origin in one Electron app, so `BroadcastChannel`
 * carries it, exactly as `sessionNameSync` already carries a rename. The
 * properties worth pinning are the two that decide whether a receiver can trust
 * what it is handed: local listeners run SYNCHRONOUSLY (the announcing window is
 * mid-switch and must not render between the write landing and knowing about
 * it), and a malformed message from another window is dropped rather than used
 * to patch a row with `undefined`.
 */
const CHANNEL_NAME = 'biorouter:session-binding';

/**
 * A well-formed message whose only job is to be delivered LAST.
 *
 * ⚠ **`await new Promise((r) => setTimeout(r, 0))` is not a delivery barrier
 * for a `BroadcastChannel`, and believing it was is what made these two tests
 * flaky together.** Under vitest's jsdom environment the channel is Node's, so
 * a message is delivered from the libuv POLL phase while `setTimeout(0)` —
 * clamped to 1ms — fires from the TIMERS phase, which runs first in a loop
 * turn. Measured ordering in this environment:
 *
 *     sync > microtask > nextTick > setImmediate > DELIVERED > timeout0
 *
 * so the tick normally loses to the delivery and the tests normally pass. But
 * the timer is armed immediately after `postMessage`, and if >=1ms of wall clock
 * passes before the loop next turns — one descheduled slice on a loaded runner,
 * one GC pause — the timer is already due and the timers phase fires it AHEAD of
 * the delivery. The test then asserts on an empty `seen`, the message lands a
 * moment later, and because the module's listener set is process-global it is
 * handed to whatever listener the NEXT test has just subscribed. That is the
 * signature failure: 'delivers a binding another window announced' fails empty
 * and 'drops a malformed message' fails holding the previous test's `s3`.
 * Reproduced deterministically by burning 3ms between arming the tick and
 * awaiting it.
 *
 * So wait for delivery itself. One channel is one MessagePort and a port is
 * FIFO, so a barrier posted after the payloads is delivered after them: its
 * arrival proves every earlier message has already been processed — including
 * the malformed ones that are supposed to leave no trace.
 */
const BARRIER = {
  sessionId: '__barrier__',
  provider: '__barrier__',
  model: '__barrier__',
} as const;

/** Resolve once everything already posted on `other` has been delivered. */
function drain(other: BroadcastChannel): Promise<void> {
  return new Promise((resolve) => {
    const stop = subscribeSessionBindingChanges((change) => {
      if (change.sessionId !== BARRIER.sessionId) return;
      stop();
      resolve();
    });
    other.postMessage(BARRIER);
  });
}

/** Everything a test's listener saw, minus the barrier it is not about. */
function payloadsOnly(listener: (change: SessionBindingChange) => void) {
  return (change: SessionBindingChange) => {
    if (change.sessionId !== BARRIER.sessionId) listener(change);
  };
}

describe('sessionBindingSync', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  /**
   * Belt and braces: every test above drains its own posts, so nothing should
   * be in flight — but a test that fails an assertion returns early, and a
   * message that leaked into the next test is the failure this file exists to
   * stop reproducing. One more barrier round trip guarantees the channel is
   * quiet before the next test subscribes.
   */
  afterEach(async () => {
    const sweeper = new BroadcastChannel(CHANNEL_NAME);
    try {
      await drain(sweeper);
    } finally {
      sweeper.close();
    }
  });

  it('calls local listeners synchronously, before the announcement returns', () => {
    const seen: string[] = [];
    const unsubscribe = subscribeSessionBindingChanges((change) => seen.push(change.model));

    announceSessionBinding({
      sessionId: 's1',
      provider: 'versa_azure',
      model: 'gpt-5.5-2026-04-24',
      contextLimit: 1_050_000,
    });

    // No await: if this needed a tick, a render could land between the daemon
    // accepting the bind and this window knowing about it — which is the stale
    // flash the announcement exists to prevent.
    expect(seen).toEqual(['gpt-5.5-2026-04-24']);
    unsubscribe();
  });

  it('posts the change to the other windows', () => {
    const posted: unknown[] = [];
    const post = vi
      .spyOn(BroadcastChannel.prototype, 'postMessage')
      .mockImplementation((message: unknown) => {
        posted.push(message);
      });

    const unsubscribe = subscribeSessionBindingChanges(() => {});
    announceSessionBinding({ sessionId: 's2', provider: 'codex', model: 'gpt-6-astra' });
    unsubscribe();

    expect(posted).toEqual([{ sessionId: 's2', provider: 'codex', model: 'gpt-6-astra' }]);
    post.mockRestore();
  });

  it('delivers a binding another window announced', async () => {
    const seen: string[] = [];
    const unsubscribe = subscribeSessionBindingChanges(
      payloadsOnly((change) => seen.push(`${change.sessionId}:${change.model}`))
    );

    // A second window, speaking on the same channel.
    const other = new BroadcastChannel(CHANNEL_NAME);
    try {
      other.postMessage({ sessionId: 's3', provider: 'versa_azure', model: 'gpt-5.2-2025-12-11' });
      await drain(other);

      expect(seen).toEqual(['s3:gpt-5.2-2025-12-11']);
    } finally {
      // In a `finally` so a failed assertion cannot leave a listener subscribed
      // to a process-global set that the next test also subscribes to.
      other.close();
      unsubscribe();
    }
  });

  /**
   * ⚠ Shape-checked, not trusted. This arrives from another window, and a
   * message missing `provider` or `model` would otherwise patch a session row
   * with `undefined` — a row naming no model at all, which every reader of the
   * binding then states.
   */
  it('drops a malformed message rather than patching a row from it', async () => {
    const seen: unknown[] = [];
    const unsubscribe = subscribeSessionBindingChanges(payloadsOnly((change) => seen.push(change)));

    const other = new BroadcastChannel(CHANNEL_NAME);
    try {
      other.postMessage({ sessionId: 's4' });
      other.postMessage({ provider: 'versa_azure', model: 'gpt-5.5-2026-04-24' });
      other.postMessage(null);
      // The barrier is posted on the SAME channel, after the three malformed
      // messages, so one FIFO port guarantees they were all processed by the
      // time it arrives. Without that, an empty `seen` would equally mean
      // "nothing has been delivered yet" — which is how this assertion used to
      // pass for the wrong reason and then fail holding another test's payload.
      await drain(other);

      expect(seen).toEqual([]);
    } finally {
      other.close();
      unsubscribe();
    }
  });
});

/**
 * F3 (provider QA, 2026-09-10). The app-wide selection — the pair `/agent/start`
 * binds a new chat to — crosses windows on the same channel as the binding, and
 * the difference between the two messages is the whole design: a binding is a
 * fact about one row, the selection announcement is a NUDGE to re-read. See
 * "The second fact crosses too" in the module header.
 */
describe('sessionBindingSync — the app-wide selection', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
  });

  it('wakes local listeners synchronously, so the writing window re-reads too', () => {
    let woken = 0;
    const unsubscribe = subscribeAppModelSelectionChanges(() => {
      woken += 1;
    });

    announceAppModelSelection();

    expect(woken).toBe(1);
    unsubscribe();
  });

  /**
   * ⚠ A kind and nothing else. A receiver that could read a provider and a
   * model off the message would eventually apply them, and two windows' writes
   * can be announced in the opposite order from the one they landed in.
   */
  it('posts a nudge carrying no provider and no model', () => {
    const posted: unknown[] = [];
    vi.spyOn(BroadcastChannel.prototype, 'postMessage').mockImplementation((message: unknown) => {
      posted.push(message);
    });

    const unsubscribe = subscribeAppModelSelectionChanges(() => {});
    announceAppModelSelection();
    unsubscribe();

    expect(posted).toEqual([{ kind: 'app-model-selection' }]);
  });

  it('wakes on a nudge another window posted', async () => {
    let woken = 0;
    const unsubscribe = subscribeAppModelSelectionChanges(() => {
      woken += 1;
    });

    const other = new BroadcastChannel('biorouter:session-binding');
    other.postMessage({ kind: 'app-model-selection' });
    await drain(other);
    other.close();
    unsubscribe();

    expect(woken).toBe(1);
  });

  /**
   * One channel, two facts, told apart by shape — and neither may be mistaken
   * for the other. A nudge must not reach a row patcher (it names no session to
   * patch), and a binding must not make every window re-read its selection: a
   * per-chat switch over there says nothing about new chats over here.
   */
  it('keeps the two facts apart on the one channel', async () => {
    const bindings: unknown[] = [];
    let nudges = 0;
    const offBinding = subscribeSessionBindingChanges(
      payloadsOnly((change) => bindings.push(change))
    );
    const offSelection = subscribeAppModelSelectionChanges(() => {
      nudges += 1;
    });

    const other = new BroadcastChannel('biorouter:session-binding');
    other.postMessage({ kind: 'app-model-selection' });
    other.postMessage({ sessionId: 's5', provider: 'codex', model: 'gpt-6-astra' });
    await drain(other);
    other.close();
    offBinding();
    offSelection();

    expect(nudges).toBe(1);
    expect(bindings).toEqual([{ sessionId: 's5', provider: 'codex', model: 'gpt-6-astra' }]);
  });

  it('ignores a message of some other kind', async () => {
    let nudges = 0;
    const unsubscribe = subscribeAppModelSelectionChanges(() => {
      nudges += 1;
    });

    const other = new BroadcastChannel('biorouter:session-binding');
    other.postMessage({ kind: 'something-else' });
    other.postMessage('app-model-selection');
    await drain(other);
    other.close();
    unsubscribe();

    expect(nudges).toBe(0);
  });

  it('stops waking a listener once it unsubscribes', () => {
    let woken = 0;
    const unsubscribe = subscribeAppModelSelectionChanges(() => {
      woken += 1;
    });
    unsubscribe();

    announceAppModelSelection();

    expect(woken).toBe(0);
  });
});
