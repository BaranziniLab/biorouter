import type { StreamRequest, Transcript } from '../components/dictation/dictationApi';
import { dictationMessage, insertRefusalMessage } from '../components/dictation/copy';
import type { InsertRefusal, InsertResult } from './textInsertHelper';

export const DEFAULT_ANYWHERE_SHORTCUT = 'CommandOrControl+Alt+Shift+D';
export const REVIEW_TEXT_MS = 5 * 60 * 1000;
/**
 * How long a failed clipboard dictation keeps its recording for **Retry**: the composer's
 * `KEEP_FAILED_MS` (`components/dictation/dictationSession.ts`), which this module cannot import
 * because Electron main imports it. `dictationAnywhere.test.ts` holds the two equal.
 */
export const HELD_FOR_RETRY_MS = 5 * 60 * 1000;

export type AnywhereSource = 'tray' | 'shortcut' | 'preview';
export type AnywherePhase = 'off' | 'idle' | 'arming' | 'recording' | 'transcribing' | 'review';
export type AnywhereReason =
  | 'policy'
  | 'no-model'
  | 'no-tray'
  | 'shortcut-taken'
  | 'capture-failed'
  | 'engine-failed';
export type AnywhereResult =
  | { kind: 'copied'; words: number }
  | { kind: 'nothing' }
  | { kind: 'echo' }
  | { kind: 'old-backend' }
  | { kind: 'review'; text: string }
  | { kind: 'failed'; reason: AnywhereReason };

export interface AnywhereSettings {
  enabled: boolean;
  shortcut: string | null;
  disclosureShown: boolean;
}

export const DEFAULT_ANYWHERE_SETTINGS: AnywhereSettings = {
  enabled: false,
  shortcut: null,
  disclosureShown: false,
};

export interface AnywhereState {
  phase: AnywherePhase;
  /**
   * Whether **Dictate into other apps** is on. The switch governs the tray item and the
   * global shortcut only, so a preview dictation can run while it is off; at rest the
   * phase is then `off` rather than `idle`.
   */
  switchOn: boolean;
  source: AnywhereSource | null;
  includePrivateTerms: boolean;
  reason: AnywhereReason | null;
  reviewText: string | null;
  reviewExpiresAt: number | null;
}

export const INITIAL_ANYWHERE_STATE: AnywhereState = {
  phase: 'off',
  switchOn: false,
  source: null,
  includePrivateTerms: false,
  reason: null,
  reviewText: null,
  reviewExpiresAt: null,
};

/**
 * Whether the **Dictate into other apps** switch governs a source: it governs the tray
 * item and the global shortcut. **Dictate into this application** in the preview starts whether
 * or not the switch is on (owner decision, 2026-11-10).
 */
export function switchGovernsSource(source: AnywhereSource | null): boolean {
  return source === 'tray' || source === 'shortcut';
}

/** Nothing is arming, recording, transcribing or in review: `idle` with the switch on, `off` with it off. */
export function anywhereAtRest(phase: AnywherePhase): boolean {
  return phase === 'idle' || phase === 'off';
}

/**
 * Whether a new recording from `source` may start now. A tray or shortcut start needs
 * the switch on and the machine idle; a preview start needs only that nothing else is
 * recording.
 */
export function canStartAnywhere(
  phase: AnywherePhase,
  source: AnywhereSource,
  switchOn: boolean
): boolean {
  if (switchGovernsSource(source)) return switchOn && phase === 'idle';
  return anywhereAtRest(phase);
}

/**
 * Whether a failed dictation keeps its recording for **Retry** (owner decision, 2026-11-13): a
 * tray or shortcut dictation, whose text goes to the clipboard, after a failure the stream can
 * send again (`DictationStreamError.retryable`, as the composer keeps one). A preview dictation
 * is not kept, because the field it was for may hold other text by then. A refusal before
 * capture and a microphone that never started have no recording to keep.
 */
export function keepsFailedRecording(source: AnywhereSource | null, retryable: boolean): boolean {
  return switchGovernsSource(source) && retryable;
}

