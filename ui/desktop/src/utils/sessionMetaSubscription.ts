import { sessionChanges } from '../api';
import type { SessionMetaDelta } from '../api';

/**
 * Handoff 04. The renderer's ear on a session ROW that something else rewrote.
 *
 * # What is already covered, and what is not
 *
 * A chat's binding and classification reach this window three ways already: a
 * per-chat switch announces itself (`sessionBindingSync`, now across windows
 * too), a turn states both in the reply stream's own first frames, and a resume
 * reads the row. None of them can see a write made by **another process** —
 * `biorouter session --resume <id> --provider …` writes SQLite directly, and a
 * schedule may run in a daemon this window has never spoken to.
 *
 * That is what this follows, and it is the same shape as
 * `catalogSubscription`: a long poll on a revision, parked in the daemon, so an
 * idle app costs one open request rather than a timer.
 *
 * ⚠ **The revision is the contract, not the payload.** A consumer that applies a
 * change's fields and never re-reads drifts the first time two changes race.
 * Every consumer here is handed a session id and told to go and look.
 *
 * ⚠ **ONE subscription per renderer, mounted once.** Not per chat and not per
 * component. A subscription that restarts issues its next poll at a revision it
 * is already behind, which the daemon answers immediately rather than parking —
 * and a loop of those claims all six of Chromium's sockets to the host and
 * starves every other request the window makes. That is a measured failure, not
 * a hypothetical (`catalogSubscription.ts`).
 */

/** The window event non-React consumers listen for. */
export const SESSION_META_CHANGED_EVENT = 'session-meta:changed';

export interface SessionMetaSubscriptionOptions {
  /**
   * The chats this window has open, read fresh on every poll.
   *
   * A function rather than a value because the set changes as tabs open and
   * close, and re-subscribing on each change is the thing the ⚠ above forbids.
   */
  openSessionIds: () => string[];
  /** Called once per changed session id. */
  onSessionChanged: (sessionId: string) => void;
  /** Injected in tests. Defaults to the generated client. */
  poll?: (
    since: number,
    ids: string[],
    signal?: AbortSignal
  ) => Promise<SessionMetaDelta | undefined>;
  /** Injected in tests. */
  sleep?: (ms: number) => Promise<void>;
  /** How long to wait after a failed poll before trying again. */
  retryDelayMs?: number;
}

const DEFAULT_RETRY_MS = 5000;

/**
 * The floor under one turn of the loop, for the socket-starvation reason
 * `catalogSubscription.MIN_INTERVAL_MS` records in full.
 */
const MIN_INTERVAL_MS = 40;

/**
 * How long to park when this window has no chats open.
 *
 * The daemon would answer such a poll immediately — it has nothing to read and
 * the revision has not moved for us — so without this the loop would spin at the
 * floor above for as long as the app sits on Settings. Sleeping locally instead
 * costs nothing and holds no socket.
 */
const IDLE_SLEEP_MS = 2000;

const defaultPoll = async (
  since: number,
  ids: string[],
  signal?: AbortSignal
): Promise<SessionMetaDelta | undefined> => {
  const response = await sessionChanges({ query: { since, ids: ids.join(',') }, signal });
  return response.data;
};

const defaultSleep = (ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms));

/**
 * Start following session rows. Returns a function that stops it.
 */
export function subscribeToSessionMeta(options: SessionMetaSubscriptionOptions): () => void {
  const poll = options.poll ?? defaultPoll;
  const sleep = options.sleep ?? defaultSleep;
  const retryDelayMs = options.retryDelayMs ?? DEFAULT_RETRY_MS;

  let stopped = false;
  let since = 0;
  // ⚠ Stopping must RELEASE the request, not merely ignore its answer: a parked
  // poll holds one of this renderer's six sockets to the daemon for up to 25
  // seconds after the subscription that opened it is gone.
  const controller = new AbortController();

  const run = async () => {
    while (!stopped) {
      const startedAt = Date.now();
      const ids = options.openSessionIds();
      if (ids.length === 0) {
        await sleep(IDLE_SLEEP_MS);
        continue;
      }

      let delta: SessionMetaDelta | undefined;
      try {
        delta = await poll(since, ids, controller.signal);
      } catch {
        // The daemon is restarting, or the machine slept. Back off rather than
        // spinning; the next successful poll re-establishes the cursor.
        await sleep(retryDelayMs);
        continue;
      }
      if (stopped) return;
      if (!delta) {
        await sleep(retryDelayMs);
        continue;
      }

      // ⚠ A revision LOWER than the one we hold means the daemon restarted and
      // its counter went back to zero. Nothing was undone — our cursor is simply
      // meaningless, and holding it would park us forever on a number the daemon
      // will take a long time to climb back to. Every open chat is re-read,
      // because we cannot know what we missed while it was down.
      const restarted = delta.revision < since;
      since = delta.revision;

      if (restarted) {
        for (const id of ids) options.onSessionChanged(id);
        dispatchChanged(ids);
      } else {
        // ⚠ Ids are filtered HERE, not by the daemon. The ring is process-wide,
        // so a delta legitimately names chats another window is watching, and
        // waking this window for one it does not show would refetch a row it has
        // no copy of.
        const open = new Set(ids);
        const touched = [
          ...new Set(
            (delta.changes ?? []).map((change) => change.session_id).filter((id) => open.has(id))
          ),
        ];
        // `truncated` is an order to refetch, not a warning: the history we were
        // handed is partial, so the only safe answer is to re-read everything we
        // hold.
        const wanted = delta.truncated === true ? ids : touched;
        if (wanted.length > 0) {
          for (const id of wanted) options.onSessionChanged(id);
          dispatchChanged(wanted);
        }
      }

      // Answered without parking — we were behind. Keep a floor under the loop
      // so this can never become a busy wait on the daemon.
      const elapsed = Date.now() - startedAt;
      if (!stopped && elapsed < MIN_INTERVAL_MS) {
        await sleep(MIN_INTERVAL_MS - elapsed);
      }
    }
  };

  void run();
  return () => {
    stopped = true;
    controller.abort();
  };
}

function dispatchChanged(sessionIds: string[]) {
  if (typeof window === 'undefined') return;
  window.dispatchEvent(
    new CustomEvent<{ sessionIds: string[] }>(SESSION_META_CHANGED_EVENT, {
      detail: { sessionIds },
    })
  );
}
