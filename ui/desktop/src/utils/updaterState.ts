// Pure, framework-free state machine for the in-app auto-updater UI.
//
// Both the in-app update modal (`UpdateAvailableModal`) and the Settings
// "Check for Updates" panel (`UpdateSection`) drive their UI from this reducer.
// Keeping it pure (no React, no Electron) makes the one-click update flow unit
// testable: feed it the `updater-event` payloads the main process emits via
// `window.electron.onUpdaterEvent`, and assert on the resulting view state.
//
// Event names mirror what `src/utils/autoUpdater.ts` sends through
// `sendStatusToWindow(event, data)`:
//   checking-for-update | update-available | update-not-available |
//   download-progress | update-downloaded | error

export type UpdatePhase =
  | 'idle'
  | 'checking'
  | 'available' // update found, downloading in the background
  | 'downloaded' // fully downloaded, ready for one-click restart+install
  | 'up-to-date'
  | 'error';

export interface UpdaterState {
  phase: UpdatePhase;
  /** Latest version offered by the release feed (no leading "v"). */
  latestVersion?: string;
  /** Download progress 0–100 while phase === 'available'. */
  percent: number;
  /** Human-readable error when phase === 'error'. */
  error?: string;
  /**
   * True when the main process fell back to the assisted GitHub downloader
   * (release lacks an electron-updater manifest, or non-mac platform). In that
   * mode "install" opens the downloaded installer instead of doing a silent
   * in-place restart.
   */
  usingFallback: boolean;
}

export const initialUpdaterState: UpdaterState = {
  phase: 'idle',
  percent: 0,
  usingFallback: false,
};

export interface UpdaterEventPayload {
  event: string;
  data?: unknown;
  /**
   * Whether the main process is serving this event from the assisted GitHub
   * downloader rather than electron-updater.
   *
   * ⚠ This rides on every event, not just the snapshot. `usingFallback` existed
   * on `UpdaterState` and was only ever populated by `stateFromSnapshot`, so a
   * renderer that mounted before the check (the normal case -- the background
   * check fires ~5s after launch) never learned it was in assisted mode and
   * showed the silent-in-place-update copy on a platform that cannot do one.
   */
  usingFallback?: boolean;
}

/**
 * Is the app waiting for the user to start an assisted download?
 *
 * ⚠ The distinction this encodes is the whole Windows update bug. On macOS
 * electron-updater downloads the update by itself, so `phase === 'available'`
 * genuinely means "downloading in the background". On Windows there is no
 * electron-updater manifest, the GitHub fallback takes over, and that fallback
 * deliberately does NOT download -- it waits for the user, via the
 * `download-update` IPC (see `autoUpdater.ts`, "Deliberately NOT downloaded
 * here"). Nothing in the renderer ever invoked that IPC, so the modal promised
 * a background download that was never going to start and parked a progress bar
 * at 0% forever.
 *
 * So in assisted mode `available` means "found, awaiting your go-ahead", and
 * only once a download is actually running does progress mean anything.
 */
export function needsAssistedDownload(state: UpdaterState): boolean {
  return state.phase === 'available' && state.usingFallback && state.percent === 0;
}

export function normalizeVersion(v: unknown): string {
  if (typeof v === 'string' || typeof v === 'number') {
    return String(v).replace(/^v/i, '').trim();
  }

  if (v && typeof v === 'object' && 'version' in v) {
    const version = (v as { version?: unknown }).version;
    if (typeof version === 'string' || typeof version === 'number') {
      return String(version).replace(/^v/i, '').trim();
    }
  }

  return '';
}

/**
 * Semantic-ish version compare on dotted numeric segments. Returns true when
 * `latest` is strictly newer than `current`. Non-numeric/missing segments are
 * treated as 0, matching the existing modal behavior.
 */
export function isNewerVersion(latest: string, current: string): boolean {
  const parts = (v: string) =>
    normalizeVersion(v)
      .split('.')
      .map((p) => parseInt(p, 10))
      .map((n) => (Number.isFinite(n) ? n : 0));
  const lat = parts(latest);
  const cur = parts(current);
  const len = Math.max(lat.length, cur.length);
  for (let i = 0; i < len; i++) {
    const l = lat[i] ?? 0;
    const c = cur[i] ?? 0;
    if (l > c) return true;
    if (l < c) return false;
  }
  return false;
}