function atRest(state: AnywhereState, reason: AnywhereReason | null = null): AnywhereState {
  return {
    ...INITIAL_ANYWHERE_STATE,
    phase: state.switchOn ? 'idle' : 'off',
    switchOn: state.switchOn,
    reason,
  };
}

export type AnywhereEvent =
  | { type: 'enable' }
  | { type: 'disable' }
  | { type: 'start'; source: AnywhereSource; privateConfirmed: boolean }
  | { type: 'capture-ready' }
  | { type: 'stop' }
  /** **Retry** on the review card: the held recording is transcribed again. */
  | { type: 'retry'; source: AnywhereSource }
  | { type: 'cancel' }
  | { type: 'refuse'; reason: AnywhereReason }
  | { type: 'review'; text: string; now: number }
  | { type: 'finish' }
  | { type: 'expire'; now: number };

export function nextAnywhereState(state: AnywhereState, event: AnywhereEvent): AnywhereState {
  switch (event.type) {
    case 'enable':
      return state.phase === 'off'
        ? { ...INITIAL_ANYWHERE_STATE, phase: 'idle', switchOn: true }
        : { ...state, switchOn: true };
    case 'disable':
      // Off ends a tray or shortcut dictation at once. A preview dictation goes on and
      // comes to rest as `off`.
      return !anywhereAtRest(state.phase) && !switchGovernsSource(state.source)
        ? { ...state, switchOn: false }
        : INITIAL_ANYWHERE_STATE;
    case 'start':
      if (!canStartAnywhere(state.phase, event.source, state.switchOn)) return state;
      return {
        ...INITIAL_ANYWHERE_STATE,
        phase: 'arming',
        switchOn: state.switchOn,
        source: event.source,
        includePrivateTerms: event.source !== 'shortcut' && event.privateConfirmed,
      };
    case 'capture-ready':
      return state.phase === 'arming' ? { ...state, phase: 'recording' } : state;
    case 'stop':
      return state.phase === 'arming' || state.phase === 'recording'
        ? { ...state, phase: 'transcribing', includePrivateTerms: false }
        : state;
    case 'retry':
      // A held recording is transcribed again exactly when a new start from its source could
      // begin: nothing else is in progress, and the switch is on for a tray or shortcut one.
      if (!canStartAnywhere(state.phase, event.source, state.switchOn)) return state;
      return {
        ...INITIAL_ANYWHERE_STATE,
        phase: 'transcribing',
        switchOn: state.switchOn,
        source: event.source,
      };
    case 'cancel':
      return atRest(state);
    case 'refuse':
      return atRest(state, event.reason);
    case 'review':
      return state.phase === 'transcribing'
        ? {
            ...state,
            phase: 'review',
            includePrivateTerms: false,
            reviewText: event.text,
            reviewExpiresAt: event.now + REVIEW_TEXT_MS,
          }
        : state;
    case 'finish':
      return atRest(state);
    case 'expire':
      return state.reviewExpiresAt !== null && event.now >= state.reviewExpiresAt
        ? atRest(state)
        : state;
  }
}

/**
 * Where a Dictate to clipboard dictation's text goes, as the dictation history names it
 * (`DictationHistorySurface`): typed into the app that was in front (`other_app`), copied
 * (`clipboard`), or inserted into an app preview or in-app browser page (`app_preview`).
 */
export type AnywhereSurface = 'other_app' | 'clipboard' | 'app_preview';

const ANYWHERE_SURFACES: readonly AnywhereSurface[] = ['other_app', 'clipboard', 'app_preview'];

/** A surface word from main, or null for anything else. */
export function parseAnywhereSurface(raw: unknown): AnywhereSurface | null {
  return ANYWHERE_SURFACES.includes(raw as AnywhereSurface) ? (raw as AnywhereSurface) : null;
}

/**
 * The surface a dictation from `source` records: a preview's is `app_preview`; a tray or
 * shortcut one is `other_app` when main kept an app to type into, else `clipboard`.
 */
