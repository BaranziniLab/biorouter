/**
 * **Find words in my recent chats** (owner, 2026-11-13): the chat extractor run once, on
 * demand, over the person's own recent chats on public models, through
 * `GET`/`POST /dictionary/suggestions/scan` and `POST /dictionary/suggestions/scan/stop`.
 *
 * One scan runs per daemon, so this is one module store, as `useDictionary.ts` is: every window
 * that shows the control reads the same scan, and a scan started in another window shows here
 * once the control mounts.
 *
 * - **Every call sends `userActionHeaders()`**, as every dictionary call does.
 * - **Polled only while it runs**: once when the control mounts, then every
 *   {@link SCAN_POLL_MS} while the daemon answers `running`, and never after the control
 *   unmounts.
 * - **The listing is read again when a scan ends**, so the suggestions it added appear.
 * - **Stale answers are dropped**: a read that lands after a newer request started is thrown
 *   away, so a poll in flight cannot undo a Stop.
 */
import { useEffect, useSyncExternalStore } from 'react';
import {
  getDictionarySuggestionScan,
  startDictionarySuggestionScan,
  stopDictionarySuggestionScan,
} from '../../api';
import type { DictionaryScan } from '../../api';
import { userActionHeaders } from '../../utils/userAction';
import { DICTIONARY_LABELS, dictionaryMessage } from './copy';
import { failureOf, refreshDictionary } from './useDictionary';

/** How often a running scan is read again. */
export const SCAN_POLL_MS = 700;

export interface ScanSnapshot {
  /** The daemon's scan as last read; null before the first answer. */
  scan: DictionaryScan | null;
  /** This window started the scan or saw it running, so how it ended is news here. */
  watched: boolean;
  /** The daemon's sentence for a start or a stop it refused; cleared by the next start. */
  refusal: string | null;
  /** A start is in flight. */
  starting: boolean;
  /** Stop was pressed and the scan has not ended yet. */
  stopping: boolean;
}

const INITIAL: ScanSnapshot = {
  scan: null,
  watched: false,
  refusal: null,
  starting: false,
  stopping: false,
};

let snapshot: ScanSnapshot = INITIAL;
const listeners = new Set<() => void>();
let generation = 0;