function versionFromData(data: unknown): string | undefined {
  if (data && typeof data === 'object' && 'version' in data) {
    const v = (data as { version?: unknown }).version;
    if (typeof v === 'string') return normalizeVersion(v);
  }
  return undefined;
}

function percentFromData(data: unknown): number | undefined {
  if (data && typeof data === 'object' && 'percent' in data) {
    const p = (data as { percent?: unknown }).percent;
    if (typeof p === 'number' && Number.isFinite(p)) {
      return Math.max(0, Math.min(100, Math.round(p)));
    }
  }
  return undefined;
}

/**
 * Fold one main-process `updater-event` into the view state. Pure: returns a
 * new object and never mutates `prev`.
 */
export function reduceUpdaterEvent(prev: UpdaterState, payload: UpdaterEventPayload): UpdaterState {
  // Latch it: the main process stamps every event, but a payload that omits it
  // (an older main process, or a hand-built test event) must not silently clear
  // a mode we already know we are in.
  const prevState: UpdaterState =
    payload.usingFallback === undefined
      ? prev
      : { ...prev, usingFallback: payload.usingFallback };
  prev = prevState;

  switch (payload.event) {
    case 'checking-for-update':
      // Don't clobber a finished download with a later background re-check.
      if (prev.phase === 'downloaded') return prev;
      return { ...prev, phase: 'checking', error: undefined };

    case 'update-available': {
      const latestVersion = versionFromData(payload.data) ?? prev.latestVersion;
      // Once downloaded, stay downloaded.
      if (prev.phase === 'downloaded') return { ...prev, latestVersion };
      return {
        ...prev,
        phase: 'available',
        latestVersion,
        percent: prev.phase === 'available' ? prev.percent : 0,
        error: undefined,
      };
    }

    case 'update-not-available':
      if (prev.phase === 'downloaded') return prev;
      return { ...prev, phase: 'up-to-date', error: undefined };

    case 'download-progress': {
      const percent = percentFromData(payload.data) ?? prev.percent;
      if (prev.phase === 'downloaded') return prev;
      // Progress is monotonic; never let a stray smaller value rewind the bar.
      return {
        ...prev,
        phase: 'available',
        percent: Math.max(prev.percent, percent),
      };
    }

    case 'update-downloaded': {
      const latestVersion = versionFromData(payload.data) ?? prev.latestVersion;
      return { ...prev, phase: 'downloaded', latestVersion, percent: 100, error: undefined };
    }

    case 'error': {
      const message =
        typeof payload.data === 'string'
          ? payload.data
          : payload.data instanceof Error
            ? payload.data.message
            : 'Update failed. Try again.';
      // A background error after a successful download must not hide the
      // ready-to-install state — the user can still restart into the update.
      if (prev.phase === 'downloaded') return prev;
      return { ...prev, phase: 'error', error: message };
    }

    default:
      return prev;
  }
}

/** Should the startup modal be visible for this state? */
export function shouldShowUpdateModal(state: UpdaterState): boolean {
  return state.phase === 'available' || state.phase === 'downloaded' || state.phase === 'error';
}

export function hasKnownUpdate(state: UpdaterState): boolean {
  if (state.phase === 'available' || state.phase === 'downloaded') return true;
  return !!state.latestVersion && (state.phase === 'checking' || state.phase === 'error');
}

/**
 * Recover view state from the main process's persisted `get-update-state`
 * snapshot for a renderer that mounted after some events already fired.
 */
export function stateFromSnapshot(
  snapshot:
    | {
        updateAvailable?: boolean;
        latestVersion?: string;
        status?: UpdatePhase;
        percent?: number;
        usingFallback?: boolean;
        error?: string;
      }
    | null
    | undefined
): UpdaterState {
  if (!snapshot) return initialUpdaterState;
  const latestVersion = snapshot.latestVersion
    ? normalizeVersion(snapshot.latestVersion)
    : undefined;
  let phase: UpdatePhase = 'idle';
  if (snapshot.status) {
    phase = snapshot.status;
  } else if (snapshot.updateAvailable) {
    phase = 'available';
  }
  return {
    phase,
    latestVersion,
    percent:
      typeof snapshot.percent === 'number' ? snapshot.percent : phase === 'downloaded' ? 100 : 0,
    usingFallback: !!snapshot.usingFallback,
    error: snapshot.error,
  };
}