export function anywhereSurface(source: AnywhereSource, typesIntoApp: boolean): AnywhereSurface {
  if (source === 'preview') return 'app_preview';
  return typesIntoApp ? 'other_app' : 'clipboard';
}

/** The stream request, with the surface the history records (an older daemon ignores it). */
export type ExternalStreamRequest = StreamRequest & { surface: AnywhereSurface };

export function externalStreamRequest(
  sampleRate: number,
  source: AnywhereSource,
  privateConfirmed: boolean,
  surface: AnywhereSurface | null = null
): ExternalStreamRequest {
  return {
    target_kind: 'external',
    sample_rate: sampleRate,
    format: 's16le',
    ...(source !== 'shortcut' && privateConfirmed ? { include_private_terms: true } : {}),
    // A preview's surface is fixed, and only the shortcut ever types into another app; anything
    // else is a copy.
    surface: anywhereSurface(source, source === 'shortcut' && surface === 'other_app'),
  };
}

export function wordCount(text: string): number {
  return text.trim().split(/\s+/u).filter(Boolean).length;
}

export function clipboardOutcome(
  transcript: Transcript,
  write: (text: string) => void,
  read: () => string
): AnywhereResult {
  if (transcript.no_speech || !transcript.text.trim()) return { kind: 'nothing' };
  if (transcript.dictionary_only === undefined) return { kind: 'old-backend' };
  if (transcript.dictionary_only) return { kind: 'echo' };
  write(transcript.text);
  if (read() !== transcript.text) return { kind: 'review', text: transcript.text };
  return { kind: 'copied', words: wordCount(transcript.text) };
}

export function parseAnywhereSettings(raw: unknown): AnywhereSettings {
  if (!raw || typeof raw !== 'object') return DEFAULT_ANYWHERE_SETTINGS;
  const row = raw as Record<string, unknown>;
  return {
    enabled: row.enabled === true,
    shortcut: typeof row.shortcut === 'string' && row.shortcut.length <= 80 ? row.shortcut : null,
    disclosureShown: row.disclosureShown === true,
  };
}

export function reviewTextBeforeExpiry(
  text: string | null,
  expiresAt: number | null,
  now: number
): string | null {
  return text !== null && expiresAt !== null && now < expiresAt ? text : null;
}

/**
 * The switch's part in a start that is still arming. A tray or shortcut start goes on only
 * while the switch stays on and no settings change arrived since it began; a preview start
 * does not depend on the switch.
 */
export function switchKeepsStart(
  source: AnywhereSource,
  settingsRevision: number,
  currentSettingsRevision: number,
  switchOn: boolean
): boolean {
  return !switchGovernsSource(source) || (settingsRevision === currentSettingsRevision && switchOn);
}

export function sameAnywhereStart(
  source: AnywhereSource,
  startGeneration: number,
  currentGeneration: number,
  phase: AnywherePhase,
  pendingGeneration: number | null,
  settingsRevision: number,
  currentSettingsRevision: number,
  switchOn: boolean
): boolean {
  return (
    startGeneration === currentGeneration &&
    phase === 'arming' &&
    pendingGeneration === startGeneration &&
    switchKeepsStart(source, settingsRevision, currentSettingsRevision, switchOn)
  );
}

export function sameCaptureOperation(token: number, current: number, cancelled: boolean): boolean {
  return token === current && !cancelled;
}

export function currentAnywhereSettingsRequest(request: number, latest: number): boolean {
  return request === latest;
}

export function usableAnywhereTrigger(
  tray: boolean,
  shortcut: string | null,
  gnomeWayland: boolean,
  registered: boolean
): boolean {
  return tray || (Boolean(shortcut) && !gnomeWayland && registered);
}

export function elapsedFromPress(pressedAtEpochMs: number, firstFrameEpochMs: number): number {
  return Math.max(0, firstFrameEpochMs - pressedAtEpochMs);
}

export function abortFinishingStream(stream: { cancel: () => Promise<void> } | null): void {
  if (stream) void stream.cancel();
}

