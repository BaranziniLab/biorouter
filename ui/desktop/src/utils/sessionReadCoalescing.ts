/**
 * One network request for the identical session reads a chat issues together.
 *
 * # What was broken
 *
 * Opening a chat mounts several components that each read the chat's own row:
 * the composer (its working directory, and separately its privacy tier), the
 * subagent header, the extension menu (`/sessions/{id}/extensions`) and the cost
 * tracker (`/sessions/{id}/usage`). Each read was issued independently, React's
 * StrictMode doubles every effect in development, and the composer remounts once
 * when the transcript replaces the empty-chat layout. Measured with CDP on
 * 2026-09-13: a plain reload into a chat sent TEN identical `GET /sessions/{id}`
 * and four identical `/extensions` inside two milliseconds, each behind its own
 * CORS preflight, and that burst filled the connection pool so the chat's own
 * `/agent/resume` queued ~400 ms behind it.
 *
 * # What this cannot share, and must not try to
 *
 * Only reads issued in the same moment. Two readers whose reads are separated by
 * a round trip are two requests, and holding a batch open long enough to join
 * them would just be a cache under another name. That is not hypothetical: the
 * subagent header read the row at mount, and a browser withholds the composer
 * until the chat store's `/agent/resume` has answered (`composerSlotMode`), so
 * under `biorouter serve` the two went out 55–290 ms apart — two requests on
 * almost every open, while the desktop, which mounts both together, sent one.
 * A reader like that is fixed by not reading: the header now takes the row the
 * chat store already holds (`components/subagent/useSubagentSession.ts`).
 *
 * # The rule, and why it cannot serve a stale answer
 *
 * **A read may share only a request that has not yet been handed to the
 * network.** The first read of a burst opens a batch and waits
 * {@link SESSION_READ_HOLD_MS}; identical reads issued in that window join it;
 * then the batch CLOSES — before the request is even created — and exactly one
 * request goes out. Every caller that joined asked before that request left the
 * renderer, so the daemon reads its store after every one of them asked: the
 * shared answer is exactly as fresh as each caller's own request would have been.
 *
 * A read issued after the batch closed opens a new one. It never joins a request
 * already in flight, and nothing is kept once a response lands. That is what
 * keeps the reads that exist to SEE a change correct: the refresh after a turn
 * (`refreshSessionBinding`, the composer's `message-stream-finished` tier read),
 * the auto-rename polls, the read-back after an edit truncates the transcript,
 * the refresh after an extension toggle, the re-reads a cross-window broadcast
 * or a declassification triggers. Each is issued after the change it waits for
 * was acknowledged, so it can only ever join a batch that has not yet asked the
 * daemon anything. Joining an in-flight request — or any short-lived cache —
 * would be the opposite: the one read that exists to see the rename could be
 * answered by a request issued before it.
 *
 * # What it shares, and what it never does
 *
 * - **Identical requests only.** The key is the method, the full URL (so a
 *   `?metadata_only=true` read never shares with a full one) and every header —
 *   `X-Secret-Key`, `X-User-Action`, `X-Caller-Provider` included. A read carrying
 *   the proof of a person never shares with one that does not, in either
 *   direction: a private chat's read cannot lose its proof here, and a proof-less
 *   caller cannot receive an answer only the proof was given.
 * - **Nothing is attached.** This sees requests the renderer already built and
 *   adds no header, which is the constraint `utils/userAction.ts` puts on anything
 *   installed through `client.setConfig`.
 * - **Only a chat's own row, its extension list and its usage** —
 *   `GET /sessions/{id}`, `GET /sessions/{id}/extensions` and
 *   `GET /sessions/{id}/usage`. The event stream, the change long-poll, the list
 *   routes and every write pass straight through.
 * - **Every caller gets its own `Response`.** A lone read receives the network's
 *   response untouched; a shared one is buffered once and each caller is handed
 *   a new `Response` over those bytes, so one caller consuming its body (or
 *   mutating what it parsed) cannot reach another.
 * - **A caller's abort is its own.** It rejects that caller only; the request the
 *   others wait on is never cancelled by one of them.
 * - **Not while the page is hidden.** A hidden window's timers are throttled to
 *   about once a second, so holding a read there would stall it. Those reads pass
 *   straight through, and a batch still open when the page hides is sent at once.
 */

