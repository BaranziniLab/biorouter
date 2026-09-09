import { beforeEach, describe, expect, it, vi } from 'vitest';
import { announceSessionBinding, subscribeSessionBindingChanges } from './sessionBindingSync';

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
