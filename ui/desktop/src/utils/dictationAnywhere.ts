import type { StreamRequest, Transcript } from '../components/dictation/dictationApi';

export const DEFAULT_ANYWHERE_SHORTCUT = 'CommandOrControl+Alt+Shift+D';
export const REVIEW_TEXT_MS = 5 * 60 * 1000;

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
 * item and the global shortcut. **Dictate into this app** in the preview starts whether
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

export function externalStreamRequest(
  sampleRate: number,
  source: AnywhereSource,
  privateConfirmed: boolean
): StreamRequest {
  return {
    target_kind: 'external',
    sample_rate: sampleRate,
    format: 's16le',
    ...(source !== 'shortcut' && privateConfirmed ? { include_private_terms: true } : {}),
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