/**
 * How long the first read of a burst waits for identical ones.
 *
 * Measured on 2026-09-13 in the dev app by instrumenting `fetch` itself: the
 * reads of one chat-open burst reached it within 1.6 ms of each other — the
 * spread is the proof-of-user IPC each caller awaits first. One frame is ten
 * times that, and small beside the ~400 ms the burst itself was costing.
 */
export const SESSION_READ_HOLD_MS = 16;

/** The fixed routes under `/sessions/` whose segment is not a chat id. */
const STATIC_SESSION_ROUTES = new Set([
  'activity',
  'changes',
  'import',
  'insights',
  'running',
  'sidebar',
]);

/** Statuses a `Response` must be constructed with a null body for. */
const NULL_BODY_STATUSES = new Set([101, 103, 204, 205, 304]);

/**
 * Is this the read of one chat's row (`GET /sessions/{id}`, any query), its
 * extension list (`GET /sessions/{id}/extensions`) or its usage
 * (`GET /sessions/{id}/usage`)?
 */
export function isCoalescableSessionRead(request: Request): boolean {
  if (request.method !== 'GET') return false;
  let pathname: string;
  try {
    pathname = new URL(request.url).pathname;
  } catch {
    return false;
  }
  const match = /^\/sessions\/([^/]+)(\/extensions|\/usage)?$/.exec(pathname);
  if (!match) return false;
  let segment: string;
  try {
    segment = decodeURIComponent(match[1]);
  } catch {
    return false;
  }
  return !STATIC_SESSION_ROUTES.has(segment);
}

/** Everything that makes two requests the same request. */
function batchKey(request: Request): string {
  const headers = [...request.headers.entries()].sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
  return JSON.stringify([
    request.method,
    request.url,
    request.mode,
    request.credentials,
    request.cache,
    request.redirect,
    headers,
  ]);
}

// Typed off `fetch` itself rather than by naming `RequestInfo`, which ESLint's
// `no-undef` does not know (the same choice `src/test/networkGuard.ts` makes).
export type FetchLike = (
  input: Parameters<typeof fetch>[0],
  init?: Parameters<typeof fetch>[1]
) => Promise<Response>;

interface Waiter {
  request: Request;
  resolve: (response: Response) => void;
  reject: (reason: unknown) => void;
  detach: () => void;
}

interface Batch {
  waiters: Set<Waiter>;
  timer: ReturnType<typeof setTimeout> | undefined;
}

export interface SessionReadCoalescingOptions {
  /** Defaults to {@link SESSION_READ_HOLD_MS}. */
  holdMs?: number;
  /** Defaults to `document.visibilityState === 'hidden'`. */
  isHidden?: () => boolean;
  /** Calls `onHidden` whenever the page becomes hidden. Defaults to `visibilitychange`. */
  subscribeHidden?: (onHidden: () => void) => void;
}

const documentIsHidden = () =>
  typeof document !== 'undefined' && document.visibilityState === 'hidden';

const subscribeDocumentHidden = (onHidden: () => void) => {
  if (typeof document === 'undefined') return;
  document.addEventListener('visibilitychange', () => {
    if (documentIsHidden()) onHidden();
  });
};

/**
 * Wrap `base` (the network) so identical chat reads issued together reach it
 * once. See the module comment for the rule and its freshness argument.
 */
