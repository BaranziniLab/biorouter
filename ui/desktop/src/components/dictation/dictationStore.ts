/**
 * What the daemon says about dictation, for every microphone in this window
 * ("The status store" in `docs/dictation/surfaces.md`).
 *
 * A `useSyncExternalStore` singleton over `GET /dictation/status`. It sends no
 * request when a composer mounts: the suite renders the real composer in many
 * test files under an offline fetch guard (`src/test/setup.ts`), and a status
 * read nobody asked for is noise there and in the app. It loads on the first
 * sign that dictation may be used ({@link noteDictationInterest}): a composer's
 * field gaining focus, the pointer entering a microphone, a microphone taking
 * focus or a click, and the Settings section or the Dictionary page mounting.
 * After the first load it refreshes when the window regains focus and after an
 * install, select or delete, and polls every 1.5 s only while a model installs.
 *
 * Until it has loaded, a microphone shows the `idle` look; a click starts
 * capture at once and waits for this one request before it opens a stream
 * (`dictationSession.ts`). The read has a 5 s deadline: a daemon that takes the
 * connection and never answers must not hold a press, or every later read, on
 * a request that will not settle. After the deadline the store reads as
 * `failed`, the phase as `idle`, and the stream's own create answers instead.
 *
 * The interface learns managed policy only from `status.policy`, never from
 * `GET /config`; the daemon checks policy on every request regardless.
 */
import { useSyncExternalStore } from 'react';
import { getApiUrl } from '../../config';
import { BUNDLED_BIOROUTER_VERSION } from '../../utils/biorouterVersion';
import { isBrowserSurface } from '../../utils/surface';
import type { MicAccessStatus } from '../../utils/micPermissionSettings';
import { captureSupported, prewarmCapture } from './capture';
import { dictationMessage, type DictationMessageId } from './copy';
import { dictationApi, type DictationApi, type DictationStatus } from './dictationApi';
import { ROSETTA_BEFORE_MACOS15_KEY } from './rosetta';

/** `absent`: the daemon answered 404 or 405, a backend from before dictation (S17). */
export type StatusLoad = 'unloaded' | 'loading' | 'loaded' | 'absent' | 'failed';

export interface DictationStatusSnapshot {
  load: StatusLoad;
  status: DictationStatus | null;
  /** The operating system's microphone access for this app; `unknown` off the desktop. */
  micStatus: MicAccessStatus;
}

export const INSTALL_POLL_MS = 1_500;
/** How long one status read may take before it counts as failed. */
export const STATUS_TIMEOUT_MS = 5_000;

const INITIAL: DictationStatusSnapshot = { load: 'unloaded', status: null, micStatus: 'unknown' };

let snapshot: DictationStatusSnapshot = INITIAL;
const listeners = new Set<() => void>();
let inflight: Promise<DictationStatus | null> | null = null;
let pollTimer: ReturnType<typeof setTimeout> | null = null;
let focusRefreshInstalled = false;
let api: DictationApi = dictationApi;

function publish(next: Partial<DictationStatusSnapshot>): void {
  snapshot = { ...snapshot, ...next };
  for (const listener of [...listeners]) listener();
}

export function getDictationStatusSnapshot(): DictationStatusSnapshot {
  return snapshot;
}