export function deliverSettingsWhenReady(
  request: number,
  latest: number,
  ready: boolean,
  sameWindow: boolean
): boolean {
  return request === latest && ready && sameWindow;
}

export function afterAnywhereSettingsWriteFailure(
  previous: AnywhereSettings,
  previousOffRequested: boolean,
  current: AnywhereSettings,
  currentOffRequested: boolean,
  requestRevision: number,
  latestRevision: number,
  requestedEnabled: boolean
): { settings: AnywhereSettings; offRequested: boolean } {
  if (requestRevision !== latestRevision) {
    return {
      settings: currentOffRequested ? { ...current, enabled: false } : current,
      offRequested: currentOffRequested,
    };
  }
  if (!requestedEnabled) return { settings: { ...current, enabled: false }, offRequested: true };
  return { settings: previous, offRequested: previousOffRequested };
}

// ── Typing the text into the app in front (DD-97, macOS) ──────────────────────
// Dictate to clipboard from the global shortcut, or from fn in another app, types its text into
// the app that was in front when it started, through the `biorouter-textinsert` helper
// (`utils/textInsertHelper.ts`), and copies it only when that app cannot take it. The tray item
// and a preview dictation never type into another app.

/** **Type into the app you are using**: `<config>/dictation/insert.json`, written only by main. */
export interface InsertSettings {
  enabled: boolean;
}

/** On by default on macOS, the one system Biorouter can type into another app on. */
export function defaultInsertSettings(platform: string): InsertSettings {
  return { enabled: platform === 'darwin' };
}

export function parseInsertSettings(raw: unknown, platform: string): InsertSettings {
  if (platform !== 'darwin') return { enabled: false };
  if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return defaultInsertSettings(platform);
  const row = raw as Record<string, unknown>;
  return { enabled: typeof row.enabled === 'boolean' ? row.enabled : true };
}

/**
 * Whether the insertion helper runs: on macOS, with the setting on and **Dictate into other apps**
 * on, while something can start a dictation that types into another app (fn in other apps, or a
 * registered global shortcut). Otherwise no helper runs at all.
 */
export function textInsertWanted(input: {
  platform: string;
  insertEnabled: boolean;
  anywhereOn: boolean;
  fnOtherApps: boolean;
  shortcutRegistered: boolean;
}): boolean {
  return (
    input.platform === 'darwin' &&
    input.insertEnabled &&
    input.anywhereOn &&
    (input.fnOtherApps || input.shortcutRegistered)
  );
}

/** The app a dictation types into: the one in front when it started. */
export interface InsertTarget {
  /** The Dictate to clipboard generation the target belongs to. */
  generation: number;
  pid: number;
  bundle: string | null;
}

/** Only `shortcut` (the global shortcut and fn in another app) ever types into an app. */
export function capturesInsertTarget(source: AnywhereSource, insertEnabled: boolean): boolean {
  return source === 'shortcut' && insertEnabled;
}

/**
 * The target a start keeps from the helper's `front` answer: the app in front, unless that is
 * Biorouter itself (whose own composers take fn's text) or the helper could not say.
 */
export function insertTargetFrom(
  front: { pid: number; bundle: string | null } | null,
  ownPid: number,
  generation: number
): InsertTarget | null {
  if (!front || !Number.isSafeInteger(front.pid) || front.pid <= 0 || front.pid === ownPid)
    return null;
  return {
    generation,
    pid: front.pid,
    bundle: typeof front.bundle === 'string' ? front.bundle : null,
  };
}

/**
 * The text a finished dictation may type into an app: real speech that is not only dictionary
 * words, from a daemon new enough to say so. Null for everything `clipboardOutcome` answers
 * without writing (nothing heard, an echo, an older daemon), which keep their own messages.
 */
export function insertableText(transcript: Transcript): string | null {
  if (transcript.no_speech || !transcript.text.trim()) return null;
  if (transcript.dictionary_only !== false) return null;
  return transcript.text;
}