function publish(next: Partial<ScanSnapshot>): void {
  snapshot = { ...snapshot, ...next };
  for (const listener of [...listeners]) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function getScanSnapshot(): ScanSnapshot {
  return snapshot;
}

function ended(scan: DictionaryScan): boolean {
  return scan.state === 'finished' || scan.state === 'stopped' || scan.state === 'failed';
}

/**
 * Take a scan the daemon answered. A scan this window saw running, or started, that has now
 * ended reads the listing again, once, so the suggestions it added show.
 */
function take(scan: DictionaryScan, fromStart = false): void {
  const wasRunning = snapshot.scan?.state === 'running';
  const running = scan.state === 'running';
  const justEnded = ended(scan) && (wasRunning || fromStart);
  publish({
    scan,
    watched: snapshot.watched || running || fromStart,
    stopping: running ? snapshot.stopping : false,
    // A scan that started since a refusal (here or in another window) answers it.
    refusal: running && !wasRunning ? null : snapshot.refusal,
  });
  if (justEnded) void refreshDictionary();
}

/** Read the scan as it stands. A failed read keeps what was shown. */
export async function readScan(): Promise<void> {
  const issued = ++generation;
  let result: { data?: unknown; response?: Response };
  try {
    result = (await getDictionarySuggestionScan({ headers: await userActionHeaders() })) as {
      data?: unknown;
      response?: Response;
    };
  } catch {
    return;
  }
  if (issued !== generation || !result.response?.ok || !result.data) return;
  take(result.data as DictionaryScan);
}

/**
 * Start a scan, or join the one running. A refusal (`suggestions_off`, `agent`, `policy`,
 * `unreadable`) shows the daemon's own sentence.
 */
export async function startScan(): Promise<void> {
  if (snapshot.starting) return;
  const issued = ++generation;
  publish({ starting: true, refusal: null });
  let result: { data?: unknown; error?: unknown; response?: Response };
  try {
    result = (await startDictionarySuggestionScan({ headers: await userActionHeaders() })) as {
      data?: unknown;
      error?: unknown;
      response?: Response;
    };
  } catch {
    result = {};
  }
  if (result.response?.ok && result.data) {
    publish({ starting: false });
    // A newer read already answered: it is at least as new as this one.
    if (issued === generation) take(result.data as DictionaryScan, true);
    return;
  }
  publish({ starting: false, refusal: failureOf(result).message });
}

/** Ask the running scan to stop after the chat it is reading; it keeps what it found. */
export async function stopScan(): Promise<void> {
  if (snapshot.stopping) return;
  const issued = ++generation;
  publish({ stopping: true });
  let result: { data?: unknown; error?: unknown; response?: Response };
  try {
    result = (await stopDictionarySuggestionScan({ headers: await userActionHeaders() })) as {
      data?: unknown;
      error?: unknown;
      response?: Response;
    };
  } catch {
    result = {};
  }
  if (result.response?.ok && result.data) {
    if (issued === generation) take(result.data as DictionaryScan);
    return;
  }
  publish({ stopping: false, refusal: failureOf(result).message });
}

/**
 * The one line the control shows beside its button: the progress while a scan runs, how a
 * watched scan ended, or the daemon's refusal. Null when there is nothing to say.
 */
export function scanLine(state: ScanSnapshot): string | null {
  const { scan } = state;
  // A refused Stop while the scan still runs says so in place of the progress.
  if (state.refusal) return state.refusal;
  if (scan?.state === 'running') {
    // Before it knows how many chats it reads there is nothing to count; Stop still shows.
    if (scan.chats_total === 0) return null;
    // `chats_read` counts the chats done, so the one being read is the next, as the CLI's
    // progress line counts it (`line_scan_progress`).
    return dictionaryMessage('MSG-SCAN-PROGRESS', {
      read: Math.min(scan.chats_read + 1, scan.chats_total).toLocaleString('en-US'),
      total: scan.chats_total.toLocaleString('en-US'),
      words: DICTIONARY_LABELS.scan.words(scan.words_found),
    });
  }
  if (!scan || !state.watched) return null;
  const chats = DICTIONARY_LABELS.scan.chats(scan.chats_total);
  const words = DICTIONARY_LABELS.scan.newWords(scan.words_found);
  switch (scan.state) {
    case 'finished':
      return scan.words_found > 0
        ? dictionaryMessage('MSG-SCAN-FINISHED', { chats, words })
        : dictionaryMessage('MSG-SCAN-FINISHED-NONE', { chats });
    case 'stopped': {
      const read = scan.chats_read.toLocaleString('en-US');
      return scan.words_found > 0
        ? dictionaryMessage('MSG-SCAN-STOPPED', { read, chats, words })
        : dictionaryMessage('MSG-SCAN-STOPPED-NONE', { read, chats });
    }
    case 'failed':
      return dictionaryMessage('MSG-SCAN-FAILED');
    default:
      return null;
  }
}

/**
 * The scan for the control that shows it: read once when it mounts (so a scan started in
 * another window shows), then every {@link SCAN_POLL_MS} while it runs. Reads only while
 * `enabled`, and stops polling when the control unmounts.
 */
export function useSuggestionScan(enabled: boolean): ScanSnapshot {
  const current = useSyncExternalStore(subscribe, getScanSnapshot);
  const running = current.scan?.state === 'running';

  useEffect(() => {
    if (enabled) void readScan();
  }, [enabled]);

  useEffect(() => {
    if (!enabled || !running) return;
    let alive = true;
    const tick = () => {
      void readScan().finally(() => {
        if (alive) timer = setTimeout(tick, SCAN_POLL_MS);
      });
    };
    let timer = setTimeout(tick, SCAN_POLL_MS);
    return () => {
      alive = false;
      clearTimeout(timer);
    };
  }, [enabled, running]);

  return current;
}

/** For tests: start over. A request still in flight from before is dropped when it lands. */
export function resetSuggestionScanForTests(): void {
  snapshot = INITIAL;
  listeners.clear();
  generation += 1;
}
