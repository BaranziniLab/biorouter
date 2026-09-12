/**
 * The standing record of extensions that failed to load.
 *
 * ⚠ **Why this module exists.** `showExtensionLoadResults` has exactly one
 * caller — `/agent/resume` in `hooks/chatStreamStore.tsx` — and a renderer load
 * always resumes. So the extension-failure toast announced something the user
 * did not do, and announced it again on every reload; dismissing it bought one
 * page load of quiet. That is precisely the "never notify for what the user did
 * not do" rule, and the fix is not a shorter dwell time: a failure that is still
 * true tomorrow is a STANDING CONDITION, and standing conditions belong on a
 * persistent surface (the Extensions page's `Note`), not in the transient layer.
 *
 * So the split is:
 *   · the record here is authoritative and survives reloads,
 *   · the toast fires **once per distinct failure** (name + error text) and
 *     never again for the same one,
 *   · an extension that later loads cleanly has its record dropped, so a fixed
 *     extension stops being reported without anyone dismissing anything.
 *
 * The `announced` flag is persisted rather than held in memory for the same
 * reason the record is: in-memory state is exactly what a renderer load resets,
 * and resetting it is what produced the recurrence.
 */

const STORAGE_KEY = 'biorouter.extensions.loadFailures';

export interface ExtensionLoadFailure {
  /** The extension's name as the daemon reported it. */
  name: string;
  /** The full error text, as reported. Never truncated here. */
  error: string;
  /** A shipped capability rather than something the user installed. */
  builtin: boolean;
  /** ms epoch of the most recent observation of this failure. */
  at: number;
  /** True once the transient layer has announced this exact failure. */
  announced: boolean;
}

/** One load result, reduced to what the record cares about. */
export interface ExtensionLoadOutcome {
  name: string;
  success: boolean;
  error?: string | null;
  builtin?: boolean;
}

type Listener = (failures: ExtensionLoadFailure[]) => void;

const listeners = new Set<Listener>();
let cache: ExtensionLoadFailure[] | null = null;

function isFailure(value: unknown): value is ExtensionLoadFailure {
  if (!value || typeof value !== 'object') return false;
  const record = value as Record<string, unknown>;
  return typeof record.name === 'string' && typeof record.error === 'string';
}

function read(): ExtensionLoadFailure[] {
  if (cache) return cache;
  let parsed: unknown = null;
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    parsed = raw ? JSON.parse(raw) : null;
  } catch {
    // A private window, cleared site data, or a document written by a build
    // that shaped this differently. An unreadable record is an empty one — it
    // must never take the page down with it.
    parsed = null;
  }
  cache = Array.isArray(parsed)
    ? parsed.filter(isFailure).map((entry) => ({
        name: entry.name,
        error: entry.error,
        builtin: entry.builtin === true,
        at: typeof entry.at === 'number' ? entry.at : 0,
        announced: entry.announced === true,
      }))
    : [];
  return cache;
}

function write(next: ExtensionLoadFailure[]): void {
  cache = next;
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  } catch {
    // Storage is full or unavailable. The in-memory cache still serves this
    // page load; the record simply does not survive the next one.
  }
  for (const listener of listeners) listener(next);
}

/** The standing failures, newest observation first. */
export function getExtensionLoadFailures(): ExtensionLoadFailure[] {
  return [...read()].sort((a, b) => b.at - a.at);
}

/**
 * Subscribe to the record. Fires on every change made in this renderer, and on
 * a `storage` event so a second window showing the Extensions page follows a
 * failure recorded in the first.
 */
export function subscribeExtensionLoadFailures(listener: Listener): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

if (typeof window !== 'undefined' && typeof window.addEventListener === 'function') {
  window.addEventListener('storage', (event) => {
    if (event.key !== null && event.key !== STORAGE_KEY) return;
    cache = null;
    const next = read();
    for (const listener of listeners) listener(next);
  });
}

/**
 * Fold one load report into the record and answer what is NEW.
 *
 * A failure is new when nothing has been announced for that extension, or when
 * the error text has changed since it was (a different failure is different
 * news). Successes drop any record for that extension.
 *
 * @returns the failures worth announcing. Empty means "nothing the user has not
 *          already been told", which is the caller's cue to stay silent.
 */
export function recordExtensionLoadResults(
  results: readonly ExtensionLoadOutcome[]
): ExtensionLoadFailure[] {
  const existing = read();
  const byName = new Map(existing.map((entry) => [entry.name, entry]));
  const fresh: ExtensionLoadFailure[] = [];
  let changed = false;

  for (const result of results) {
    if (result.success) {
      if (byName.delete(result.name)) changed = true;
      continue;
    }
    const error = result.error || 'Unknown error';
    const previous = byName.get(result.name);
    const alreadyAnnounced = previous?.announced === true && previous.error === error;
    const entry: ExtensionLoadFailure = {
      name: result.name,
      error,
      builtin: result.builtin === true,
      at: Date.now(),
      announced: alreadyAnnounced,
    };
    byName.set(result.name, entry);
    changed = true;
    if (!alreadyAnnounced) fresh.push(entry);
  }

  if (changed) write([...byName.values()]);
  return fresh;
}

/**
 * Mark these failures as announced, so no later renderer load repeats them.
 * Called only once a toast has actually been rendered — a suppressed report
 * must not consume the one announcement its failure is owed.
 */
export function markExtensionLoadFailuresAnnounced(names: readonly string[]): void {
  const wanted = new Set(names);
  const current = read();
  if (!current.some((entry) => wanted.has(entry.name) && !entry.announced)) return;
  write(current.map((entry) => (wanted.has(entry.name) ? { ...entry, announced: true } : entry)));
}

/** Drop one extension's record — the Dismiss control on the standing notice. */
export function dismissExtensionLoadFailure(name: string): void {
  const current = read();
  const next = current.filter((entry) => entry.name !== name);
  if (next.length !== current.length) write(next);
}

/** Test seam — the record is persistent by design, so it must be cleared explicitly. */
export function resetExtensionLoadFailuresForTests(): void {
  cache = null;
  try {
    localStorage.removeItem(STORAGE_KEY);
  } catch {
    // Nothing to clear.
  }
}
