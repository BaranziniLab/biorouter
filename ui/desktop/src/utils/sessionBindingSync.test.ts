import { beforeEach, describe, expect, it, vi } from 'vitest';
import {
  announceAppModelSelection,
  announceSessionBinding,
  subscribeAppModelSelectionChanges,
  subscribeSessionBindingChanges,
} from './sessionBindingSync';

const deliver = () => new Promise((resolve) => setTimeout(resolve, 0));

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
describe('sessionBindingSync', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
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
    const unsubscribe = subscribeSessionBindingChanges((change) =>
      seen.push(`${change.sessionId}:${change.model}`)
    );

    // A second window, speaking on the same channel.
    const other = new BroadcastChannel('biorouter:session-binding');
    other.postMessage({ sessionId: 's3', provider: 'versa_azure', model: 'gpt-5.2-2025-12-11' });
    await new Promise((resolve) => setTimeout(resolve, 0));
    other.close();
    unsubscribe();

    expect(seen).toEqual(['s3:gpt-5.2-2025-12-11']);
  });

  /**
   * ⚠ Shape-checked, not trusted. This arrives from another window, and a
   * message missing `provider` or `model` would otherwise patch a session row
   * with `undefined` — a row naming no model at all, which every reader of the
   * binding then states.
   */
  it('drops a malformed message rather than patching a row from it', async () => {
    const seen: unknown[] = [];
    const unsubscribe = subscribeSessionBindingChanges((change) => seen.push(change));

    const other = new BroadcastChannel('biorouter:session-binding');
    other.postMessage({ sessionId: 's4' });
    other.postMessage({ provider: 'versa_azure', model: 'gpt-5.5-2026-04-24' });
    other.postMessage(null);
    await new Promise((resolve) => setTimeout(resolve, 0));
    other.close();
    unsubscribe();

    expect(seen).toEqual([]);
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
    await deliver();
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
    const offBinding = subscribeSessionBindingChanges((change) => bindings.push(change));
    const offSelection = subscribeAppModelSelectionChanges(() => {
      nudges += 1;
    });

    const other = new BroadcastChannel('biorouter:session-binding');
    other.postMessage({ kind: 'app-model-selection' });
    other.postMessage({ sessionId: 's5', provider: 'codex', model: 'gpt-6-astra' });
    await deliver();
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
    await deliver();
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