export function coalesceSessionReads(
  base: (request: Request) => Promise<Response>,
  options: SessionReadCoalescingOptions = {}
): FetchLike {
  const holdMs = options.holdMs ?? SESSION_READ_HOLD_MS;
  const isHidden = options.isHidden ?? documentIsHidden;
  const open = new Map<string, Batch>();

  const settleAlone = (waiter: Waiter, request: Request) => {
    base(request).then(
      (response) => {
        waiter.detach();
        waiter.resolve(response);
      },
      (error) => {
        waiter.detach();
        waiter.reject(error);
      }
    );
  };

  const dispatch = (key: string, batch: Batch) => {
    // CLOSE FIRST — before the request exists. A read issued from here on opens a
    // new batch, so no caller can join a request that has already asked the
    // daemon. This ordering IS the freshness argument; do not move it below.
    if (open.get(key) === batch) open.delete(key);
    if (batch.timer !== undefined) clearTimeout(batch.timer);
    batch.timer = undefined;

    const waiters = [...batch.waiters];
    if (waiters.length === 0) return;

    if (waiters.length === 1) {
      // Exactly the request the caller would have sent, its own signal included.
      settleAlone(waiters[0], waiters[0].request);
      return;
    }

    // Shared: the request carries no caller's signal, so one caller aborting
    // cannot cancel the answer the others are waiting for.
    const shared = new Request(waiters[0].request, { signal: null });
    base(shared).then(
      async (response) => {
        const constructible = response.status >= 200 && response.status <= 599;
        let body: ArrayBuffer | null = null;
        if (constructible && !NULL_BODY_STATUSES.has(response.status)) {
          try {
            body = await response.arrayBuffer();
          } catch (error) {
            for (const waiter of waiters) {
              waiter.detach();
              waiter.reject(error);
            }
            return;
          }
        }
        waiters.forEach((waiter, index) => {
          if (!constructible) {
            // A status `new Response` cannot carry. The first caller gets the
            // original; the rest ask again for themselves — later than they first
            // asked, so never less fresh.
            if (index === 0) {
              waiter.detach();
              waiter.resolve(response);
            } else {
              settleAlone(waiter, waiter.request);
            }
            return;
          }
          waiter.detach();
          waiter.resolve(
            new Response(body, {
              status: response.status,
              statusText: response.statusText,
              headers: response.headers,
            })
          );
        });
      },
      (error) => {
        for (const waiter of waiters) {
          waiter.detach();
          waiter.reject(error);
        }
      }
    );
  };

  (options.subscribeHidden ?? subscribeDocumentHidden)(() => {
    for (const [key, batch] of [...open.entries()]) dispatch(key, batch);
  });

  return (input, init) => {
    const request =
      input instanceof Request && init === undefined ? input : new Request(input, init);
    if (!isCoalescableSessionRead(request) || request.signal.aborted || isHidden()) {
      return base(request);
    }

    const key = batchKey(request);
    let batch = open.get(key);
    if (!batch) {
      const fresh: Batch = { waiters: new Set(), timer: undefined };
      fresh.timer = setTimeout(() => dispatch(key, fresh), holdMs);
      open.set(key, fresh);
      batch = fresh;
    }
    const joined = batch;

    return new Promise<Response>((resolve, reject) => {
      const onAbort = () => {
        waiter.detach();
        // Still waiting for the batch to close: leave it, and a batch nobody is
        // waiting on sends nothing. Already sent: the others keep their answer.
        if (joined.waiters.delete(waiter) && joined.waiters.size === 0) {
          if (open.get(key) === joined) open.delete(key);
          if (joined.timer !== undefined) clearTimeout(joined.timer);
          joined.timer = undefined;
        }
        reject(
          request.signal.reason ?? new DOMException('The operation was aborted.', 'AbortError')
        );
      };
      const waiter: Waiter = {
        request,
        resolve,
        reject,
        detach: () => request.signal.removeEventListener('abort', onAbort),
      };
      request.signal.addEventListener('abort', onAbort);
      joined.waiters.add(waiter);
    });
  };
}