export function subscribeDictationStatus(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** The snapshot, for a React component. */
export function useDictationStatus(): DictationStatusSnapshot {
  return useSyncExternalStore(subscribeDictationStatus, getDictationStatusSnapshot);
}

const installRunning = (status: DictationStatus | null): boolean =>
  !!status?.install_job &&
  (status.install_job.phase === 'downloading' || status.install_job.phase === 'verifying');

async function readMicStatus(): Promise<MicAccessStatus> {
  if (isBrowserSurface()) return 'unknown';
  try {
    return (await window.electron?.dictationMicStatus?.()) ?? 'unknown';
  } catch {
    return 'unknown';
  }
}

function schedulePoll(status: DictationStatus | null): void {
  if (pollTimer) {
    clearTimeout(pollTimer);
    pollTimer = null;
  }
  if (!installRunning(status)) return;
  pollTimer = setTimeout(() => {
    pollTimer = null;
    void refreshDictationStatus();
  }, INSTALL_POLL_MS);
}

function installFocusRefresh(): void {
  if (focusRefreshInstalled || typeof window === 'undefined') return;
  focusRefreshInstalled = true;
  window.addEventListener('focus', onWindowFocus);
}

function onWindowFocus(): void {
  void refreshDictationStatus();
}

/**
 * Run `work` with a deadline: past `ms` its signal aborts and the answer
 * rejects, whether or not `work` honours the signal.
 */
function withDeadline<T>(work: (signal: AbortSignal) => Promise<T>, ms: number): Promise<T> {
  const controller = new AbortController();
  let timer: ReturnType<typeof setTimeout> | undefined;
  const expired = new Promise<never>((_, reject) => {
    timer = setTimeout(() => {
      controller.abort();
      reject(new Error('dictation status read timed out'));
    }, ms);
  });
  return Promise.race([work(controller.signal), expired]).finally(() => clearTimeout(timer));
}

/** Read the status now, whatever was loaded before. */
export function refreshDictationStatus(): Promise<DictationStatus | null> {
  if (inflight) return inflight;
  if (snapshot.load === 'unloaded' || snapshot.load === 'failed') publish({ load: 'loading' });
  inflight = (async () => {
    try {
      const [result, micStatus] = await withDeadline(
        (signal) => Promise.all([api.status(signal), readMicStatus()]),
        STATUS_TIMEOUT_MS
      );
      if (result.ok) {
        publish({ load: 'loaded', status: result.data, micStatus });
        rememberPolicyHint(result.data);
        installFocusRefresh();
        schedulePoll(result.data);
        return result.data;
      }
      if (result.status === 404 || result.status === 405) {
        publish({ load: 'absent', status: null, micStatus });
        installFocusRefresh();
        return null;
      }
      publish({ load: snapshot.status ? 'loaded' : 'failed', micStatus });
      return snapshot.status;
    } catch {
      publish({ load: snapshot.status ? 'loaded' : 'failed' });
      return snapshot.status;
    } finally {
      inflight = null;
    }
  })();
  return inflight;
}

/** Load once; later calls answer what is loaded. A failed read is tried again. */
export function ensureDictationStatus(): Promise<DictationStatus | null> {
  if (snapshot.load === 'loaded' || snapshot.load === 'absent') {
    return Promise.resolve(snapshot.status);
  }
  return refreshDictationStatus();
}

/**
 * The first sign that dictation may be used: the first focus of a composer's
 * field, the pointer entering or focus or a click on a microphone, the Settings
 * section or the Dictionary page mounting. Cheap to call on every such event.
 *
 * Besides loading the status once, it prepares the press (`capture.ts`
 * `prewarmCapture`): it warms the browser's media device list, at most every
 * 30 s, which takes about 200 ms off the first press in a window, and makes the
 * suspended 16 kHz capture context the press resumes. No microphone is opened
 * and nothing is asked.
 */
export function noteDictationInterest(): void {
  prewarmCapture();
  if (snapshot.load === 'unloaded' || snapshot.load === 'failed') void ensureDictationStatus();
}

/** After an install, select or delete: read the status again. */
export function noteDictationModelsChanged(): void {
  void refreshDictationStatus();
}

// ── per viewer conveniences ──────────────────────────────────────────────────

export const DICTATION_VIEWER_KEY = 'biorouter:dictation-viewer';

/**
 * Per viewer settings in `localStorage`, never state that must persist: the
 * device, noise reduction, whether the microphone shows, the hosts whose
 * disclosure was shown or whose plain `http:` address was confirmed, and a hint
 * to hide the button at the next launch when policy forbade dictation. Every
 * read and write is wrapped, so a blocked storage behaves as the defaults.
 */
export interface DictationViewerSettings {
  deviceId?: string;
  deviceLabel?: string;
  noiseReduction: boolean;
  showButton: boolean;
  ackHosts: string[];
  policyOffHint: boolean;
}

const VIEWER_DEFAULTS: DictationViewerSettings = {
  noiseReduction: false,
  showButton: true,
  ackHosts: [],
  policyOffHint: false,
};

export function readDictationViewer(): DictationViewerSettings {
  try {
    const raw = window.localStorage.getItem(DICTATION_VIEWER_KEY);
    if (!raw) return { ...VIEWER_DEFAULTS };
    const parsed = JSON.parse(raw) as Partial<DictationViewerSettings>;
    return {
      ...(typeof parsed.deviceId === 'string' ? { deviceId: parsed.deviceId } : {}),
      ...(typeof parsed.deviceLabel === 'string' ? { deviceLabel: parsed.deviceLabel } : {}),
      noiseReduction: parsed.noiseReduction === true,
      showButton: parsed.showButton !== false,
      ackHosts: Array.isArray(parsed.ackHosts)
        ? parsed.ackHosts.filter((host): host is string => typeof host === 'string').slice(0, 64)
        : [],
      policyOffHint: parsed.policyOffHint === true,
    };
  } catch {
    return { ...VIEWER_DEFAULTS };
  }
}

export function writeDictationViewer(patch: Partial<DictationViewerSettings>): void {
  try {
    const next = { ...readDictationViewer(), ...patch };
    window.localStorage.setItem(DICTATION_VIEWER_KEY, JSON.stringify(next));
  } catch {
    // A private window or blocked storage: the setting lasts for this page only.
  }
}

function rememberPolicyHint(status: DictationStatus): void {
  const off = !status.policy.enabled.allowed || !status.policy.capture.allowed;
  if (readDictationViewer().policyOffHint !== off) writeDictationViewer({ policyOffHint: off });
}

// ── the microphone's phase before any recording ──────────────────────────────

export type ResolvedPhase = 'removed' | 'unavailable' | 'not-installed' | 'installing' | 'idle';

export interface PhaseDecision {
  phase: ResolvedPhase;
  /** The sentence for `unavailable`. */
  reason?: string;
  reasonId?: DictationMessageId;
  /** 0 to 100 while installing. */
  progress?: number;
}

export interface PhaseEnvironment {
  /** `window.isSecureContext` and `getUserMedia`; defaults to the real check. */
  secure?: boolean;
  surface?: 'desktop' | 'browser';
  /** The page's own address, for MSG-INSECURE; defaults to `location.host`. */
  host?: string;
  /**
   * The daemon's address, for MSG-REMOTE-OLD; defaults to `host` when that is
   * given, else to the address the renderer's API calls go to.
   */
  backendHost?: string;
  /** For MSG-REMOTE-OLD; defaults to the version this renderer was built as. */
  appVersion?: string;
  /**
   * The desktop app is the Intel build running under Rosetta on a macOS before
   * 15, where Rosetta cannot translate AVX2 (MSG-CPU-ROSETTA). The main process
   * knows it (`app.runningUnderARM64Translation` and the OS version); the
   * renderer passes it in once a door reads it.
   */
  rosettaBeforeMacOS15?: boolean;
}

function pageHost(): string {
  try {
    return typeof location === 'undefined' ? '' : location.host;
  } catch {
    return '';
  }
}

/**
 * What `main.ts` told this window about Rosetta (`rosetta.ts`): the Intel build under
 * translation on a macOS before 15. Read from the window's start up configuration, so it costs
 * no request; false off the desktop and when the value is absent.
 */
function rosettaBeforeMacOS15FromMain(): boolean {
  if (isBrowserSurface()) return false;
  try {
    return window.appConfig?.get(ROSETTA_BEFORE_MACOS15_KEY) === true;
  } catch {
    return false;
  }
}

/** The host and port the renderer's API calls go to, or `''` when there is none. */
export function dictationBackendHost(): string {
  try {
    return new URL(getApiUrl('/')).host;
  } catch {
    return '';
  }
}

/** What MSG-REMOTE-OLD fills in: the daemon's address and this app's version. */
export function remoteOldValues(env: PhaseEnvironment = {}): Record<string, string> {
  const host = env.backendHost ?? env.host ?? dictationBackendHost();
  return {
    ...(host ? { host } : {}),
    "this app's version": env.appVersion ?? BUNDLED_BIOROUTER_VERSION,
  };
}

/**
 * Where speech is transcribed, as a person should be told (DD-92): by the
 * desktop's own daemon, by an external backend the desktop app is connected to
 * (S17), or by the daemon a browser reaches on `serve`. No surface says "this
 * computer" for the latter two, so the default, when this renderer cannot tell,
 * is the server. `host` prefers the name the daemon reports for itself.
 *
 * MSG-ENGINE-FAILED and MSG-SETUP fill `<where>` from it, and the setup popover
 * picks MSG-DISCLOSE-LOCAL, MSG-DISCLOSE-REMOTE or MSG-DISCLOSE-HOST by `kind`.
 */
export type DictationDaemonPlace =
  | { kind: 'this-computer' }
  | { kind: 'external-backend'; host: string }
  | { kind: 'serve'; host: string };

export function dictationDaemonPlace(
  status: DictationStatus | null = snapshot.status
): DictationDaemonPlace {
  if (isBrowserSurface()) {
    return { kind: 'serve', host: status?.host || pageHost() || dictationBackendHost() };
  }
  let local = false;
  try {
    // `main.ts` passes `BIOROUTER_LOCAL_BACKEND` to each window: true only for
    // the daemon the app itself started, false for an external backend.
    local = window.appConfig?.get('BIOROUTER_LOCAL_BACKEND') === true;
  } catch {
    local = false;
  }
  if (local) return { kind: 'this-computer' };
  return { kind: 'external-backend', host: status?.host || dictationBackendHost() };
}

/** Whether a verified speech model is ready to use. */
export function hasUsableModel(status: DictationStatus): boolean {
  if (!status.active_model) return false;
  return status.models.some((model) => model.id === status.active_model && model.installed);
}

/**
 * Which look a microphone takes before any recording, from the snapshot, in the
 * order of the phase table in `docs/dictation/surfaces.md`. Until the status
 * has loaded the answer is `idle`, so a first click still starts capture.
 *
 * Whatever `env` leaves out is read from the page, so a caller that passes no
 * environment still gets every placeholder filled.
 */
export function resolveDictationPhase(
  current: DictationStatusSnapshot,
  env: PhaseEnvironment = {}
): PhaseDecision {
  const secure = env.secure ?? captureSupported();
  const surface = env.surface ?? (isBrowserSurface() ? 'browser' : 'desktop');
  const unavailable = (reasonId: DictationMessageId, values: Record<string, string> = {}) => ({
    phase: 'unavailable' as const,
    reasonId,
    reason: dictationMessage(reasonId, values),
  });
  if (!secure) {
    const host = env.host ?? pageHost();
    return unavailable('MSG-INSECURE', host ? { host } : {});
  }
  if (current.load === 'absent') return unavailable('MSG-REMOTE-OLD', remoteOldValues(env));
  const status = current.status;
  if (!status) return { phase: 'idle' };
  if (!status.policy.enabled.allowed || !status.policy.capture.allowed) return { phase: 'removed' };
  if (!status.engine.compiled) return unavailable('MSG-NO-ENGINE');
  if (status.engine.state === 'unsupported_cpu') {
    const missing = status.engine.missing_cpu_features.join(', ');
    if (surface === 'browser') return unavailable('MSG-CPU-SERVER', { missing });
    if (env.rosettaBeforeMacOS15 ?? rosettaBeforeMacOS15FromMain())
      return unavailable('MSG-CPU-ROSETTA');
    return {
      phase: 'unavailable',
      reasonId: 'MSG-CPU',
      reason: `${dictationMessage('MSG-CPU', { missing })} ${dictationMessage('MSG-CPU-DESKTOP')}`,
    };
  }
  if (!hasUsableModel(status)) {
    if (installRunning(status)) {
      const job = status.install_job!;
      const progress =
        job.bytes_total > 0
          ? Math.min(100, Math.floor((job.bytes_done / job.bytes_total) * 100))
          : 0;
      return { phase: 'installing', progress };
    }
    if (status.install.allowed) return { phase: 'not-installed' };
    return { phase: 'unavailable', reason: status.install.reason ?? '' };
  }
  return { phase: 'idle' };
}

// ── tests ───────────────────────────────────────────────────────────────────

/** For tests: start over, optionally with a fake API. */
export function resetDictationStoreForTests(fake?: DictationApi): void {
  snapshot = INITIAL;
  listeners.clear();
  inflight = null;
  if (pollTimer) clearTimeout(pollTimer);
  pollTimer = null;
  if (focusRefreshInstalled && typeof window !== 'undefined') {
    window.removeEventListener('focus', onWindowFocus);
  }
  focusRefreshInstalled = false;
  api = fake ?? dictationApi;
}
