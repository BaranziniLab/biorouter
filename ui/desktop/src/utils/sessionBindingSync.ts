/**
 * Round 3 / N1 — keeping the renderer's copy of a chat's BINDING fresh.
 *
 * # Why this exists
 *
 * The composer must state the model a chat actually runs on, and that is the
 * chat's own session row: `restore_provider_from_session` binds
 * `session.provider_name` + `session.model_config` whenever either is set, and
 * only falls back to the app-wide selection for a chat that names neither.
 *
 * The renderer cannot simply prefer the row, though, because its copy of the
 * row is a cache. `ChatStreamController` reads it once (from `/agent/resume`)
 * and then only ever patches it; nothing refetches it. So a per-chat model
 * switch — which writes the row through `updateAgentProvider` and the global
 * default through `setConfigProvider`, in that order — left the daemon with the
 * two in step and this renderer holding the value from BEFORE the switch. A
 * composer preferring the row would answer the switch by showing the model the
 * user had just switched away from. That regression is why PR #192 shipped the
 * override only for the case where the privacy barrier proves the selection
 * cannot run here.
 *
 * The fix is to make the cached row fresh rather than to guess around it, and
 * this module is one of the two halves. `changeModel` announces the binding it
 * just wrote, the controller for that session patches its snapshot
 * SYNCHRONOUSLY, and only then does the global selection move. There is
 * therefore no render in which the row is stale — during the gap the row holds
 * the NEW binding and the selection still holds the old one, which is the
 * direction that shows the user what they just chose.
 *
 * The other half is `ChatStreamController.refreshSessionBinding`, which
 * re-reads the row after a turn: a turn can change `privacy_tier` (the ratchet)
 * and the bound provider in ways only the daemon knows, so those cannot be
 * announced from here.
 *
 * ⚠ **Announce only what the daemon accepted.** `updateAgentProvider` can be
 * refused — Gate A returns 409 for a public model on a private chat — and a
 * refused switch leaves the row exactly as it was. The announcement therefore
 * belongs after that call has resolved, never beside the optimistic UI update.
 *
 * ⚠ **In-renderer only, deliberately.** Unlike `sessionNameSync`, this is not
 * carried over a `BroadcastChannel`: a model switch is an act of one window,
 * the row it writes is re-read by any other window on its next turn or resume,
 * and a second window's own `ModelAndProviderContext` is not updated by this
 * event either. Broadcasting the binding alone would make one of the two facts
 * cross windows and not the other.
 */

export interface SessionBindingChange {
  /** The chat whose row was rewritten. */
  sessionId: string;
  /** The provider id the daemon accepted, e.g. `versa_azure`. */
  provider: string;
  /** The model name the daemon accepted, e.g. `gpt-5.5-2026-04-24`. */
  model: string;
  /**
   * The context window sent with the bind, when the picker knew one.
   *
   * Carried so a patched row cannot end up naming one model beside another's
   * window. `undefined` means "not known here" and clears the stale value
   * rather than keeping it — the daemon derives the real limit, and the
   * post-turn refresh brings it back.
   */
  contextLimit?: number | null;
}

type Listener = (change: SessionBindingChange) => void;

const listeners = new Set<Listener>();

/** Subscribe to model-switch announcements. Returns the unsubscribe. */
export function subscribeSessionBindingChanges(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * Announce a binding the daemon has already accepted for `sessionId`.
 *
 * Synchronous by design: the caller is mid-switch, and the point of the
 * announcement is that no render happens between the write landing and this
 * renderer knowing about it.
 */
export function announceSessionBinding(change: SessionBindingChange): void {
  for (const listener of [...listeners]) listener(change);
}
