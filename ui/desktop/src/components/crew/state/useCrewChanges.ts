import { useEffect, useRef } from 'react';
import { crewChanges } from '../../../api';
import type { CrewChange, CrewChangesDelta } from '../../../api';
import { isBrowserSurface } from '../../../utils/surface';
import { userActionHeaders } from '../../../utils/userAction';

export type { CrewChange, CrewChangesDelta };

/**
 * Crew's change feed in the renderer (chat parity, WI-15): the window's ear on `GET /crew/changes`.
 *
 * Crew shows this computer's own Crew state (saved connections, setup jobs, grants, read
 * consents, transfers, the Crew task model), read when a surface opens and after its own actions.
 * A change made anywhere else used to stay invisible until a reload or a focus change: a chat's
 * `crew__change`, a second window, `biorouter crew` in a terminal. The daemon now announces every
 * one of them, and this module tells the surfaces, which read again what they show.
 *
 * ⚠ **A change is a reason to read again, never the data.** It names a kind and some IDs only, by
 * design (no names, no content). A surface re-reads through its own route, which decides what it
 * may show. `truncated` (the window fell behind, or the daemon restarted) means: read everything.
 *
 * ⚠ **Short polls, not a parked one.** The route can hold a request until something changes, as
 * `/catalog/changes` does, but this window does not ask it to: Chromium gives a page six
 * connections to the daemon, and a Crew view already holds the observer, the catalogue and session
 * long polls, a running reply and up to two live observers. A parked poll here would take the last
 * free one, and every other request (a Stop, a steer) would queue behind it. So the feed is asked
 * every {@link CREW_CHANGES_POLL_MS} with `timeout_ms=0`, which holds no connection between asks,
 * and not at all while the window is hidden.
 *
 * One poller per window, shared: it runs while any surface listens ({@link onCrewChanged},
 * {@link useCrewChanged}) and stops with the last. Every delta is also dispatched on `window` as
 * {@link CREW_CHANGED_EVENT}, which is what the surfaces listen to (and what tests dispatch).
 */

/** The window event a change is dispatched as. Its `detail` is the {@link CrewChangesDelta}. */
export const CREW_CHANGED_EVENT = 'crew:changed';
/** How often the feed is asked while the window is visible. A change shows within about this. */
export const CREW_CHANGES_POLL_MS = 1500;
/** How long to wait after a failed ask (the daemon restarting, the computer waking). */
export const CREW_CHANGES_RETRY_MS = 5000;
/** How long a surface waits for more changes before it reads again, so a burst is one read. */
export const CREW_CHANGED_DEBOUNCE_MS = 250;

export interface CrewChangesSubscriptionOptions {
  /** Called for every delta that reports a change. The window event is dispatched too. */
  onChange?: (delta: CrewChangesDelta) => void;
  /** Ask the feed. `since` absent: where it stands now. Injected in tests. */
  poll?: (since: number | undefined, signal: AbortSignal) => Promise<CrewChangesDelta | undefined>;
  /** Injected in tests. */
  sleep?: (ms: number) => Promise<void>;
  /** Whether the window is shown. Injected in tests. */
  visible?: () => boolean;
  intervalMs?: number;
  retryDelayMs?: number;
}

const defaultPoll = async (
  since: number | undefined,
  signal: AbortSignal
): Promise<CrewChangesDelta | undefined> => {
  const response = await crewChanges({
    query: since === undefined ? {} : { since, timeout_ms: 0 },
    headers: await userActionHeaders(),
    signal,
  });
  return response.data;
};

const defaultSleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

const windowVisible = () =>
  typeof document === 'undefined' || document.visibilityState !== 'hidden';

function isDelta(value: unknown): value is CrewChangesDelta {
  return (
    typeof value === 'object' &&
    value !== null &&
    typeof (value as CrewChangesDelta).revision === 'number' &&
    Array.isArray((value as CrewChangesDelta).changes)
  );
}

/**
 * Follow the feed. Returns a function that stops it. The first ask learns where the feed stands
 * and reports nothing: the surfaces read on open, so only what changes after that is news.
 */
export function subscribeToCrewChanges(options: CrewChangesSubscriptionOptions = {}): () => void {
  const poll = options.poll ?? defaultPoll;
  const sleep = options.sleep ?? defaultSleep;
  const visible = options.visible ?? windowVisible;
  const intervalMs = options.intervalMs ?? CREW_CHANGES_POLL_MS;
  const retryDelayMs = options.retryDelayMs ?? CREW_CHANGES_RETRY_MS;
  let stopped = false;
  let since: number | undefined;
  // Stopping releases the request in flight rather than only ignoring its answer.
  const controller = new AbortController();

  const run = async () => {
    while (!stopped) {
      if (!visible()) {
        await sleep(intervalMs);
        continue;
      }
      let delta: CrewChangesDelta | undefined;
      try {
        delta = await poll(since, controller.signal);
      } catch {
        if (stopped) return;
        await sleep(retryDelayMs);
        continue;
      }
      if (stopped) return;
      if (!isDelta(delta)) {
        await sleep(retryDelayMs);
        continue;
      }
      const first = since === undefined;
      since = delta.revision;
      if (!first && (delta.changes.length > 0 || delta.truncated)) {
        options.onChange?.(delta);
        dispatchCrewChanged(delta);
      }
      await sleep(intervalMs);
    }
  };

  void run();
  return () => {
    stopped = true;
    controller.abort();
  };
}