/** One line of the unpackaged insert log: never the text, only what happened and its length. */
export interface InsertLogEntry {
  route: 'other_app' | 'clipboard' | 'app_preview';
  method: 'paste' | 'copy' | 'embedded';
  /** The target app's bundle id, or null when there was none or the helper did not say. */
  bundle: string | null;
  result: 'inserted' | 'copied' | 'review' | 'refused' | InsertRefusal;
  /** Why a paste was refused, beside the copy that replaced it. */
  reason?: InsertRefusal;
  chars: number;
}

const BUNDLE_ID = /^[A-Za-z0-9.-]{1,255}$/;

/** {@link InsertLogEntry} as one JSON line. A bundle id of any other shape is left out. */
export function insertLogLine(entry: InsertLogEntry, at: Date = new Date()): string {
  const line: Record<string, unknown> = {
    at: at.toISOString(),
    route: entry.route,
    method: entry.method,
    bundle: entry.bundle && BUNDLE_ID.test(entry.bundle) ? entry.bundle : null,
    result: entry.result,
    chars: Number.isSafeInteger(entry.chars) && entry.chars >= 0 ? entry.chars : 0,
  };
  if (entry.reason) line.reason = entry.reason;
  return JSON.stringify(line);
}

/** What main lends {@link deliverAnywhereTranscript}, so a test can hand it fakes. */
export interface AnywhereDelivery {
  /** The helper's `insert` (`TextInsertService.insert`). */
  insert: (pid: number, text: string) => Promise<InsertResult>;
  /** The clipboard, or the unpackaged test file standing in for it. */
  write: (text: string) => void;
  read: () => string;
  /** A system notification. */
  notify: (body: string) => void;
  /** The focused review card, with its sentence and the text to copy from it. */
  review: (message: string, text: string | null) => void;
  /** The paste shortcut the sentences name (⌘V on macOS). */
  pasteKey: string;
  /** The review card's sentence for a daemon too old to say whether the text is dictionary words only. */
  oldBackendMessage: string;
  log?: (entry: InsertLogEntry) => void;
}

/** What {@link deliverAnywhereTranscript} did, for its tests. */
export type AnywhereDelivered =
  | { kind: 'inserted' }
  | { kind: 'copied'; reason?: InsertRefusal }
  | { kind: 'review'; reason?: InsertRefusal }
  | { kind: 'nothing' | 'echo' | 'old-backend' };

const REVIEW_COPY_FAILED = 'The clipboard did not keep the text. Copy it from this card.';

/**
 * A finished Dictate to clipboard dictation's text, delivered:
 *
 * - with a target and real speech, typed into the target app and nothing else: the clipboard is
 *   never written and no card or notification is shown (the indicator fades);
 * - when the helper refuses (no permission, a password field, another app in front, a failure),
 *   copied, with a notification that names the reason;
 * - without a target, copied as before, with "Copied N words";
 * - nothing heard, an echo of dictionary words, or an older daemon: their own message, and
 *   neither the clipboard nor any app is touched.
 *
 * The review card opens only when the clipboard did not keep a copy (or for an older daemon), so
 * it never takes focus from an app the text is still going to.
 */