/** Tell every surface in this window that Crew changed. */
export function dispatchCrewChanged(delta: CrewChangesDelta): void {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(new CustomEvent<CrewChangesDelta>(CREW_CHANGED_EVENT, { detail: delta }));
}

// ---------------------------------------------------------------------------------------------
// The window's one poller
// ---------------------------------------------------------------------------------------------

let holders = 0;
let stopPoller: (() => void) | null = null;
/** Tests leave the poller off unless they ask for it, with their own poll. */
let testFeed: CrewChangesSubscriptionOptions | null = null;

function feedAllowed(): boolean {
  if (import.meta.env.MODE === 'test') return testFeed !== null;
  // A browser opened with `biorouter serve` has no Crew: every ask would be refused.
  return !isBrowserSurface();
}

/** Keep the window's poller running until the returned function is called. */
function retainFeed(): () => void {
  holders += 1;
  if (holders === 1 && feedAllowed()) stopPoller = subscribeToCrewChanges(testFeed ?? {});
  let released = false;
  return () => {
    if (released) return;
    released = true;
    holders -= 1;
    if (holders === 0) {
      stopPoller?.();
      stopPoller = null;
    }
  };
}

/** For tests: run the window's poller with `options` (null turns it off again). */
export function setCrewChangeFeedForTests(options: CrewChangesSubscriptionOptions | null): void {
  testFeed = options;
}

/** For tests: how many surfaces hold the poller, and whether it runs. */
export function crewChangeFeedStateForTests(): { holders: number; running: boolean } {
  return { holders, running: stopPoller !== null };
}

// ---------------------------------------------------------------------------------------------
// Listening
// ---------------------------------------------------------------------------------------------

function merged(a: CrewChangesDelta, b: CrewChangesDelta): CrewChangesDelta {
  return {
    revision: Math.max(a.revision, b.revision),
    changes: [...a.changes, ...b.changes],
    truncated: a.truncated || b.truncated,
  };
}

/**
 * Call `listener` once for each burst of changes, with every change in it, {@link
 * CREW_CHANGED_DEBOUNCE_MS} after the last. Keeps the window's poller running meanwhile. Returns
 * the function that stops listening.
 */
export function onCrewChanged(
  listener: (delta: CrewChangesDelta) => void,
  options: { debounceMs?: number } = {}
): () => void {
  if (typeof window === 'undefined') return () => undefined;
  const debounceMs = options.debounceMs ?? CREW_CHANGED_DEBOUNCE_MS;
  let pending: CrewChangesDelta | null = null;
  let timer: ReturnType<typeof setTimeout> | undefined;
  const flush = () => {
    timer = undefined;
    const delta = pending;
    pending = null;
    if (delta) listener(delta);
  };
  const handle = (event: Event) => {
    const delta = (event as CustomEvent<unknown>).detail;
    if (!isDelta(delta)) return;
    pending = pending ? merged(pending, delta) : delta;
    if (timer !== undefined) clearTimeout(timer);
    timer = setTimeout(flush, debounceMs);
  };
  const release = retainFeed();
  window.addEventListener(CREW_CHANGED_EVENT, handle);
  return () => {
    window.removeEventListener(CREW_CHANGED_EVENT, handle);
    if (timer !== undefined) clearTimeout(timer);
    pending = null;
    release();
  };
}

/**
 * {@link onCrewChanged} for a component, while `enabled` (default true). The newest `listener` is
 * called, so it may close over the render's values without restarting anything.
 */
export function useCrewChanged(
  listener: (delta: CrewChangesDelta) => void,
  options: { enabled?: boolean; debounceMs?: number } = {}
): void {
  const enabled = options.enabled ?? true;
  const debounceMs = options.debounceMs;
  const latest = useRef(listener);
  latest.current = listener;
  useEffect(() => {
    if (!enabled) return;
    return onCrewChanged((delta) => latest.current(delta), { debounceMs });
  }, [enabled, debounceMs]);
}

// ---------------------------------------------------------------------------------------------
// Reading a delta
// ---------------------------------------------------------------------------------------------

/**
 * Whether `delta` may concern any of `connectionIds`: it is truncated, or a change names one of
 * them, or names no connection at all (a change whose connection is not known may be any).
 */
export function changeTouches(delta: CrewChangesDelta, connectionIds: readonly string[]): boolean {
  return (
    delta.truncated ||
    delta.changes.some(
      (change) => !change.connection_id || connectionIds.includes(change.connection_id)
    )
  );
}

/**
 * Whether `delta` may concern the chat `sessionId`: it is truncated, or a change names that chat,
 * or names no chat at all (a connection connected or removed concerns every chat that uses it).
 */
export function changeMayConcernChat(delta: CrewChangesDelta, sessionId: string): boolean {
  return (
    delta.truncated ||
    delta.changes.some((change) => !change.session_id || change.session_id === sessionId)
  );
}

/** Whether `delta` holds a change of one of `kinds`, or may (it is truncated). */
export function changeOfKind(delta: CrewChangesDelta, kinds: readonly string[]): boolean {
  return delta.truncated || delta.changes.some((change) => kinds.includes(change.kind));
}

/** The connections `delta` names, newest change last, each once. */
export function changedConnections(delta: CrewChangesDelta): string[] {
  const named: string[] = [];
  for (const change of delta.changes) {
    const id = change.connection_id;
    if (!id) continue;
    const at = named.indexOf(id);
    if (at >= 0) named.splice(at, 1);
    named.push(id);
  }
  return named;
}