export async function deliverAnywhereTranscript(
  transcript: Transcript,
  target: InsertTarget | null,
  delivery: AnywhereDelivery
): Promise<AnywhereDelivered> {
  const text = insertableText(transcript);
  let refused: InsertRefusal | undefined;
  if (target && text !== null) {
    let result: InsertResult;
    try {
      result = await delivery.insert(target.pid, text);
    } catch {
      result = { ok: false, reason: 'failed' };
    }
    if (result.ok) {
      delivery.log?.({
        route: 'other_app',
        method: 'paste',
        bundle: target.bundle,
        result: 'inserted',
        chars: text.length,
      });
      return { kind: 'inserted' };
    }
    refused = result.reason;
    delivery.log?.({
      route: 'other_app',
      method: 'paste',
      bundle: target.bundle,
      result: refused,
      chars: text.length,
    });
  }
  const outcome = clipboardOutcome(transcript, delivery.write, delivery.read);
  const logCopy = (result: 'copied' | 'review') =>
    delivery.log?.({
      route: target ? 'other_app' : 'clipboard',
      method: 'copy',
      bundle: target?.bundle ?? null,
      result,
      ...(refused ? { reason: refused } : {}),
      chars: transcript.text.length,
    });
  switch (outcome.kind) {
    case 'copied':
      logCopy('copied');
      delivery.notify(
        refused
          ? insertRefusalMessage(refused, delivery.pasteKey)
          : dictationMessage('MSG-CLIPBOARD-COPIED', {
              n: String(outcome.words),
              key: delivery.pasteKey,
            })
      );
      return refused ? { kind: 'copied', reason: refused } : { kind: 'copied' };
    case 'review':
      logCopy('review');
      delivery.review(REVIEW_COPY_FAILED, outcome.text);
      return refused ? { kind: 'review', reason: refused } : { kind: 'review' };
    case 'nothing':
      delivery.notify(dictationMessage('MSG-CLIPBOARD-NOTHING'));
      return { kind: 'nothing' };
    case 'echo':
      delivery.notify(dictationMessage('MSG-CLIPBOARD-ECHO'));
      return { kind: 'echo' };
    case 'old-backend':
      delivery.review(delivery.oldBackendMessage, null);
      return { kind: 'old-backend' };
    case 'failed':
      // `clipboardOutcome` never answers `failed`; the type allows it.
      return { kind: 'nothing' };
  }
}

/**
 * Whether the hidden capture window stays open between dictations instead of closing after its
 * idle wait: while fn may dictate into other apps (macOS, the fn shortcut and its **Use fn in
 * other apps** row, and **Dictate into other apps**), so a short first hold does not wait for the
 * whole page to load and lose its words.
 */
export function captureWindowStaysWarm(input: {
  platform: string;
  fnEnabled: boolean;
  fnOtherApps: boolean;
  anywhereOn: boolean;
}): boolean {
  return input.platform === 'darwin' && input.fnEnabled && input.fnOtherApps && input.anywhereOn;
}

// ── Test seams (DESIGN 3.12) ─────────────────────────────────────────────────

/** The unpackaged-only variables main reads through {@link dictationTestSeams}, and nowhere else. */
export const DICTATION_TEST_SEAM_ENV = {
  fnTrace: 'BIOROUTER_FN_TRACE',
  clipboardFile: 'BIOROUTER_TEST_CLIPBOARD_FILE',
  insertLog: 'BIOROUTER_TEST_INSERT_LOG',
} as const;

export interface DictationTestSeams {
  /** Log each fn helper line and intent (`formatFnTrace`): never a key code or any text. */
  fnTrace: boolean;
  /** A file that stands in for the system clipboard in the copy paths. */
  clipboardFile: string | null;
  /** A file that takes one {@link insertLogLine} per insertion attempt. */
  insertLog: string | null;
}

const NO_SEAMS: DictationTestSeams = { fnTrace: false, clipboardFile: null, insertLog: null };

/** An absolute path, on either system's spelling, with no NUL; anything else is not believed. */
function seamPath(raw: string | undefined): string | null {
  const value = raw?.trim();
  if (!value || value.includes('\0') || value.length > 4096) return null;
  return /^(?:\/|[A-Za-z]:[\\/])/.test(value) ? value : null;
}

/**
 * The dictation test seams. A packaged app reads none of them, whatever its environment holds,
 * so a shipped Biorouter never logs its fn gestures, never writes a dictation to a file in place
 * of the clipboard and never keeps an insert log.
 */
export function dictationTestSeams(
  app: { isPackaged: boolean },
  env: Readonly<Record<string, string | undefined>>
): DictationTestSeams {
  if (app.isPackaged) return NO_SEAMS;
  return {
    fnTrace: env[DICTATION_TEST_SEAM_ENV.fnTrace]?.trim() === '1',
    clipboardFile: seamPath(env[DICTATION_TEST_SEAM_ENV.clipboardFile]),
    insertLog: seamPath(env[DICTATION_TEST_SEAM_ENV.insertLog]),
  };
}
